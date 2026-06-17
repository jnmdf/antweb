use std::{net::SocketAddr, path::Path};

use rcgen::CertifiedKey;

use antweb::{config, rock::AppResult, route::router};
use axum::{
    middleware::{self, Next},
    response::IntoResponse,
};
use axum::http::{StatusCode, header};
use axum_server::tls_rustls::RustlsConfig;
use tracing::{Level, info};
use tracing_appender::non_blocking;
use tracing_subscriber::{filter::FilterFn, layer::SubscriberExt, util::SubscriberInitExt};

async fn favicon_middleware(request: axum::extract::Request, next: Next) -> impl IntoResponse {
    if request.uri().path() == "/favicon.ico" {
        return (
            StatusCode::NO_CONTENT,
            [(header::CACHE_CONTROL, "public, max-age=31536000, immutable")],
        )
            .into_response();
    }
    next.run(request).await
}

async fn log_http_middleware(request: axum::extract::Request, next: Next) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let version = request.version();

    let response = next.run(request).await;

    let duration = start.elapsed();
    let status = response.status();
    if status.is_server_error() {
        tracing::error!(
            method = %method,
            uri = %uri,
            version = ?version,
            status = %status,
            latency = ?duration,
            "request completed with server error",
        );
    } else {
        info!(
            method = %method,
            uri = %uri,
            version = ?version,
            status = %status,
            latency = ?duration,
            "request completed",
        );
    }

    response
}

fn ensure_dev_tls_certs(cert_path: &Path, key_path: &Path) -> AppResult<()> {
    if cert_path.exists() && key_path.exists() {
        return Ok(());
    }
    if let Some(parent) = cert_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec![
            "localhost".into(),
            "127.0.0.1".into(),
            "[::1]".into(),
        ])?;
    std::fs::write(cert_path, cert.pem())?;
    std::fs::write(key_path, signing_key.serialize_pem())?;
    info!(
        cert = %cert_path.display(),
        key = %key_path.display(),
        "generated self-signed TLS certificate for local HTTPS/HTTP2"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> AppResult<()> {
    let (non_blocking_writer, _guard) = non_blocking(std::io::stdout());
    tracing_subscriber::registry()
        .with(FilterFn::new(|metadata| {
            metadata.target().starts_with(env!("CARGO_PKG_NAME"))
                || *metadata.level() == Level::ERROR
        }))
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking_writer))
        .init();

    let pid = std::process::id();
    info!(pid = ?pid);

    let cert_path = Path::new(config::TLS_CERT);
    let key_path = Path::new(config::TLS_KEY);
    ensure_dev_tls_certs(cert_path, key_path)?;

    let tls = RustlsConfig::from_pem_file(cert_path, key_path).await?;
    let addr: SocketAddr = config::BIND_ADDR.parse()?;
    let app = router()
        .layer(middleware::from_fn(log_http_middleware))
        .layer(middleware::from_fn(favicon_middleware));

    info!(
        addr = %addr,
        "listening on https://127.0.0.1:{} (HTTP/2 over TLS; plain http:// stays on HTTP/1.1 in browsers)",
        addr.port()
    );

    axum_server::bind_rustls(addr, tls)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
