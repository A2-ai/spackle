use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    fs,
    path::PathBuf,
    time::Duration,
};
use tera::{Context, Tera};
use walkdir::WalkDir;

const TEMPLATE_EXT: &str = ".j2";

#[derive(Debug)]
pub enum Error {
    ErrorInitializingTera(tera::Error),
    ErrorCopyingFromSource {
        source: Box<dyn std::error::Error>,
        path: PathBuf,
    },
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ErrorInitializingTera(e) => write!(f, "error initializing Tera: {}", e),
            Error::ErrorCopyingFromSource { source, path } => {
                write!(f, "error copying source {}\n{}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ErrorInitializingTera(e) => e.source(),
            Error::ErrorCopyingFromSource { source, .. } => source.source(),
        }
    }
}

#[derive(Debug)]
pub struct FileError {
    pub kind: FileErrorKind,
    pub file: String,
    pub source: Box<dyn std::error::Error>,
}

#[derive(Debug)]
pub enum FileErrorKind {
    ErrorRenderingContents,
    ErrorRenderingName,
    ErrorCreatingDest,
    ErrorWritingToDest,
}

impl Display for FileErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileErrorKind::ErrorRenderingContents => write!(f, "error rendering template contents"),
            FileErrorKind::ErrorRenderingName => write!(f, "error rendering template name"),
            FileErrorKind::ErrorCreatingDest => write!(f, "error creating directory"),
            FileErrorKind::ErrorWritingToDest => write!(f, "error writing file"),
        }
    }
}

impl Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} for {}", self.kind, self.file)
    }
}

impl std::error::Error for FileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

#[derive(Debug)]
pub struct RenderedFile {
    pub path: PathBuf,
    pub contents: String,
    pub elapsed: Duration,
}

// Fills the templates in the project with the provided slot data.
// Returns a list of results, where each is the result of rendering a given template
pub fn fill(
    project_dir: &PathBuf,
    data: HashMap<String, String>,
    out_dir: &PathBuf,
) -> Result<Vec<Result<RenderedFile, FileError>>, Error> {
    let glob = project_dir.join("**").join("*".to_owned() + TEMPLATE_EXT);

    let tera = Tera::new(&glob.to_string_lossy()).map_err(|e| Error::ErrorInitializingTera(e))?;
    let context = Context::from_serialize(data).map_err(|e| Error::ErrorInitializingTera(e))?;

    // Copy all files not matching the template extension or the out dir
    for entry in WalkDir::new(project_dir) {
        let entry = entry.map_err(|e| Error::ErrorCopyingFromSource {
            path: e.path().map(|p| p.to_path_buf()).unwrap_or_default(),
            source: Box::new(e),
        })?;
        let src_path = entry.path();
        let relative_path =
            src_path
                .strip_prefix(project_dir)
                .map_err(|e| Error::ErrorCopyingFromSource {
                    source: e.into(),
                    path: src_path.to_path_buf(),
                })?;
        let dst_path = out_dir.join(relative_path);

        println!("{} -> {}", src_path.display(), dst_path.display());

        if entry.file_name() == out_dir {
            continue;
        }

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst_path).map_err(|e| Error::ErrorCopyingFromSource {
                source: e.into(),
                path: dst_path.clone(),
            })?;
        } else if entry.file_type().is_file() {
            // Skip entry if it has template extension
            if dst_path.to_string_lossy().ends_with(TEMPLATE_EXT) {
                continue;
            }

            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent).map_err(|e| Error::ErrorCopyingFromSource {
                    source: e.into(),
                    path: parent.to_path_buf(),
                })?;
            }
            fs::copy(&src_path, &dst_path).map_err(|e| Error::ErrorCopyingFromSource {
                source: e.into(),
                path: dst_path.clone(),
            })?;
        }
    }

    let template_names = tera.get_template_names().collect::<Vec<_>>();
    let rendered_templates = template_names.iter().map(|template_name| {
        let start_time = std::time::Instant::now();

        // Render the file contents
        let output = match tera.render(template_name, &context) {
            Ok(o) => o,
            Err(e) => {
                return Err(FileError {
                    kind: FileErrorKind::ErrorRenderingContents,
                    file: template_name.to_string(),
                    source: Box::new(e),
                });
            }
        };

        // Render the file name
        let mut tera = tera.clone();
        let template_name = match tera.render_str(template_name, &context) {
            Ok(o) => o,
            Err(e) => {
                return Err(FileError {
                    kind: FileErrorKind::ErrorRenderingName,
                    file: template_name.to_string(),
                    source: Box::new(e),
                });
            }
        };

        let template_name = match template_name.strip_suffix(TEMPLATE_EXT) {
            Some(name) => name,
            None => template_name.as_str(),
        };

        // Write the output
        let output_dir = out_dir.join(template_name);

        match fs::create_dir_all(output_dir.parent().unwrap()) {
            Ok(_) => (),
            Err(e) => match e.kind() {
                std::io::ErrorKind::AlreadyExists => (),
                _ => {
                    return Err(FileError {
                        kind: FileErrorKind::ErrorCreatingDest,
                        file: template_name.to_string(),
                        source: Box::new(e),
                    })
                }
            },
        }

        fs::write(&output_dir, output.clone()).map_err(|e| FileError {
            kind: FileErrorKind::ErrorWritingToDest,
            file: template_name.to_string(),
            source: Box::new(e),
        })?;

        Ok(RenderedFile {
            path: output_dir,
            contents: output,
            elapsed: start_time.elapsed(),
        })
    });

    Ok(rendered_templates.collect::<Vec<_>>())
}
