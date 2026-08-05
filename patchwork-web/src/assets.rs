use std::path::PathBuf;

use actix_web::{
    HttpRequest, HttpResponse, Result,
    http::{Method, header},
    web,
};

#[cfg(debug_assertions)]
use actix_files::NamedFile;
#[cfg(not(debug_assertions))]
use rust_embed::RustEmbed;
#[cfg(not(debug_assertions))]
use std::borrow::Cow;
#[cfg(debug_assertions)]
use std::io;

const INDEX_FILE: &str = "index.html";
const INDEX_BASE_HREF_PLACEHOLDER: &str = "__PATCHWORK_BASE_HREF__";

#[derive(Clone)]
pub(crate) struct FrontendAssets {
    #[cfg(debug_assertions)]
    root: PathBuf,
    base_path: String,
    base_href: String,
}

#[cfg(not(debug_assertions))]
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/dist/"]
struct EmbeddedDist;

#[cfg(not(debug_assertions))]
const EMBEDDED_INDEX: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/index.html"));

impl FrontendAssets {
    #[cfg(debug_assertions)]
    pub(crate) fn new(root: PathBuf, base_path: String) -> Self {
        let base_href = crate::config::base_href(&base_path);
        Self {
            root,
            base_path,
            base_href,
        }
    }

    #[cfg(not(debug_assertions))]
    pub(crate) fn new(_root: PathBuf, base_path: String) -> Self {
        let base_href = crate::config::base_href(&base_path);
        Self {
            base_path,
            base_href,
        }
    }

    #[cfg(debug_assertions)]
    async fn response(&self, path: &str, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        let dist_path = self.root.join(path);
        let file_path = if path == INDEX_FILE && !dist_path.is_file() {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(INDEX_FILE)
        } else {
            dist_path
        };
        if path == INDEX_FILE {
            let data = match std::fs::read(file_path) {
                Ok(data) => data,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            return Ok(Some(self.index_response(data, request)));
        }
        let file = match NamedFile::open_async(file_path).await {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Ok(Some(
            file.set_content_type(content_type(path))
                .into_response(request),
        ))
    }

    #[cfg(not(debug_assertions))]
    async fn response(&self, path: &str, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        let data = if path == INDEX_FILE {
            Cow::Borrowed(EMBEDDED_INDEX)
        } else {
            let Some(file) = EmbeddedDist::get(path) else {
                return Ok(None);
            };
            file.data
        };
        if path == INDEX_FILE {
            return Ok(Some(self.index_response(data.into_owned(), request)));
        }
        let length = data.len();
        let mut response = HttpResponse::Ok();
        response.insert_header((header::CONTENT_TYPE, content_type(path).to_string()));
        response.insert_header((header::CONTENT_LENGTH, length));
        if request.method() == Method::HEAD {
            Ok(Some(response.finish()))
        } else {
            Ok(Some(response.body(data.into_owned())))
        }
    }

    fn index_response(&self, data: Vec<u8>, request: &HttpRequest) -> HttpResponse {
        let source = String::from_utf8_lossy(&data);
        let body = source
            .replace(INDEX_BASE_HREF_PLACEHOLDER, &self.base_href)
            .into_bytes();
        let mut response = HttpResponse::Ok();
        response.insert_header((header::CONTENT_TYPE, "text/html; charset=utf-8"));
        response.insert_header((header::CONTENT_LENGTH, body.len()));
        if request.method() == Method::HEAD {
            response.finish()
        } else {
            response.body(body)
        }
    }

    fn request_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        if self.base_path == "/" {
            return path.strip_prefix('/');
        }
        if path == self.base_path {
            return Some("");
        }
        path.strip_prefix(&self.base_path)?.strip_prefix('/')
    }
}

pub(crate) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/pkg", web::route().to(not_found))
        .route("/pkg/{path:.*}", web::route().to(package_asset));
}

pub(crate) async fn fallback(
    request: HttpRequest,
    assets: web::Data<FrontendAssets>,
) -> Result<HttpResponse> {
    if !is_asset_method(request.method()) {
        return Ok(not_found_response());
    }

    let Some(path) = assets.request_path(request.path()) else {
        return Ok(not_found_response());
    };
    if path.is_empty() {
        return required_asset(&assets, INDEX_FILE, &request).await;
    }
    let Some(path) = safe_relative_path(path) else {
        return Ok(not_found_response());
    };
    if let Some(response) = assets.response(path, &request).await? {
        return Ok(response);
    }
    if looks_like_asset(path) {
        return Ok(not_found_response());
    }

    required_asset(&assets, INDEX_FILE, &request).await
}

async fn package_asset(
    request: HttpRequest,
    path: web::Path<String>,
    assets: web::Data<FrontendAssets>,
) -> Result<HttpResponse> {
    if !is_asset_method(request.method()) {
        return Ok(not_found_response());
    }
    let path = format!("pkg/{}", path.into_inner());
    let Some(path) = safe_relative_path(&path) else {
        return Ok(not_found_response());
    };
    required_asset(&assets, path, &request).await
}

async fn required_asset(
    assets: &FrontendAssets,
    path: &str,
    request: &HttpRequest,
) -> Result<HttpResponse> {
    Ok(assets
        .response(path, request)
        .await?
        .unwrap_or_else(not_found_response))
}

async fn not_found() -> HttpResponse {
    not_found_response()
}

pub(crate) async fn outside_base_path() -> HttpResponse {
    not_found_response()
}

fn not_found_response() -> HttpResponse {
    HttpResponse::NotFound()
        .content_type("text/plain; charset=utf-8")
        .body("404 Not Found")
}

fn is_asset_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
}

fn safe_relative_path(path: &str) -> Option<&str> {
    use std::path::Component;

    (!path.is_empty()
        && !path.contains('\\')
        && std::path::Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_))))
    .then_some(path)
}

fn looks_like_asset(path: &str) -> bool {
    path.starts_with("pkg/")
        || path.starts_with("assets/")
        || std::path::Path::new(path).extension().is_some()
}

fn content_type(path: &str) -> mime_guess::Mime {
    mime_guess::from_path(path).first_or_octet_stream()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_that_can_escape_the_asset_root() {
        assert!(safe_relative_path("pkg/patchwork_web.js").is_some());
        assert!(safe_relative_path("../patchwork.toml").is_none());
        assert!(safe_relative_path("pkg\\..\\patchwork.toml").is_none());
    }

    #[test]
    fn static_looking_paths_do_not_use_the_spa_fallback() {
        assert!(looks_like_asset("pkg/missing.wasm"));
        assert!(looks_like_asset("missing.css"));
        assert!(!looks_like_asset("browse"));
    }

    #[test]
    fn wasm_has_the_webassembly_content_type() {
        assert_eq!(
            content_type("pkg/patchwork_web.wasm").essence_str(),
            "application/wasm"
        );
    }
}
