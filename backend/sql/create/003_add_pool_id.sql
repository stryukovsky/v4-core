-- Pool IDs are canonical bytes32 values from PoolManager Initialize events.
-- Existing custom pools remain NULL until their IDs are supplied by the operator.
ALTER TABLE pools ADD COLUMN pool_id BYTEA
    CHECK (octet_length(pool_id) = 32);
ALTER TABLE pools ADD CONSTRAINT pools_chain_base_pool_id_key
    UNIQUE (chain_id, base_contract, pool_id);

-- Backfill the existing Ethereum fixture without recomputing IDs during swaps.
UPDATE pools
SET pool_id = decode('50b00c1a5a8e582ec808e97e71598cd135206a9f9c548eab2ed73659e7ee5fa8', 'hex')
WHERE chain_id = 1
  AND base_contract = decode('000000000004444c5dc75cb358380d2e3de08a90', 'hex')
  AND token0 = decode(repeat('00', 20), 'hex')
  AND token1 = decode('dac17f958d2ee523a2206206994597c13d831ec7', 'hex')
  AND fee = 3000 AND tick_spacing = 60
  AND hooks = decode(repeat('00', 20), 'hex');
