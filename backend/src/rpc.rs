use jsonrpsee::{RpcModule, types::ErrorObjectOwned};
use tracing::warn;

use crate::{
    AppState,
    eth::trading::{self, DeployParams, SwapParams},
};

fn failure(operation: &'static str) -> ErrorObjectOwned {
    // Provider errors can contain credentials. Never forward their text to clients or logs.
    tracing::error!(operation, "chain operation failed");
    ErrorObjectOwned::owned(-32000, format!("{operation} failed"), None::<()>)
}

pub fn module(state: AppState) -> RpcModule<AppState> {
    let mut module = RpcModule::new(state);
    module
        .register_async_method("deploy_swapper", |params, state, _| async move {
            let params: DeployParams = params.parse()?;
            if params.chain_id <= 0 {
                return Err(ErrorObjectOwned::owned(
                    -32602,
                    "chain_id must be positive",
                    None::<()>,
                ));
            }
            trading::deploy_swapper_contract(&state, params.chain_id)
                .await
                .map_err(|err| { 
                    println!("{}", err.to_string());
                    failure("swapper deployment")})
        })
        .expect("unique RPC method");
    module
        .register_async_method("swap", |params, state, _| async move {
            let params: SwapParams = params.parse()?;
            params
                .parse()
                .map_err(|error| ErrorObjectOwned::owned(-32602, error.to_string(), None::<()>))?;
            trading::execute_swap(&state, params)
                .await
                .map_err(|_| failure("swap"))
        })
        .expect("unique RPC method");
    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_requests_return_jsonrpc_errors_without_database_access() {
        let state = AppState {
            db: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://localhost/unused")
                .unwrap(),
            private_key: String::new(),
        };
        let module = module(state);
        for request in [
            r#"{"jsonrpc":"2.0","id":7,"method":"deploy_swapper","params":{"chain_id":0}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"swap","params":{"chain_id":1,"pool_id":"0x1111111111111111111111111111111111111111111111111111111111111111","input_token":"bad","output_token":"bad","input_amount":"1"}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"swap","params":{"chain_id":1,"pool_id":"0x1111111111111111111111111111111111111111111111111111111111111111","input_token":"0x0000000000000000000000000000000000000000","output_token":"0x0000000000000000000000000000000000000001","input_amount":"0"}}"#,
        ] {
            let (response, _) = module.raw_json_request(request, 1).await.unwrap();
            let response: serde_json::Value = serde_json::from_str(response.get()).unwrap();
            assert_eq!(response["id"], 7);
            assert_eq!(response["error"]["code"], -32602);
            assert!(response.get("result").is_none());
        }
    }
}
