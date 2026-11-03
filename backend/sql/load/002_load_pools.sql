-- Ethereum mainnet native ETH / USDT: fee 3000 (0.3%), tick spacing 60, no hooks.
-- PoolManager: https://developers.uniswap.org/docs/protocols/v4/deployments
-- Pool ID: keccak256(abi.encode(token0, token1, fee, tickSpacing, hooks)).
-- Deploy a swapper before trading: NULL means not deployed yet.
INSERT INTO pools (protocol, chain_id, pool_id, base_contract, interaction_contract, token0, token1, fee, tick_spacing, hooks)
VALUES ('uni_v4', 1,
    decode('50b00c1a5a8e582ec808e97e71598cd135206a9f9c548eab2ed73659e7ee5fa8', 'hex'),
    decode('000000000004444c5dc75cb358380d2e3de08a90', 'hex'), NULL,
    decode(repeat('00', 20), 'hex'), decode('dac17f958d2ee523a2206206994597c13d831ec7', 'hex'),
    3000, 60, decode(repeat('00', 20), 'hex'))
ON CONFLICT DO NOTHING;
