use std::time::Instant;

use antweb::{rock::AppResult, route::api_router};
use axum::{
    middleware::{self, Next},
    response::IntoResponse,
};
use tracing::{Level, info};
use tracing_appender::non_blocking;
use tracing_subscriber::{filter::FilterFn, layer::SubscriberExt, util::SubscriberInitExt};

async fn log_http_middleware(request: axum::extract::Request, next: Next) -> impl IntoResponse {
    let start = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();

    let response = next.run(request).await;

    let duration = start.elapsed();
    info!(
        method = %method,
        uri = %uri,
        status = ?response.status(),
        latency = ?duration,
        "request completed",
    );

    response
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
    let app = api_router().layer(middleware::from_fn(log_http_middleware));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await?;
    Ok(())
}
