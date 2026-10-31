CREATE TABLE ethereum_config (
    rpc_url TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    uniswapv4_core_pool_manager BYTEA NOT NULL,
    my_swapper_contract BYTEA NOT NULL DEFAULT '\x'::bytea
);
