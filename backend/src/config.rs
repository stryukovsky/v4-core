use std::{env, net::SocketAddr};

use eyre::{Result, WrapErr};

pub struct Config {
    pub api_addr: SocketAddr,
    pub database_url: String,
    pub private_key: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        // Always use the backend env file, even when launched from the repo root.
        match dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")) {
            Ok(_) => {}
            Err(error) if error.not_found() => {}
            Err(error) => return Err(error).wrap_err("failed to load backend/.env"),
        }
        Ok(Self {
            api_addr: env::var("API_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:3000".into())
                .parse()
                .wrap_err("API_ADDR must be an IP address and port")?,
            database_url: env::var("DATABASE_URL").wrap_err("DATABASE_URL is required")?,
            private_key: env::var("PRIVATE_KEY").wrap_err("PRIVATE_KEY is required")?,
        })
    }
}
