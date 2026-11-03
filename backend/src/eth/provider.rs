use std::{str::FromStr, time::Duration};

use alloy::{
    network::EthereumWallet,
    primitives::Address,
    providers::{ConnectionConfig, DynProvider, Provider as _, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use eyre::{Result, WrapErr, bail};
use sqlx::PgExecutor;

pub struct ProviderFromDb {
    pub alloy_provider: DynProvider,
    pub pool_manager: Address,
    pub signer: Address,
}

impl ProviderFromDb {
    pub async fn new<'e>(
        db: impl PgExecutor<'e>,
        chain_id: i64,
        private_key: &str,
    ) -> Result<Self> {
        let configs = sqlx::query_as::<_, (String, Vec<u8>)>(
            "SELECT rpc_url, uniswapv4_core_pool_manager
             FROM ethereum_config WHERE chain_id = $1 LIMIT 2",
        )
        .bind(chain_id)
        .fetch_all(db)
        .await?;
        let [(rpc_url, pool_manager)] = configs.as_slice() else {
            bail!("expected exactly one configuration for chain {chain_id}");
        };
        let pool_manager = Address::try_from(pool_manager.as_slice())
            .wrap_err("PoolManager must contain exactly 20 bytes")?;
        let signer = PrivateKeySigner::from_str(private_key)?;
        let signer_address = signer.address();
        let wallet = EthereumWallet::new(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_with_config(
                rpc_url,
                ConnectionConfig::new()
                    .with_retry_interval(Duration::from_secs(3))
                    .with_max_retries(5),
            )
            .await?;
        if provider.get_chain_id().await? != u64::try_from(chain_id)? {
            bail!("RPC chain ID does not match the requested chain");
        }
        Ok(Self {
            alloy_provider: provider.erased(),
            pool_manager,
            signer: signer_address,
        })
    }
}
