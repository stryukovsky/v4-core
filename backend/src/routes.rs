use alloy::sol;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Serialize;

use crate::{AppState, eth::ProviderFromDb};

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    MySwapper,
    "abi/MySwapper.json"
);

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/hello/{name}", get(hello))
        .route("/chains/{chain_id}/swapper", post(deploy_my_swapper))
        .with_state(state)
}

#[derive(Serialize)]
struct HelloResponse {
    name: String,
}

async fn hello(Path(name): Path<String>) -> Json<HelloResponse> {
    Json(HelloResponse { name })
}

#[derive(Serialize)]
struct DeploymentResponse {
    chain_id: i64,
    contract_address: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: &'static str,
}

async fn deploy_my_swapper(
    State(state): State<AppState>,
    Path(chain_id): Path<i64>,
) -> Result<(StatusCode, Json<DeploymentResponse>), (StatusCode, Json<ErrorResponse>)> {
    if chain_id <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "chain_id must be positive",
            }),
        ));
    }
    let deployment = async {
        let provider = ProviderFromDb::new(&state.db, chain_id, &state.private_key).await?;
        let contract = MySwapper::deploy(&provider.alloy_provider, provider.pool_manager).await?;
        Ok::<_, eyre::Report>(DeploymentResponse {
            chain_id,
            contract_address: contract.address().to_string(),
        })
    }
    .await;
    deployment
        .map(|response| (StatusCode::CREATED, Json(response)))
        .map_err(|_| {
            // Provider errors may contain RPC credentials; do not expose them.
            tracing::error!(chain_id, "swapper deployment failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "swapper deployment failed",
                }),
            )
        })
}
