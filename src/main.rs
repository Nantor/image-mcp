mod config;
mod image_api;
mod image_ops;
mod image_store;
mod server;
mod tools;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use tracing_subscriber::EnvFilter;

use server::ImageMcpServer;

#[tokio::main]
async fn main() {
    // All logging must go to stderr — stdout is reserved for JSON-RPC on
    // the stdio transport.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = match config::load_config().and_then(|cfg| {
        if let Err(err) = config::validate_config(&cfg) {
            Err(err)
        } else {
            Ok(cfg)
        }
    }) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("image-mcp: failed to load config: {err}");
            std::process::exit(1);
        }
    };

    tracing::info!("Starting image-mcp server");

    let service = ImageMcpServer::new(config);

    let running = match service.serve(stdio()).await {
        Ok(running) => running,
        Err(err) => {
            tracing::error!("failed to start serving: {err:?}");
            std::process::exit(1);
        }
    };

    if let Err(err) = running.waiting().await {
        tracing::error!("server error: {err:?}");
        std::process::exit(1);
    }
}
