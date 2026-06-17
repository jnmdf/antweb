use axum::Router;

mod account;
mod share;

pub fn router() -> Router {
    Router::new()
        .merge(share::page_router())
        .nest("/api", api_router())
}

fn api_router() -> Router {
    Router::new()
        .merge(account::router())
        .merge(share::api_router())
}
