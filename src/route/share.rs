use std::{
    env,
    path::{Component, Path, PathBuf},
};

use axum::{
    Router,
    extract::{
        DefaultBodyLimit, Multipart, Path as AxumPath,
        multipart::{Field, MultipartError},
    },
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect},
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeDir;
use tracing::warn;

use crate::{
    config,
    rock::{AppError, AppResult},
};

const PAGE_DIR: &str = "/dir";
const API_FILE: &str = "/api/file";
const API_UPLOAD: &str = "/api/upload";

pub fn page_router() -> Router {
    Router::new()
        .route("/", get(redirect_to_dir))
        .route("/dir", get(traverse_root))
        .route("/dir/", get(traverse_root))
        .route("/dir/{*path}", get(traverse_dir))
}

pub fn api_router() -> Router {
    Router::new()
        .nest_service("/file", ServeDir::new("."))
        .route("/upload", post(upload_root))
        .route("/upload/", post(upload_root))
        .route("/upload/{*path}", post(upload_dir))
        .layer(DefaultBodyLimit::max(config::UPLOAD_MAX_BODY))
}

async fn redirect_to_dir() -> Redirect {
    Redirect::to(PAGE_DIR)
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
    let mut entries = fs::read_dir(path).await?;
    let mut items = vec![];
    while let Some(entry) = entries.next_entry().await? {
        let ft = entry.file_type().await?;
        let abs_path = entry.path();
        let rel_path = abs_path.strip_prefix(root)?;
        let rel_path = rel_path.display().to_string();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = ft.is_dir();
        let href = if is_dir {
            format!("{PAGE_DIR}/{rel_path}")
        } else {
            format!("{API_FILE}/{rel_path}")
        };
        let meta = if is_dir {
            "Folder".to_string()
        } else {
            format_size(fs::metadata(&abs_path).await?.len())
        };
        items.push((is_dir, name, href, meta));
    }
    items.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let rel_dir = path.strip_prefix(root)?.display().to_string();
    let upload_url = if rel_dir.is_empty() {
        API_UPLOAD.to_string()
    } else {
        format!("{API_UPLOAD}/{rel_dir}")
    };

    let mut rows = String::new();
    if !rel_dir.is_empty() {
        let parent_href = Path::new(&rel_dir)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| !p.is_empty())
            .map(|p| format!("{PAGE_DIR}/{p}"))
            .unwrap_or_else(|| PAGE_DIR.to_string());
        rows.push_str(&entry_row(
            true,
            "..",
            &parent_href,
            "Parent directory",
            false,
        ));
    }
    for (is_dir, name, href, meta) in items {
        rows.push_str(&entry_row(is_dir, &name, &href, &meta, !is_dir));
    }
    let listing = if rows.is_empty() {
        r#"<div class="empty">This directory is empty</div>"#.to_string()
    } else {
        rows
    };

    Ok(Html(page_html(
        &breadcrumb_html(&rel_dir),
        &upload_url,
        &listing,
    )))
}

type UploadResponse = (StatusCode, Json<Value>);

enum UploadError {
    Multipart(MultipartError),
    App(AppError),
}

impl From<AppError> for UploadError {
    fn from(err: AppError) -> Self {
        Self::App(err)
    }
}

impl IntoResponse for UploadError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Multipart(err) => err.into_response(),
            Self::App(err) => err.into_response(),
        }
    }
}

async fn upload_root(mut multipart: Multipart) -> Result<UploadResponse, UploadError> {
    let base = env::current_dir()
        .map_err(AppError::from)
        .map_err(UploadError::App)?;
    upload_to(base, &mut multipart).await
}

async fn upload_dir(
    AxumPath(path): AxumPath<PathBuf>,
    mut multipart: Multipart,
) -> Result<UploadResponse, UploadError> {
    let base = env::current_dir()
        .map_err(AppError::from)
        .map_err(UploadError::App)?;
    let rel = path.display().to_string();
    let Some(dest) = safe_join(&base, &rel) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid upload path" })),
        ));
    };
    if !dest.is_dir() {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "directory not found" })),
        ));
    }
    upload_to(dest, &mut multipart).await
}

