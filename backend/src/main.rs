use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, I256, U160, aliases::I24},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use eyre::Result;
use reqwest::Url;

use crate::PoolManager::{PoolKey, SwapParams};

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    ERC20,
    "abi/ERC20.json"
);

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    PoolManager,
    "abi/IPoolManager.json"
);

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_url: Url = "http://localhost:8545".parse()?;
    let pool_manager_address: Address = "0x000000000004444c5dc75cB358380D2e3dE08A90".parse()?;
    let usdt_address: Address = "0xdAC17F958D2ee523a2206206994597C13D831ec7".parse()?;
    let usdc_address: Address = "0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse()?;
    let weth_address: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".parse()?;

    let private_key_mock: PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse()?;
    let signer_address = private_key_mock.address();
    let wallet = EthereumWallet::new(private_key_mock);

    let provider = ProviderBuilder::new().wallet(wallet).connect_http(rpc_url);

    let pool_manager = PoolManager::new(pool_manager_address, &provider);
    let usdt = ERC20::new(usdt_address, &provider);
    let eth_usdt_pool_key = PoolKey {
        currency0: Address::ZERO,
        currency1: usdt_address,
        fee: 3000.try_into()?,
        tickSpacing: I24::try_from(60)?,
        hooks: Address::ZERO,
    };
    let decimals_usdt_await = usdt.decimals();

    // Build calldata only: execution requires a router's unlock callback and settlement.
    pool_manager
        .swap(
            eth_usdt_pool_key,
            SwapParams {
                zeroForOne: true,
                amountSpecified: "-100000000000000000".parse::<I256>()?, // exact input: 0.1 ETH
                sqrtPriceLimitX96: U160::from(4_295_128_740u64),         // MIN_SQRT_PRICE + 1
            },
            Bytes::new(),
        )
        .send_sync()
        .await?;

    let native_balance = provider
        .get_balance(signer_address)
        .await?;
    let usdt_balance = usdt
        .balanceOf(signer_address)
        .call()
        .await?;

    println!("{}, {}", native_balance, usdt_balance);

    Ok(())
}
