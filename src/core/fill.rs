use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    fs,
    path::PathBuf,
    time::Duration,
};
use tera::{Context, Tera};

pub const TEMPLATE_EXT: &str = ".j2";

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

pub fn fill(
    project_dir: &PathBuf,
    data: HashMap<String, String>,
    out_dir: &PathBuf,
) -> Result<Vec<Result<RenderedFile, FileError>>, tera::Error> {
    let glob = project_dir.join("**").join("*".to_owned() + TEMPLATE_EXT);

    let tera = Tera::new(&glob.to_string_lossy())?;
    let context = Context::from_serialize(data)?;

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
        let mut template_name = template_name.to_string();
        if template_name.ends_with(TEMPLATE_EXT) {
            let mut tera = tera.clone();
            template_name = match tera.render_str(&template_name, &context) {
                Ok(s) => s,
                Err(e) => {
                    return Err(FileError {
                        kind: FileErrorKind::ErrorRenderingName,
                        file: template_name.to_string(),
                        source: Box::new(e),
                    });
                }
            };
        }

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
            path: template_name.into(),
            contents: output,
            elapsed: start_time.elapsed(),
        })
    });

    Ok(rendered_templates.collect::<Vec<_>>())
}
