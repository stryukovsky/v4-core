-- RPC_URL is passed as a transaction-local setting by the database runner.
-- Ethereum mainnet: https://developers.uniswap.org/docs/protocols/v4/deployments
INSERT INTO ethereum_config (rpc_url, chain_id, uniswapv4_core_pool_manager)
VALUES (
    current_setting('app.rpc_url'),
    1,
    decode('000000000004444c5dc75cb358380d2e3de08a90', 'hex')
);
