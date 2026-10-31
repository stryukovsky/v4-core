use jsonrpsee::RpcModule;

use crate::AppState;

pub fn module(state: AppState) -> RpcModule<AppState> {
    // Register methods here, or merge a #[rpc(server)] trait's into_rpc().
    RpcModule::new(state)
}
