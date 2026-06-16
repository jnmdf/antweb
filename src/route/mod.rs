use axum::Router;

mod account;
mod share;

pub fn api_router() -> Router {
    Router::new()
        .merge(account::router())
        .merge(share::router())
}
