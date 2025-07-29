use rust_embed::RustEmbed;
use salvo::prelude::*;
use salvo::serve_static::static_embed;

use tracing_subscriber::EnvFilter;

mod broadcast;
mod single;

#[cfg(test)]
mod tests;

#[derive(RustEmbed)]
#[folder = "static"]
struct Assets;

#[handler]
async fn health(res: &mut Response) {
    res.status_code(StatusCode::OK);
}

#[handler]
async fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
#[tokio::main]
async fn main() {
    // Initialize logging subsystem
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Bind server to port 5800
    let acceptor = TcpListener::new("0.0.0.0:5800").bind().await;
    let static_files = Router::with_hoop(Compression::new().enable_gzip(CompressionLevel::Fastest))
        .path("{*path}")
        .get(static_embed::<Assets>().fallback("index.html"));

    let router = Router::new()
        .push(Router::with_path("single/sub").goal(single::user_connected))
        .push(Router::with_path("single/pub").post(single::publish_message))
        .push(Router::with_path("broad/sub").goal(broadcast::broadcast_subscribe))
        .push(Router::with_path("broad/pub").post(broadcast::broadcast_publish))
        .push(Router::with_path("health").goal(health))
        .push(Router::with_path("version").goal(version))
        .push(static_files);

    println!(
        "Notir server start, binding: {:?}",
        acceptor.local_addr().unwrap()
    );

    Server::new(acceptor).serve(router).await;
}