async fn upload_to(
    base: PathBuf,
    multipart: &mut Multipart,
) -> Result<UploadResponse, UploadError> {
    let mut count = 0usize;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(UploadError::Multipart)?
    {
        if field.name() != Some("file") {
            continue;
        }
        let Some(rel_path) = field.file_name().map(str::to_owned) else {
            continue;
        };
        let Some(dest) = safe_join(&base, &rel_path) else {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid path: {rel_path}") })),
            ));
        };
        save_upload_field(&dest, field).await?;
        count += 1;
    }
    Ok((StatusCode::OK, Json(json!({ "uploaded": count }))))
}

async fn save_upload_field(dest: &Path, mut field: Field<'_>) -> Result<(), UploadError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(AppError::from)
            .map_err(UploadError::App)?;
    }

    match write_field_to_disk(dest, &mut field).await {
        Ok(()) => Ok(()),
        Err(err) => {
            cleanup_partial_file(dest).await;
            Err(err)
        }
    }
}

async fn write_field_to_disk(dest: &Path, field: &mut Field<'_>) -> Result<(), UploadError> {
    let mut file = fs::File::create(dest)
        .await
        .map_err(AppError::from)
        .map_err(UploadError::App)?;
    while let Some(chunk) = field.chunk().await.map_err(UploadError::Multipart)? {
        file.write_all(&chunk)
            .await
            .map_err(AppError::from)
            .map_err(UploadError::App)?;
    }
    Ok(())
}

async fn cleanup_partial_file(path: &Path) {
    match fs::remove_file(path).await {
        Ok(()) => warn!(path = %path.display(), "removed partial upload"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => warn!(
            path = %path.display(),
            error = %err,
            "failed to remove partial upload",
        ),
    }
}

fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel);
    if path.is_absolute() {
        return None;
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            _ => {}
        }
    }
    Some(base.join(path))
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = UNITS[0];
    for next in UNITS.iter().skip(1) {
        if size < 1024.0 {
            break;
        }
        size /= 1024.0;
        unit = next;
    }
    if unit == "B" {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {unit}")
    }
}

