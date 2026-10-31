#[path = "../config.rs"]
mod config;

use std::{collections::BTreeMap, path::PathBuf};

use eyre::{Result, WrapErr, bail, eyre};
use sqlx::{Connection, PgConnection};

fn scripts(directory: &str) -> Result<BTreeMap<i64, PathBuf>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("sql")
        .join(directory);
    let mut scripts = BTreeMap::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "sql") {
            continue;
        }
        let version = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split_once('_'))
            .map(|(version, _)| version)
            .ok_or_else(|| eyre!("expected <number>_<description>.sql: {}", path.display()))?
            .parse::<i64>()?;
        if scripts.insert(version, path).is_some() {
            bail!("duplicate migration number {version} in sql/{directory}");
        }
    }
    Ok(scripts)
}

async fn execute(
    connection: &mut PgConnection,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<()> {
    for path in paths {
        println!("Running {}", path.display());
        sqlx::raw_sql(&std::fs::read_to_string(&path)?)
            .execute(&mut *connection)
            .await
            .wrap_err_with(|| format!("failed to execute {}", path.display()))?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .ok_or_else(|| eyre!("missing database command"))?;
    if !matches!(
        command.as_str(),
        "db-migrate" | "db-purify" | "db-load" | "db-reload" | "db-clear-structs"
    ) {
        bail!("unknown database command: {command}");
    }
    let config = config::Config::load()?;
    // API_ADDR is shared configuration; database commands do not bind it.
    let _ = config.api_addr;
    let mut connection = PgConnection::connect(&config.database_url).await?;
    if command == "db-migrate" {
        sqlx::migrate::Migrator::new(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/sql/create"
        )))
        .await?
        .run(&mut connection)
        .await?;
        return Ok(());
    }

    let mut transaction = connection.begin().await?;
    let mut applied = Vec::new();
    let reverts = if command == "db-clear-structs" {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
            .fetch_one(&mut *transaction)
            .await?;
        if exists {
            applied = sqlx::query_scalar::<_, i64>(
                "SELECT version FROM _sqlx_migrations WHERE success ORDER BY version DESC",
            )
            .fetch_all(&mut *transaction)
            .await?;
        }
        let pairs = scripts("revert")?;
        let mut paths = Vec::new();
        for version in &applied {
            paths.push(
                pairs
                    .get(version)
                    .ok_or_else(|| eyre!("missing revert script for migration {version}"))?
                    .clone(),
            );
        }
        paths
    } else {
        Vec::new()
    };
    if matches!(
        command.as_str(),
        "db-purify" | "db-reload" | "db-clear-structs"
    ) {
        // Clear dependent tables before the tables they reference.
        execute(&mut transaction, scripts("purify")?.into_values().rev()).await?;
    }
    if matches!(command.as_str(), "db-load" | "db-reload") {
        let rpc_url = std::env::var("RPC_URL")
            .wrap_err("RPC_URL is required to load the Ethereum mainnet fixture")?;
        if rpc_url.trim().is_empty() {
            bail!("RPC_URL must not be empty");
        }
        sqlx::query("SELECT set_config('app.rpc_url', $1, true)")
            .bind(&rpc_url)
            .execute(&mut *transaction)
            .await?;
        execute(&mut transaction, scripts("load")?.into_values()).await?;
    }
    if command == "db-clear-structs" {
        execute(&mut transaction, reverts).await?;
        for version in applied {
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
                .bind(version)
                .execute(&mut *transaction)
                .await?;
        }
    }
    transaction.commit().await?;
    Ok(())
}
