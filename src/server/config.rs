use rocket::response::Responder;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, fs, io, path::PathBuf};

#[derive(Deserialize, Debug)]
pub struct Config {
    pub projects: Vec<Project>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
}

#[derive(Debug)]
pub enum Error {
    ReadError(io::Error),
    ParseError(toml::de::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ReadError(e) => write!(f, "Error reading file\n{}", e),
            Error::ParseError(e) => write!(f, "Error parsing contents\n{}", e),
        }
    }
}

impl Responder<'_, 'static> for Error {
    fn respond_to(self, _: &rocket::Request<'_>) -> rocket::response::Result<'static> {
        Err(rocket::http::Status::InternalServerError)
    }
}

// Loads the config for the given directory
pub fn load(path: &PathBuf) -> Result<Config, Error> {
    let config_str = match fs::read_to_string(path) {
        Ok(o) => o,
        Err(e) => return Err(Error::ReadError(e)),
    };

    let config = match toml::from_str(&config_str) {
        Ok(o) => o,
        Err(e) => return Err(Error::ParseError(e)),
    };

    Ok(config)
}
