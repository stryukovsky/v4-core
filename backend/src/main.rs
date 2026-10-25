use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, I256, U160, U256, aliases::I24},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use eyre::Result;
use reqwest::Url;
use tokio::try_join;

use crate::MySwapper::{PoolKey, SwapParams};

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    ERC20,
    "abi/ERC20.json"
);

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    MySwapper,
    "abi/MySwapper.json"
);

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_url: Url = "http://localhost:8545".parse()?;
    let pool_manager_address: Address = "0x000000000004444c5dc75cB358380D2e3dE08A90".parse()?;
    let usdt_address: Address = "0xdAC17F958D2ee523a2206206994597C13D831ec7".parse()?;

    let private_key_mock: PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse()?;
    let signer_address = private_key_mock.address();
    let wallet = EthereumWallet::new(private_key_mock);

    let provider = ProviderBuilder::new().wallet(wallet).connect_http(rpc_url);

    let my_swapper = MySwapper::deploy(&provider, pool_manager_address).await?;
    let usdt = ERC20::new(usdt_address, &provider);
    let eth_usdt_pool_key = PoolKey {
        currency0: Address::ZERO,
        currency1: usdt_address,
        fee: 3000.try_into()?,
        tickSpacing: I24::try_from(60)?,
        hooks: Address::ZERO,
    };

    let native_balance_before = provider.get_balance(signer_address).await?;
    println!("Before {} ETH", native_balance_before);

    let eth_input = U256::from(100_000_000_000_000_000u64); // 0.1 ETH in wei
    // Fund the native currency input used by the swapper's unlock callback.
    my_swapper
        .swap(
            eth_usdt_pool_key,
            SwapParams {
                zeroForOne: true,
                amountSpecified: -I256::try_from(eth_input)?, // negative means exact input
                sqrtPriceLimitX96: U160::from(4_295_128_740u64), // MIN_SQRT_PRICE + 1 for zeroForOne
            },
            Bytes::new(),
        )
        .value(eth_input)
        .send_sync()
        .await?;

    let native_balance_after = provider.get_balance(signer_address).await?;
    let decimals_usdt_await = usdt.decimals();
    let usdt_balance_await = usdt.balanceOf(signer_address);
    let results = try_join!(decimals_usdt_await.call(), usdt_balance_await.call())?;

    println!("{} ETH, {} USDT", native_balance_after, results.1 / alloy::primitives::U256::from(results.0));

    Ok(())
}
