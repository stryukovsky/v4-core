mod config;
mod eth;
mod rpc;

use std::time::Duration;

use eyre::{Result, WrapErr};
use jsonrpsee::server::{ServerBuilder, ServerConfig};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub private_key: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = config::Config::load()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .wrap_err("failed to connect to PostgreSQL")?;

    sqlx::migrate::Migrator::new(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/sql/create"
    )))
    .await?
    .run(&db)
    .await
    .wrap_err("failed to run database migrations")?;

    let state = AppState {
        db: db.clone(),
        // Moves String ownership (pointer, length, capacity) into AppState, reusing
        // its heap buffer without allocating. config.private_key is moved from.
        private_key: config.private_key,
    };
    let server = ServerBuilder::default()
        .set_config(ServerConfig::builder().http_only().build())
        .build(config.api_addr)
        .await?;
    tracing::info!(address = %server.local_addr()?, "JSON-RPC listening");
    let handle = server.start(rpc::module(state));
    shutdown_signal().await;
    handle.stop()?;
    handle.stopped().await;
    db.close().await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