fn breadcrumb_html(rel_dir: &str) -> String {
    if rel_dir.is_empty() {
        return r#"<nav class="breadcrumb"><span class="current">/</span></nav>"#.to_string();
    }

    let segments: Vec<&str> = rel_dir.split('/').filter(|s| !s.is_empty()).collect();
    let mut acc = String::new();
    let mut segment_parts = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            acc.push('/');
        }
        acc.push_str(segment);
        let escaped = html_escape(segment);
        if index + 1 == segments.len() {
            segment_parts.push(format!(r#"<span class="current">{escaped}</span>"#));
        } else {
            segment_parts.push(format!(r#"<a href="{PAGE_DIR}/{acc}">{escaped}</a>"#));
        }
    }

    format!(
        r#"<nav class="breadcrumb"><a href="{PAGE_DIR}">/</a>{}</nav>"#,
        segment_parts.join(r#"<span class="sep">/</span>"#)
    )
}

fn entry_row(is_dir: bool, name: &str, href: &str, meta: &str, download: bool) -> String {
    let kind = if is_dir { "folder" } else { "file" };
    let icon = if is_dir {
        r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10 4l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h6z"/></svg>"#
    } else {
        r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zm-1 2l5 5h-5V4zM8 13h8v2H8v-2zm0 4h8v2H8v-2z"/></svg>"#
    };
    let download_attr = if download { r#" download"# } else { "" };
    format!(
        r#"<a class="entry {kind}" href="{href}"{download_attr}>
  <span class="icon">{icon}</span>
  <span class="name">{name}</span>
  <span class="meta">{meta}</span>
</a>"#,
        kind = kind,
        href = html_escape(href),
        download_attr = download_attr,
        icon = icon,
        name = html_escape(name),
        meta = html_escape(meta),
    )
}

fn page_html(breadcrumb: &str, upload_url: &str, listing: &str) -> String {
    let upload_url = serde_json::to_string(upload_url).unwrap_or_else(|_| "\"/api/upload\"".into());
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="icon" href="data:;base64,=">
<title>File Share</title>
<style>
  :root {{
    color-scheme: light;
    --bg: #f3f4f6;
    --panel: #ffffff;
    --border: #e5e7eb;
    --text: #111827;
    --muted: #6b7280;
    --accent: #2563eb;
    --accent-soft: #eff6ff;
    --folder: #f59e0b;
    --file: #64748b;
    --hover: #f8fafc;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    margin: 0;
    background: var(--bg);
    color: var(--text);
    line-height: 1.5;
  }}
  .page {{
    max-width: 980px;
    margin: 0 auto;
    padding: 2rem 1.25rem 3rem;
  }}
  .header {{
    margin-bottom: 1.25rem;
  }}
  h1 {{
    margin: 0 0 0.35rem;
    font-size: 1.75rem;
    font-weight: 700;
  }}
  .subtitle {{
    color: var(--muted);
    font-size: 0.95rem;
  }}
  .breadcrumb {{
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    margin-top: 0.75rem;
    padding: 0.65rem 0.85rem;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 0.92rem;
  }}
  .breadcrumb a {{
    color: var(--accent);
    text-decoration: none;
  }}
  .breadcrumb a:hover {{ text-decoration: underline; }}
  .breadcrumb .sep {{ color: #cbd5e1; user-select: none; }}
  .breadcrumb .current {{ color: var(--text); font-weight: 600; }}
  .panel {{
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    overflow: hidden;
    box-shadow: 0 1px 2px rgba(15, 23, 42, 0.04);
  }}
  #dropzone {{
    border-bottom: 1px solid var(--border);
    padding: 1.35rem 1rem;
    text-align: center;
    color: var(--muted);
    background: linear-gradient(180deg, #fafafa 0%, #ffffff 100%);
    transition: border-color 0.15s, background 0.15s, color 0.15s;
    cursor: pointer;
  }}
  #dropzone.dragover {{
    background: var(--accent-soft);
    color: var(--accent);
  }}
  #dropzone.uploading {{ opacity: 0.65; pointer-events: none; }}
  .drop-title {{ font-weight: 600; color: var(--text); margin-bottom: 0.25rem; }}
  .actions {{
    margin-top: 0.85rem;
    display: flex;
    gap: 0.65rem;
    justify-content: center;
    flex-wrap: wrap;
  }}
  button {{
    padding: 0.45rem 0.95rem;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: #fff;
    cursor: pointer;
    font: inherit;
    color: var(--text);
  }}
  button:hover {{ border-color: #cbd5e1; background: var(--hover); }}
  #status {{
    min-height: 1.25rem;
    margin: 0.85rem 0 0;
    color: var(--muted);
    font-size: 0.92rem;
  }}
  .list-head {{
    display: grid;
    grid-template-columns: 2.5rem 1fr auto;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--border);
    background: #f9fafb;
    color: var(--muted);
    font-size: 0.78rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }}
  .file-list {{ display: flex; flex-direction: column; }}
  .entry {{
    display: grid;
    grid-template-columns: 2.5rem 1fr auto;
    gap: 0.75rem;
    align-items: center;
    padding: 0.85rem 1rem;
    border-bottom: 1px solid var(--border);
    color: inherit;
    text-decoration: none;
    transition: background 0.12s ease;
  }}
  .entry:last-child {{ border-bottom: none; }}
  .entry:hover {{ background: var(--hover); }}
  .entry .icon {{
    display: flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: 8px;
    background: #f8fafc;
  }}
  .entry.folder .icon {{ color: var(--folder); background: #fff7ed; }}
  .entry.file .icon {{ color: var(--file); background: #f1f5f9; }}
  .entry svg {{ width: 1.15rem; height: 1.15rem; fill: currentColor; }}
  .entry .name {{
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }}
  .entry.folder .name {{ color: #92400e; }}
  .entry .meta {{
    color: var(--muted);
    font-size: 0.88rem;
    white-space: nowrap;
  }}
  .empty {{
    padding: 2rem 1rem;
    text-align: center;
    color: var(--muted);
  }}
  input[type=file] {{ display: none; }}
  @media (max-width: 640px) {{
    .list-head {{ display: none; }}
    .entry {{ grid-template-columns: 2.5rem 1fr; }}
    .entry .meta {{ grid-column: 2; font-size: 0.82rem; }}
  }}
</style>
</head>
<body>
  <div class="page">
    <div class="header">
      <h1>File Share</h1>
      <div class="subtitle">Browse, download, and upload files</div>
      {breadcrumb}
    </div>
    <div class="panel">
      <div id="dropzone">
        <div class="drop-title">Drag files or folders here to upload</div>
        <div>Files keep their folder structure when uploading directories</div>
        <div class="actions">
          <button type="button" id="pick-files">Choose files</button>
          <button type="button" id="pick-folder">Choose folder</button>
        </div>
        <div id="status"></div>
      </div>
      <div class="list-head">
        <span></span>
        <span>Name</span>
        <span>Info</span>
      </div>
      <div class="file-list">{listing}</div>
    </div>
  </div>
  <input type="file" id="file-input" multiple>
  <input type="file" id="folder-input" webkitdirectory multiple>
<script>
const uploadUrl = {upload_url};
const dropzone = document.getElementById('dropzone');
const statusEl = document.getElementById('status');
const fileInput = document.getElementById('file-input');
const folderInput = document.getElementById('folder-input');

function setStatus(text) {{
  statusEl.textContent = text;
}}

async function readAllEntries(reader) {{
  const entries = [];
  let batch;
  do {{
    batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
    entries.push(...batch);
  }} while (batch.length > 0);
  return entries;
}}

async function collectFromEntry(entry, prefix, out) {{
  if (entry.isFile) {{
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
    out.push({{ file, path: prefix + entry.name }});
    return;
  }}
  if (entry.isDirectory) {{
    const reader = entry.createReader();
    const entries = await readAllEntries(reader);
    const nextPrefix = prefix + entry.name + '/';
    for (const child of entries) {{
      await collectFromEntry(child, nextPrefix, out);
    }}
  }}
}}

async function collectFromDataTransfer(dataTransfer) {{
  const out = [];
  const items = dataTransfer.items;
  if (items && items.length > 0) {{
    for (const item of items) {{
      if (item.kind !== 'file') continue;
      const entry = item.webkitGetAsEntry ? item.webkitGetAsEntry() : null;
      if (entry) {{
        await collectFromEntry(entry, '', out);
      }} else {{
        const file = item.getAsFile();
        if (file) out.push({{ file, path: file.name }});
      }}
    }}
    return out;
  }}
  for (const file of dataTransfer.files) {{
    out.push({{ file, path: file.webkitRelativePath || file.name }});
  }}
  return out;
}}

function collectFromFileList(fileList) {{
  return Array.from(fileList).map((file) => ({{
    file,
    path: file.webkitRelativePath || file.name,
  }}));
}}

const SMALL_FILE_LIMIT = 256 * 1024;
const SMALL_BATCH_SIZE = 20;
const UPLOAD_CONCURRENCY = 6;

function splitEntries(entries) {{
  const small = [];
  const large = [];
  for (const entry of entries) {{
    if (entry.file.size < SMALL_FILE_LIMIT) {{
      small.push(entry);
    }} else {{
      large.push(entry);
    }}
  }}
  return {{ small, large }};
}}

function chunkEntries(items, size) {{
  const chunks = [];
  for (let i = 0; i < items.length; i += size) {{
    chunks.push(items.slice(i, i + size));
  }}
  return chunks;
}}

function buildUploadTasks(entries) {{
  const {{ small, large }} = splitEntries(entries);
  const tasks = [];
  for (const entry of large) {{
    tasks.push({{ kind: 'single', entries: [entry], fileCount: 1 }});
  }}
  for (const batch of chunkEntries(small, SMALL_BATCH_SIZE)) {{
    tasks.push({{ kind: 'batch', entries: batch, fileCount: batch.length }});
  }}
  return tasks;
}}

async function uploadOne({{ file, path }}) {{
  const formData = new FormData();
  formData.append('file', file, path);
  const resp = await fetch(uploadUrl, {{ method: 'POST', body: formData }});
  const data = await resp.json().catch(() => ({{}}));
  if (!resp.ok) {{
    throw new Error(data.error || `Upload failed (${{resp.status}}): ${{path}}`);
  }}
  return data.uploaded ?? 1;
}}

async function uploadBatch(batchEntries) {{
  const formData = new FormData();
  for (const {{ file, path }} of batchEntries) {{
    formData.append('file', file, path);
  }}
  const resp = await fetch(uploadUrl, {{ method: 'POST', body: formData }});
  const data = await resp.json().catch(() => ({{}}));
  if (!resp.ok) {{
    const sample = batchEntries[0]?.path || 'batch';
    throw new Error(data.error || `Batch upload failed (${{resp.status}}): ${{sample}}`);
  }}
  return data.uploaded ?? batchEntries.length;
}}

async function runUploadTask(task) {{
  if (task.kind === 'batch') {{
    return uploadBatch(task.entries);
  }}
  return uploadOne(task.entries[0]);
}}

async function runPool(items, limit, worker) {{
  let index = 0;
  async function next() {{
    while (true) {{
      const current = index++;
      if (current >= items.length) return;
      await worker(items[current], current);
    }}
  }}
  const workers = Array.from(
    {{ length: Math.min(limit, items.length) }},
    () => next(),
  );
  await Promise.all(workers);
}}

async function uploadFiles(entries) {{
  if (!entries.length) {{
    setStatus('No files selected.');
    return;
  }}
  dropzone.classList.add('uploading');
  let done = 0;
  let failed = 0;
  let lastError = '';
  const total = entries.length;
  const tasks = buildUploadTasks(entries);
  setStatus(`Uploading 0/${{total}} file(s)...`);

  try {{
    await runPool(tasks, UPLOAD_CONCURRENCY, async (task) => {{
      try {{
        const uploaded = await runUploadTask(task);
        done += uploaded;
      }} catch (err) {{
        failed += task.fileCount;
        lastError = err.message || 'Upload failed.';
      }}
      setStatus(`Uploading ${{done + failed}}/${{total}} (${{done}} ok, ${{failed}} failed)...`);
    }});

    if (done === 0) {{
      throw new Error(lastError || 'Upload failed.');
    }}
    if (failed > 0) {{
      setStatus(`Uploaded ${{done}} file(s), ${{failed}} failed. Reloading...`);
    }} else {{
      setStatus(`Uploaded ${{done}} file(s). Reloading...`);
    }}
    location.reload();
  }} catch (err) {{
    setStatus(err.message || 'Upload failed.');
    dropzone.classList.remove('uploading');
  }}
}}

['dragenter', 'dragover'].forEach((eventName) => {{
  dropzone.addEventListener(eventName, (event) => {{
    event.preventDefault();
    dropzone.classList.add('dragover');
  }});
}});
['dragleave', 'drop'].forEach((eventName) => {{
  dropzone.addEventListener(eventName, (event) => {{
    event.preventDefault();
    dropzone.classList.remove('dragover');
  }});
}});
dropzone.addEventListener('drop', async (event) => {{
  const entries = await collectFromDataTransfer(event.dataTransfer);
  await uploadFiles(entries);
}});
dropzone.addEventListener('click', (event) => {{
  if (event.target.tagName === 'BUTTON') return;
  fileInput.click();
}});
document.getElementById('pick-files').addEventListener('click', (event) => {{
  event.stopPropagation();
  fileInput.click();
}});
document.getElementById('pick-folder').addEventListener('click', (event) => {{
  event.stopPropagation();
  folderInput.click();
}});
fileInput.addEventListener('change', async () => {{
  const entries = collectFromFileList(fileInput.files);
  fileInput.value = '';
  await uploadFiles(entries);
}});
folderInput.addEventListener('change', async () => {{
  const entries = collectFromFileList(folderInput.files);
  folderInput.value = '';
  await uploadFiles(entries);
}});
</script>
</body>
</html>"#,
    )
}
