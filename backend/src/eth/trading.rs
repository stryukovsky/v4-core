use alloy::{
    primitives::{Address, B256, Bytes, I256, U160, U256, U512, aliases::I24},
    sol,
};
use eyre::{Result, bail, eyre};
use serde::{Deserialize, Serialize};

use crate::{AppState, eth::ProviderFromDb};

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    MySwapper,
    "abi/MySwapper.json"
);
sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    ERC20,
    "abi/ERC20.json"
);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployParams {
    pub chain_id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwapParams {
    pub chain_id: i64,
    pub pool_id: B256,
    pub input_token: String,
    pub output_token: String,
    /// Decimal integer in the input token's smallest unit.
    pub input_amount: String,
}

impl SwapParams {
    pub fn parse(&self) -> Result<(Address, Address, U256)> {
        let input_token = self
            .input_token
            .parse::<Address>()
            .map_err(|_| eyre!("input_token must be a 20-byte hex address string"))?;
        let output_token = self
            .output_token
            .parse::<Address>()
            .map_err(|_| eyre!("output_token must be a 20-byte hex address string"))?;
        if self.chain_id <= 0 || input_token == output_token {
            bail!("positive chain_id and distinct tokens required");
        }
        if self.input_amount.is_empty() || !self.input_amount.bytes().all(|b| b.is_ascii_digit()) {
            bail!("input_amount must be a decimal integer string");
        }
        let amount: U256 = self.input_amount.parse()?;
        if amount.is_zero() || amount > U256::from(i128::MAX as u128) {
            bail!("input_amount must be between 1 and 2^127 - 1");
        }
        Ok((input_token, output_token, amount))
    }
}

#[derive(Clone, Serialize)]
pub struct DeploymentResponse {
    pub chain_id: i64,
    pub contract_address: String,
}

#[derive(Clone, Serialize)]
pub struct SwapResponse {
    pub tx_id: String,
    pub input_amount: String,
    pub output_amount: String,
    pub price: String,
    #[serde(rename = "humanPrice")]
    pub human_price: String,
    pub slippage: String,
}

// Serialize signer transactions across requests and API processes sharing this database.
// Hold this transaction open until confirmation and persistence complete.
async fn lock_chain(
    state: &AppState,
    chain_id: i64,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(chain_id)
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

pub async fn deploy_swapper_contract(state: &AppState, chain_id: i64) -> Result<DeploymentResponse> {
    if chain_id <= 0 {
        bail!("chain_id must be positive");
    }
    let mut tx = lock_chain(state, chain_id).await?;
    let provider = ProviderFromDb::new(&mut *tx, chain_id, &state.private_key).await?;
    let contract = MySwapper::deploy(&provider.alloy_provider, provider.pool_manager).await?;
    let address = *contract.address();
    sqlx::query("UPDATE ethereum_config SET my_swapper_contract = $1 WHERE chain_id = $2")
        .bind(address.as_slice())
        .bind(chain_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE pools SET interaction_contract = $1 WHERE chain_id = $2 AND protocol = 'uni_v4' AND base_contract = $3")
        .bind(address.as_slice()).bind(chain_id).bind(provider.pool_manager.as_slice())
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(DeploymentResponse {
        chain_id,
        contract_address: address.to_string(),
    })
}

pub async fn execute_swap(state: &AppState, params: SwapParams) -> Result<SwapResponse> {
    let (input_token, output_token, amount) = params.parse()?;
    let mut tx = lock_chain(state, params.chain_id).await?;
    let (token0, token1) = if input_token < output_token {
        (input_token, output_token)
    } else {
        (output_token, input_token)
    };
    // Select the stored canonical ID; tokens also validate the requested direction.
    let pools = sqlx::query_as::<_, (Vec<u8>, Option<Vec<u8>>, i32, i32, Vec<u8>)>(
        "SELECT base_contract, interaction_contract, fee, tick_spacing, hooks FROM pools
         WHERE chain_id = $1 AND pool_id = $2 AND token0 = $3 AND token1 = $4 AND protocol = 'uni_v4' LIMIT 2",
    )
    .bind(params.chain_id)
    .bind(params.pool_id.as_slice())
    .bind(token0.as_slice())
    .bind(token1.as_slice())
    .fetch_all(&mut *tx)
    .await?;
    let [(base, interaction, fee, spacing, hooks)] = pools.as_slice() else {
        bail!("expected exactly one pool for pool_id and token pair");
    };
    let interaction = Address::try_from(
        interaction
            .as_ref()
            .ok_or_else(|| eyre!("deploy a swapper first"))?
            .as_slice(),
    )?;
    let provider = ProviderFromDb::new(&mut *tx, params.chain_id, &state.private_key).await?;
    let base = Address::try_from(base.as_slice())?;
    if provider.pool_manager != base {
        bail!("pool base does not match chain configuration");
    }
    let swapper = MySwapper::new(interaction, &provider.alloy_provider);
    if swapper.poolManager().call().await? != base {
        bail!("swapper has wrong PoolManager");
    }
    let signer = provider.signer;
    let input_decimals = decimals(&provider, input_token).await?;
    let output_decimals = decimals(&provider, output_token).await?;
    if input_token != Address::ZERO {
        let token = ERC20::new(input_token, &provider.alloy_provider);
        let allowance = token.allowance(signer, interaction).call().await?;
        if allowance < amount {
            // USDT requires zeroing a nonzero allowance before replacing it.
            if !allowance.is_zero() {
                let receipt = token
                    .approve(interaction, U256::ZERO)
                    .send()
                    .await?
                    .get_receipt()
                    .await?;
                if !receipt.status() {
                    bail!("approval reset reverted");
                }
            }
            let receipt = token
                .approve(interaction, amount)
                .send()
                .await?
                .get_receipt()
                .await?;
            if !receipt.status() || token.allowance(signer, interaction).call().await? < amount {
                bail!("token approval failed");
            }
        }
    }
    let zero_for_one = input_token == token0;
    let key = MySwapper::PoolKey {
        currency0: token0,
        currency1: token1,
        fee: (*fee as u32).try_into()?,
        tickSpacing: I24::try_from(*spacing)?,
        hooks: Address::try_from(hooks.as_slice())?,
    };
    let swap_params = MySwapper::SwapParams {
        zeroForOne: zero_for_one,
        amountSpecified: -I256::try_from(amount)?,
        sqrtPriceLimitX96: if zero_for_one {
            U160::from(4_295_128_740u64)
        } else {
            "1461446703485210103287273052203988822378723970341".parse()?
        },
    };
    let call = swapper
        .swap(key, swap_params, Bytes::new())
        .from(signer)
        .value(if input_token == Address::ZERO {
            amount
        } else {
            U256::ZERO
        });
    let (quoted_input, quoted_output) = uniswap_v4_delta_to_uint128_amounts(call.call().await?, zero_for_one)?;
    let receipt = call.send().await?.get_receipt().await?;
    if !receipt.status() {
        bail!("swap reverted: {}", receipt.transaction_hash);
    }
    let executions: Vec<_> = receipt
        .inner
        .logs()
        .iter()
        .filter(|log| log.address() == interaction)
        .filter_map(|log| log.log_decode::<MySwapper::SwapExecuted>().ok())
        .filter(|log| log.inner.data.sender == signer)
        .collect();
    let [execution] = executions.as_slice() else {
        bail!(
            "expected one SwapExecuted event in {}",
            receipt.transaction_hash
        );
    };
    let (actual_input, actual_output) = uniswap_v4_delta_to_uint128_amounts(execution.inner.data.delta, zero_for_one)?;
    let response = convert_raw_result_to_human(
        receipt.transaction_hash.to_string(),
        actual_input,
        actual_output,
        quoted_input,
        quoted_output,
        input_decimals,
        output_decimals,
    );
    tx.commit().await?;
    Ok(response)
}

async fn decimals(provider: &ProviderFromDb, token: Address) -> Result<u8> {
    let decimals = if token == Address::ZERO {
        18
    } else {
        ERC20::new(token, &provider.alloy_provider)
            .decimals()
            .call()
            .await?
    };
    if decimals > 38 {
        bail!("token decimals above 38 are unsupported");
    }
    Ok(decimals)
}

fn uniswap_v4_delta_to_uint128_amounts(delta: I256, zero_for_one: bool) -> Result<(u128, u128)> {
    let bytes = delta.to_be_bytes::<32>();
    let amount0 = i128::from_be_bytes(bytes[..16].try_into()?);
    let amount1 = i128::from_be_bytes(bytes[16..].try_into()?);
    let (input, output) = if zero_for_one {
        (amount0, amount1)
    } else {
        (amount1, amount0)
    };
    if input >= 0 || output <= 0 {
        bail!("swap must consume input and produce output");
    }
    Ok((input.unsigned_abs(), output as u128))
}

// Exact integer arithmetic; decimal strings truncated to 18 fractional places.
fn ratio_to_human(numerator: U512, denominator: U512) -> String {
    let scale = U512::from(10).pow(U512::from(18));
    let scaled = numerator * scale / denominator;
    let fraction = format!("{:0>18}", (scaled % scale).to_string());
    format!("{}.{}", scaled / scale, fraction)
}

fn convert_raw_result_to_human(
    tx_id: String,
    input: u128,
    output: u128,
    quoted_input: u128,
    quoted_output: u128,
    input_decimals: u8,
    output_decimals: u8,
) -> SwapResponse {
    let input = U512::from(input);
    let output = U512::from(output);
    let quoted = U512::from(quoted_output) * input;
    let actual = output * U512::from(quoted_input);
    let difference = if quoted >= actual {
        quoted - actual
    } else {
        actual - quoted
    };
    let slippage = format!(
        "{}{}",
        if actual > quoted { "-" } else { "" },
        ratio_to_human(difference * U512::from(100), quoted)
    );
    SwapResponse {
        tx_id,
        input_amount: input.to_string(),
        output_amount: output.to_string(),
        price: ratio_to_human(output, input),
        human_price: ratio_to_human(
            output * U512::from(10).pow(U512::from(input_decimals)),
            input * U512::from(10).pow(U512::from(output_decimals)),
        ),
        slippage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_address_strings_and_validates_tokens() {
        let mut params: SwapParams = serde_json::from_str(
            r#"{"chain_id":1,"pool_id":"0x1111111111111111111111111111111111111111111111111111111111111111","input_token":"0x0000000000000000000000000000000000000000","output_token":"0xdAC17F958D2ee523a2206206994597C13D831ec7","input_amount":"100"}"#,
        ).unwrap();
        assert_eq!(params.output_token, "0xdAC17F958D2ee523a2206206994597C13D831ec7");
        let (input, output, amount) = params.parse().unwrap();
        assert_eq!(input, Address::ZERO);
        assert_eq!(output, "0xdac17f958d2ee523a2206206994597c13d831ec7".parse::<Address>().unwrap());
        assert_eq!(amount, U256::from(100));

        params.output_token = "invalid".into();
        assert!(params.parse().unwrap_err().to_string().contains("output_token"));
        params.output_token = "0xdac17f958d2ee523a2206206994597c13d831ec7".into();
        params.input_token = "invalid".into();
        assert!(params.parse().unwrap_err().to_string().contains("input_token"));
        params.input_token = "0xdAC17F958D2ee523a2206206994597C13D831ec7".into();
        assert!(params.parse().is_err(), "same token with different casing");
    }

    #[test]
    fn decodes_both_directions_and_minimum_signed_delta() {
        let mut bytes = [0u8; 32];
        bytes[..16].copy_from_slice(&(-100i128).to_be_bytes());
        bytes[16..].copy_from_slice(&200i128.to_be_bytes());
        assert_eq!(
            uniswap_v4_delta_to_uint128_amounts(I256::from_be_bytes(bytes), true).unwrap(),
            (100, 200)
        );
        assert!(uniswap_v4_delta_to_uint128_amounts(I256::from_be_bytes(bytes), false).is_err());
        bytes[..16].copy_from_slice(&200i128.to_be_bytes());
        bytes[16..].copy_from_slice(&i128::MIN.to_be_bytes());
        assert_eq!(
            uniswap_v4_delta_to_uint128_amounts(I256::from_be_bytes(bytes), false).unwrap(),
            (1u128 << 127, 200)
        );
        assert!(uniswap_v4_delta_to_uint128_amounts(I256::ZERO, true).is_err());
    }

    #[test]
    fn reports_decimal_adjusted_prices_and_signed_quote_slippage() {
        let response = convert_raw_result_to_human(
            "tx".into(),
            1_000_000_000_000_000_000,
            2_970_000_000,
            1_000_000_000_000_000_000,
            3_000_000_000,
            18,
            6,
        );
        assert_eq!(response.price, "0.000000002970000000");
        assert_eq!(response.human_price, "2970.000000000000000000");
        assert_eq!(response.slippage, "1.000000000000000000");
        assert_eq!(
            convert_raw_result_to_human("tx".into(), 100, 202, 100, 200, 6, 6).slippage,
            "-1.000000000000000000"
        );
        // Partial fills compare execution rates, rather than unrelated totals.
        assert_eq!(
            convert_raw_result_to_human("tx".into(), 50, 100, 100, 200, 18, 18).slippage,
            "0.000000000000000000"
        );
    }

    #[test]
    fn rejects_invalid_amounts_before_any_chain_operation() {
        let mut params = SwapParams {
            chain_id: 1,
            pool_id: B256::repeat_byte(1),
            input_token: Address::ZERO.to_string(),
            output_token: Address::repeat_byte(1).to_string(),
            input_amount: "1".into(),
        };
        for amount in [
            "0",
            "-1",
            "0x10",
            "1.5",
            "",
            "170141183460469231731687303715884105728",
        ] {
            params.input_amount = amount.into();
            assert!(params.parse().is_err(), "accepted {amount}");
        }
        params.input_amount = i128::MAX.to_string();
        assert!(params.parse().is_ok());
    }
}
