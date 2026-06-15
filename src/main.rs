use antweb::{handler::router, rock::AppResult};
#[tokio::main]
async fn main() -> AppResult<()> {
    let app = router();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await?;
    println!("Hello, world!");
    Ok(())
}
