use std::{
    env,
    path::{Path, PathBuf},
};

use axum::{Router, http::StatusCode, response::Html, routing::get};
use tokio::fs;
use tower_http::services::ServeDir;

use crate::rock::AppResult;

pub fn router() -> Router {
    Router::new()
        .nest_service("/file", ServeDir::new("."))
        .route("/dir/{*path}", get(traverse_dir))
        .route("/dir", get(traverse_root))
        .route("/dir/", get(traverse_root))
}

async fn traverse_root() -> AppResult<Html<String>> {
    let root = env::current_dir()?;
    traverse(&root, &root).await
}
async fn traverse_dir(
    axum::extract::Path(path): axum::extract::Path<PathBuf>,
) -> AppResult<(StatusCode, Html<String>)> {
    let base = env::current_dir()?;
    let cwd = base.join(&path);
    if !cwd.is_dir() {
        return Ok((
            StatusCode::NOT_FOUND,
            Html(format!("<h1>Directory {:?} not found</h1>", path)),
        ));
    }
    Ok((StatusCode::OK, traverse(&base, &cwd).await?))
}
async fn traverse(root: &Path, path: &Path) -> AppResult<Html<String>> {
    let mut entries = fs::read_dir(&path).await?;
    let mut items = vec![];
    while let Some(entry) = entries.next_entry().await? {
        let ft = entry.file_type().await?;
        let abs_path = entry.path();
        let rel_path = abs_path.strip_prefix(root)?;
        let rel_path = rel_path.display().to_string();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = ft.is_dir();
        let link = if is_dir {
            format!(r#"📁 <a href="/dir/{rel_path}">{name}/</a>"#)
        } else {
            format!(r#"📄 <a href="/file/{rel_path}" download>{name}</a>"#)
        };
        items.push((is_dir, name, link));
    }
    items.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let show: String = items
        .into_iter()
        .map(|(_, _, link)| format!("<li>{link}</li>"))
        .collect();
    Ok(Html(format!(
        "<!DOCTYPE html><html><head><title>File Share</title></head><body><h1>File Share</h1><ul>{show}</ul></body></html>",
    )))
}
