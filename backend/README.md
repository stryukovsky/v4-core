# Backend

Rust JSON-RPC API using jsonrpsee, Tokio, Alloy, and SQLx/PostgreSQL. Deploys MySwapper, persists its address, and executes exact-input swaps using database pool configurations.

From this directory:

```sh
cp .env.example .env
docker compose up -d --wait
cargo run
```

The HTTP server listens on `127.0.0.1:3000` by default. JSON-RPC is served over HTTP at `/rpc` by jsonrpsee directly. All API operations use JSON-RPC methods. Change `API_ADDR`, `DATABASE_URL`, `PRIVATE_KEY`, and `RUST_LOG` in `backend/.env` or the process environment (environment variables take precedence). `PRIVATE_KEY` is required by the existing configuration loader.

A ready-to-run HTTP client file is available at [`requests/eth_to_usdt.http`](requests/eth_to_usdt.http), including optional swapper deployment and a 0.001 ETH → USDT mainnet swap. Its pool ID and PoolManager are stored together in `sql/load/002_load_pools.sql`.

JSON-RPC methods at `POST /rpc` accept named parameters:

```sh
curl http://127.0.0.1:3000/rpc -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"deploy_swapper","params":{"chain_id":1}}'

curl http://127.0.0.1:3000/rpc -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"swap","params":{"chain_id":1,"pool_id":"0x50b00c1a5a8e582ec808e97e71598cd135206a9f9c548eab2ed73659e7ee5fa8","input_token":"0x0000000000000000000000000000000000000000","output_token":"0xdAC17F958D2ee523a2206206994597C13D831ec7","input_amount":"1000000000000000"}}'
```

These requests sign transactions with `PRIVATE_KEY` and spend that account's funds. Native ETH uses the zero address; amounts are decimal integer strings in token base units. ERC20 input automatically approves the selected swapper for the requested amount, resetting a nonzero insufficient allowance first for tokens such as USDT. The signer receives the output.

`deploy_swapper` verifies the RPC chain ID, waits for deployment, then stores the address in `ethereum_config.my_swapper_contract` and every matching `pools.interaction_contract` (same chain and PoolManager). Its result contains `chain_id` and `contract_address`. Each request deploys a new contract. Load pool fixtures before deploying. Use the `deploy_swapper` JSON-RPC method for deployment.

`swap` requires a 32-byte hex `pool_id` and fetches that stored ID for the requested chain. The input/output tokens must match the selected pool in either direction. Multiple pools for the same token pair are supported by choosing their IDs. Missing or ambiguous pools, missing deployments, mismatched managers, reverted transactions, and configuration failures return JSON-RPC error `-32000`. Invalid request parameters return `-32602`. Provider errors and credentials are never returned. Pools store the canonical `pool_id` (32-byte `BYTEA`), protocol, chain ID, base contract (PoolManager), interaction contract (MySwapper, nullable until deployment), sorted token addresses, fee, tick spacing, and hooks. Hook data is currently empty; hooks requiring custom data are unsupported. Token decimals above 38 are rejected before swapping.

A successful swap result contains:

| Field | Meaning |
| --- | --- |
| `tx_id` | Confirmed swap transaction hash. |
| `input_amount` | Actual consumed input, in base units; may be less than requested at a price boundary. |
| `output_amount` | Actual output from the receipt's `SwapExecuted` event, including hook delta adjustments. |
| `price` | Output base units / input base units. |
| `humanPrice` | Output tokens / input tokens, adjusted for decimals (native ETH: 18). |
| `slippage` | Percentage deterioration of the executed rate relative to a preflight `eth_call` quote; negative means improvement. |

Prices and slippage are decimal strings truncated to 18 fractional places. Slippage is a reported measurement, not a minimum-output guarantee: the existing contract uses the full v4 sqrt-price range. Redeploy existing swappers to obtain the new `SwapExecuted` receipt event. API requests sharing a database serialize signer transactions by chain with PostgreSQL advisory locks. On-chain transactions cannot be rolled back if the process or database fails after submission; reconcile the signer's transaction history before retrying a failed request.

The pool connects and migrations finish before the server starts. Ctrl+C and SIGTERM stop the RPC server and wait for shutdown before closing the pool. You can verify unknown-method handling:

```sh
curl http://127.0.0.1:3000/rpc -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"app_unknown","params":[]}'
```

The response contains error code `-32601` (method not found) and the same request ID.

