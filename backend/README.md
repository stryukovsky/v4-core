# Backend

Rust JSON-RPC API scaffold using jsonrpsee, Tokio, and SQLx/PostgreSQL. Includes an Ethereum configuration table; no application RPC methods are registered yet. Axum also serves a swapper deployment endpoint.

From this directory:

```sh
cp .env.example .env
docker compose up -d --wait
cargo run
```

The HTTP server listens on `127.0.0.1:3000` by default. JSON-RPC is served over HTTP at `/rpc`; Axum endpoints share the same server. Change `API_ADDR`, `DATABASE_URL`, `PRIVATE_KEY`, and `RUST_LOG` in `backend/.env` or the process environment (environment variables take precedence). `PRIVATE_KEY` is required by the existing configuration loader.

Add JSON-RPC methods in `src/rpc.rs`. You can register them on the shared `RpcModule<AppState>` or define a `#[jsonrpsee::proc_macros::rpc(server)]` trait, implement its generated server trait, and merge `into_rpc()`. The `macros` feature is enabled. Methods can access `state.db` and `state.private_key` through the module context.

Axum routes live in `src/routes.rs`. `POST /chains/{chain_id}/swapper` loads exactly one matching database configuration, verifies the RPC chain ID, signs and deploys `MySwapper` with `PRIVATE_KEY`, and waits for deployment before returning HTTP 201 with `chain_id` and `contract_address`. It returns HTTP 400 for nonpositive chain IDs and HTTP 500 with a generic JSON error for configuration or deployment failures. Errors do not expose RPC URLs or private keys. Each successful request creates a new contract; the returned address is not written back to `ethereum_config`. The configured signer needs funds on the requested chain. `GET /hello/{name}` returns a JSON greeting.

Example deployment request (sends an on-chain transaction when called):

```sh
curl -X POST http://127.0.0.1:3000/chains/1/swapper
```

The pool connects and migrations finish before the server starts. Ctrl+C and SIGTERM stop the RPC server and wait for shutdown before closing the pool. You can verify the protocol before adding methods:

```sh
curl http://127.0.0.1:3000/rpc -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"app_unknown","params":[]}'
```

The response contains error code `-32601` (method not found) and the same request ID.

Add migrations as `sql/create/<version>_<description>.sql` (for example `001_create_users.sql`). Pending migrations run on startup or with `make db-migrate`; both use the same SQLx migration history. Applied migrations must not be edited. SQL directories are loaded at runtime from this crate's source path; include them when moving or packaging the application.

Database commands (run from `backend/`, or use `make -C backend <command>` from the repository root):

| Command | Action |
| --- | --- |
| `make db-migrate` | Apply pending `sql/create/` migrations in numeric order. |
| `make db-purify` | Run `sql/purify/` scripts in reverse numeric order to clear data. |
| `make db-load` | Run PostgreSQL fixture scripts in `sql/load/` in numeric order. |
| `make db-reload` | Purify, then load fixtures in one transaction. |
| `make db-clear-structs` | Purify, then revert all applied migrations in reverse order and remove their SQLx history entries. |

Commands connect using `DATABASE_URL` from `backend/.env` or the process environment. Start PostgreSQL first. They do not start the API. Run destructive maintenance commands with the API stopped and check that `DATABASE_URL` targets the intended database.

Set `RPC_URL` in `backend/.env` or the process environment before `db-load` or `db-reload`. The Ethereum fixture inserts one mainnet row (`chain_id = 1`) with the official Uniswap v4 PoolManager address and an empty `my_swapper_contract`. Contract addresses are stored as `BYTEA`. The runner passes `RPC_URL` as a bound, transaction-local PostgreSQL setting; it is not interpolated into SQL or printed. No columns are nullable or unique. Repeated `db-load` calls append rows; use `db-reload` to reset fixtures.

Use matching numbers for related scripts: `create/001_create_members.sql`, `revert/001_members_revert.sql`, `purify/001_purify_members.sql`, and `load/001_load_members.sql`. Revert scripts must undo their paired migration; purify scripts delete data while preserving schema. Fixture scripts contain actual PostgreSQL SQL. Keep foreign-key dependencies consistent with execution order.

Maintenance commands run in a transaction and stop on SQL errors, rolling back the operation. Do not include `BEGIN`/`COMMIT` or statements that cannot run in a transaction in maintenance scripts. `db-clear-structs` checks that every applied migration has a revert script before purifying. After clearing, run `db-migrate` before loading fixtures again.

```sh
cargo check --bin backend --bin db
cargo clippy --bin backend -- -D warnings
docker compose stop
```

Compose uses local development credentials and a persistent volume. `docker compose down` preserves the database; `docker compose down -v` deletes it. If port 5432 is already occupied, change the published port and `DATABASE_URL` together.

The existing chain swap demo is preserved in `src/bin/swap_demo.rs` and can be run explicitly with `cargo run --bin swap_demo`. It requires its original local Ethereum RPC setup and performs chain transactions.
