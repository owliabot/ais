use alloy::{
    rpc::client::{BuiltInConnectionString, ClientBuilder, RpcClient},
    transports::BoxTransport,
};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

pub(crate) struct AlloyRpcClientPool {
    pub(crate) runtime: tokio::runtime::Runtime,
    clients: Mutex<BTreeMap<String, CachedRpcClient>>,
}

#[derive(Clone)]
struct CachedRpcClient {
    rpc_url: String,
    timeout_ms: u64,
    client: RpcClient<BoxTransport>,
}

impl AlloyRpcClientPool {
    pub(crate) fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime for evm provider pool");
        Self {
            runtime,
            clients: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn client(
        &self,
        chain: &str,
        rpc_url: &str,
        timeout_ms: u64,
    ) -> Result<RpcClient<BoxTransport>, String> {
        if let Some(cached) = self
            .clients
            .lock()
            .expect("evm pool lock")
            .get(chain)
            .cloned()
        {
            if cached.rpc_url == rpc_url && cached.timeout_ms == timeout_ms {
                return Ok(cached.client);
            }
        }
        let connect = rpc_url
            .parse::<BuiltInConnectionString>()
            .map_err(|error| format!("invalid rpc url `{rpc_url}`: {error}"))?;
        let timeout = Duration::from_millis(timeout_ms);
        let connect_result = self.runtime.block_on(async move {
            tokio::time::timeout(timeout, ClientBuilder::default().connect_boxed(connect)).await
        });
        let client = match connect_result {
            Ok(Ok(client)) => client,
            Ok(Err(error)) => {
                return Err(format!("connect rpc `{rpc_url}` failed: {error}"));
            }
            Err(_) => {
                return Err(format!("connect rpc `{rpc_url}` timeout after {timeout_ms}ms"));
            }
        };
        self.clients
            .lock()
            .expect("evm pool lock")
            .insert(
                chain.to_string(),
                CachedRpcClient {
                    rpc_url: rpc_url.to_string(),
                    timeout_ms,
                    client: client.clone(),
                },
            );
        Ok(client)
    }
}
