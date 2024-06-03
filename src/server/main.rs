use std::path::PathBuf;

use config::Project;
use rocket::serde::json::Json;

#[macro_use]
extern crate rocket;

mod config;

#[get("/")]
fn list_projects() -> Result<Json<Vec<Project>>, config::Error> {
    config::load(&PathBuf::from("tests/data/server/config1.toml"))
        .map(|config| Json(config.projects))
        .map_err(|e| {
            tracing::error!(error = e.to_string(), "Could not load config");
            e
        })
}

#[launch]
fn rocket() -> _ {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    rocket::build()
        .mount("/", routes![serve_frontend])
        .mount("/api/", routes![list_projects])
}
