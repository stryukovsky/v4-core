CREATE TABLE pools (
    id BIGSERIAL PRIMARY KEY,
    protocol TEXT NOT NULL CHECK (protocol = 'uni_v4'),
    chain_id BIGINT NOT NULL CHECK (chain_id > 0),
    base_contract BYTEA NOT NULL CHECK (octet_length(base_contract) = 20),
    interaction_contract BYTEA CHECK (octet_length(interaction_contract) = 20),
    token0 BYTEA NOT NULL CHECK (octet_length(token0) = 20),
    token1 BYTEA NOT NULL CHECK (octet_length(token1) = 20),
    fee INTEGER NOT NULL CHECK (fee BETWEEN 0 AND 16777215),
    tick_spacing INTEGER NOT NULL CHECK (tick_spacing BETWEEN 1 AND 32767),
    hooks BYTEA NOT NULL CHECK (octet_length(hooks) = 20),
    CHECK (token0 < token1),
    UNIQUE (chain_id, base_contract, token0, token1, fee, tick_spacing, hooks)
);
