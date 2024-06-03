use rocket::http::ContentType;
use rust_embed::RustEmbed;

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::PathBuf;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct FrontendFS;

#[get("/<file..>")]
pub fn serve_spa(file: PathBuf) -> (ContentType, Cow<'static, [u8]>) {
    tracing::trace!(file = ?file, "Serving file");

    let file_path = file.display().to_string();
    let asset = FrontendFS::get(&file_path);

    match asset {
        Some(asset) => {
            tracing::debug!(file = ?file, "File found, serving");

            // Get the content type from the file extension
            let content_type = file
                .extension()
                .and_then(OsStr::to_str)
                .and_then(ContentType::from_extension)
                .unwrap_or(ContentType::Bytes);

            (content_type, asset.data)
        }
        None => {
            // If the file doesn't exist, serve index.html
            tracing::debug!(file = ?file, "File not found, serving index.html");

            let asset = FrontendFS::get("index.html").unwrap();

            (ContentType::HTML, asset.data)
        }
    }
}
