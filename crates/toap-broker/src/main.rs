//! TOAP broker binary. Host/port from `TOAP_HOST` / `TOAP_PORT` (default 127.0.0.1:7700).

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let host = std::env::var("TOAP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("TOAP_PORT").unwrap_or_else(|_| "7700".to_string());
    let addr = format!("{host}:{port}");
    println!("[broker] starting TOAP broker v{}", env!("CARGO_PKG_VERSION"));
    toap_broker::serve(&addr).await
}
