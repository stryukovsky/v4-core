mod config;
mod eth;
mod routes;
mod rpc;

use std::time::Duration;

use eyre::{Result, WrapErr};
use jsonrpsee::server::{ServerBuilder, ServerConfig, stop_channel};
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
        private_key: config.private_key,
    };
    let (stop, handle) = stop_channel();
    let service = ServerBuilder::default()
        .set_config(ServerConfig::builder().http_only().build())
        .to_service_builder()
        .build(rpc::module(state.clone()), stop);
    let service = axum::error_handling::HandleError::new(service, |_: axum::BoxError| async {
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    });
    let app = routes::router(state).route_service("/rpc", service);
    let listener = tokio::net::TcpListener::bind(config.api_addr).await?;
    tracing::info!(address = %listener.local_addr()?, "HTTP API and JSON-RPC listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = handle.stop();
        })
        .await?;
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