Add migrations as `sql/create/<version>_<description>.sql` (for example `001_create_users.sql`). Pending migrations run on startup or with `make db-migrate`; both use the same SQLx migration history. Applied migrations must not be edited. SQL directories are loaded at runtime from this crate's source path; include them when moving or packaging the application.

Run these commands from `backend/`, or use `make -C backend <target>` from the repository root. Configure `DATABASE_URL` in `backend/.env` and start PostgreSQL first; `db-load` also needs `RPC_URL` for the mainnet fixture. The existing configuration loader also requires `PRIVATE_KEY`.

| Command | Action |
| --- | --- |
| `make db-migrate` | Apply pending `sql/create/` migrations in numeric order to create or update the schema. Already applied migrations are skipped. |
| `make db-load` | Execute `sql/load/` in numeric order to insert fixtures into the existing schema. |
| `make db-purify` | Execute `sql/purify/` in reverse numeric order to delete data while preserving the schema. |
| `make db-reload` | Purify data and load fixtures in one transaction. |
| `make db-clear-structs` | Purify data, execute applied migrations' paired `sql/revert/` scripts in reverse order, and remove their SQLx history entries. This removes all managed schema. |

Typical workflow:

```sh
make db-migrate
make db-load

# Replace data with fresh fixtures, keeping the schema:
make db-purify
make db-load

# Remove the managed schema and its data:
make db-clear-structs

# Rebuild and load it again:
make db-migrate
make db-load
```

Keep these as separate commands so execution stops on a failure, or join them with `&&`, such as `make db-migrate && make db-load`. Stop the API before purifying or reverting. `db-load` executes fixture SQL every time: the current chain fixture appends rows, so use `db-purify` before loading again to avoid duplicate chain configurations. `make db-reload` performs purify and load in one transaction.

Commands connect using `DATABASE_URL` from `backend/.env` or the process environment. Start PostgreSQL first. They do not start the API. Run destructive maintenance commands with the API stopped and check that `DATABASE_URL` targets the intended database.

Set `RPC_URL` in `backend/.env` or the process environment before `db-load` or `db-reload`. The Ethereum fixture inserts a mainnet configuration (`chain_id = 1`) with the [official Uniswap v4 PoolManager address](https://developers.uniswap.org/docs/protocols/v4/deployments) and an empty `my_swapper_contract`. Migration 002 adds `pools`; migration 003 adds `pool_id` and backfills the known Ethereum fixture. Existing custom pools retain NULL IDs until an operator supplies their canonical IDs from PoolManager Initialize events; they cannot be selected for swaps until populated. New pool inserts must provide the canonical ID; swaps never derive it from a PoolKey. The pool fixture adds the native ETH / USDT PoolKey from the original demo (fee 3000, tick spacing 60, no hooks). The interaction contract starts NULL; deploy a swapper after loading fixtures. Pool initialization and liquidity are on-chain prerequisites; fixtures do not create or fund pools. Contract addresses are stored as `BYTEA`. The runner passes `RPC_URL` as a bound, transaction-local PostgreSQL setting; it is not interpolated into SQL or printed. The original `ethereum_config` table has no uniqueness constraint: repeated `db-load` calls append chain configurations, so use `db-reload` to reset fixtures. The pool fixture is idempotent and preserves existing interaction addresses.

Use matching numbers for related scripts: `create/001_create_members.sql`, `revert/001_members_revert.sql`, `purify/001_purify_members.sql`, and `load/001_load_members.sql`. Revert scripts must undo their paired migration; purify scripts delete data while preserving schema. Fixture scripts contain actual PostgreSQL SQL. Keep foreign-key dependencies consistent with execution order.

Maintenance commands run in a transaction and stop on SQL errors, rolling back the operation. Do not include `BEGIN`/`COMMIT` or statements that cannot run in a transaction in maintenance scripts. `db-clear-structs` checks that every applied migration has a revert script before purifying. After clearing, run `db-migrate` before loading fixtures again.

```sh
cargo test --bin backend
cargo check --bin backend --bin db
cargo clippy --bin backend -- -D warnings
docker compose stop
```

For an isolated end-to-end check, run `python tests/rpc_e2e.py` from `backend/`. It requires Foundry, PostgreSQL binaries, and Python 3; it creates disposable local services, checks both swap directions and deployment persistence, and exercises fixture loading, purify, revert, and migration reapplication.

Compose uses local development credentials and a persistent volume. `docker compose down` preserves the database; `docker compose down -v` deletes it. If port 5432 is already occupied, change the published port and `DATABASE_URL` together.

The original `src/bin/swap_demo.rs` is commented out and has no `main`; use the API for swaps.
