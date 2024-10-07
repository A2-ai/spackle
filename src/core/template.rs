use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    fs, io,
    path::PathBuf,
    time::Duration,
};
use tera::{Context, Tera};

use super::slot::Slot;

pub const TEMPLATE_EXT: &str = ".j2";

#[derive(Debug)]
pub struct FileError {
    pub kind: FileErrorKind,
    pub file: String,
}

impl Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.kind)
    }
}

#[derive(Debug)]
pub enum FileErrorKind {
    ErrorRenderingContents(tera::Error),
    ErrorRenderingName(tera::Error),
    ErrorCreatingDest(io::ErrorKind),
    ErrorWritingToDest(io::Error),
}

impl Display for FileErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileErrorKind::ErrorRenderingContents(e) => {
                write!(f, "error rendering template contents: {}", e)
            }
            FileErrorKind::ErrorRenderingName(e) => {
                write!(f, "error rendering template name: {}", e)
            }
            FileErrorKind::ErrorCreatingDest(e) => write!(f, "error creating directory: {}", e),
            FileErrorKind::ErrorWritingToDest(e) => write!(f, "error writing file: {}", e),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderedFile {
    pub path: PathBuf,
    pub contents: String,
    pub elapsed: Duration,
}

pub fn fill(
    project_dir: &PathBuf,
    out_dir: &PathBuf,
    slot_data: &HashMap<String, String>,
) -> Result<Vec<Result<RenderedFile, FileError>>, tera::Error> {
    let glob = project_dir.join("**").join("*".to_owned() + TEMPLATE_EXT);

    let tera = Tera::new(&glob.to_string_lossy())?;
    let context = Context::from_serialize(slot_data)?;

    let template_names = tera.get_template_names().collect::<Vec<_>>();
    let rendered_templates = template_names.iter().map(|template_name| {
        let start_time = std::time::Instant::now();

        // Render the file contents
        let output = match tera.render(template_name, &context) {
            Ok(o) => o,
            Err(e) => {
                return Err(FileError {
                    kind: FileErrorKind::ErrorRenderingContents(e),
                    file: template_name.to_string(),
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
                        kind: FileErrorKind::ErrorRenderingName(e),
                        file: template_name.to_string(),
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
                e => {
                    return Err(FileError {
                        kind: FileErrorKind::ErrorCreatingDest(e),
                        file: template_name.to_string(),
                    })
                }
            },
        }

        fs::write(&output_dir, output.clone()).map_err(|e| FileError {
            kind: FileErrorKind::ErrorWritingToDest(e),
            file: template_name.to_string(),
        })?;

        Ok(RenderedFile {
            path: template_name.into(),
            contents: output,
            elapsed: start_time.elapsed(),
        })
    });

    Ok(rendered_templates.collect::<Vec<_>>())
}

pub enum ValidateError {
    TeraError(tera::Error),
    RenderError(Vec<(String, tera::Error)>),
}

// Validates the templates in the directory against the slots
// Returns an error if any of the templates reference a slot that doesn't exist
pub fn validate(dir: &PathBuf, slots: &Vec<Slot>) -> Result<(), ValidateError> {
    let glob = dir.join("**").join("*".to_owned() + TEMPLATE_EXT);

    let tera = Tera::new(&glob.to_string_lossy()).map_err(|e| ValidateError::TeraError(e))?;
    let mut context = Context::from_serialize(
        slots
            .iter()
            .map(|s| (s.key.clone(), ""))
            .collect::<HashMap<_, _>>(),
    )
    .map_err(|e| ValidateError::TeraError(e))?;
    context.insert("_project_name".to_string(), "");

    let errors = tera
        .get_template_names()
        .filter_map(|template_name| match tera.render(template_name, &context) {
            Ok(_) => None,
            Err(e) => Some((template_name.to_string(), e)),
        })
        .collect::<Vec<_>>();

    if !errors.is_empty() {
        return Err(ValidateError::RenderError(errors));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempdir::TempDir;

    use crate::core::slot::SlotType;

    use super::*;

    #[test]
    fn fill_proj1() {
        let dir = TempDir::new("spackle").unwrap().into_path();

        let result = fill(
            &PathBuf::from("tests/data/proj1"),
            &dir.join("proj1_filled"),
            &HashMap::from([
                ("person_name".to_string(), "Joe Bloggs".to_string()),
                ("person_age".to_string(), "42".to_string()),
                ("file_name".to_string(), "main".to_string()),
            ]),
        );

        println!("{:?}", result);

        assert!(result.is_ok());
    }

    #[test]
    fn validate_dir_proj1() {
        let result = validate(
            &PathBuf::from("tests/data/proj1"),
            &vec![Slot {
                key: "defined_field".to_string(),
                r#type: SlotType::String,
                name: None,
                description: None,
            }],
        );

        assert!(result.is_err());
    }

    #[test]
    fn validate_dir_proj2() {
        let result = validate(
            &PathBuf::from("tests/data/proj2"),
            &vec![Slot {
                key: "defined_field".to_string(),
                r#type: SlotType::String,
                name: None,
                description: None,
            }],
        );

        assert!(result.is_ok());
    }
}
