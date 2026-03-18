use std::collections::BTreeMap;

use ais_agent_core::binding::{evm::EvmConnectionSpec, solana::SolanaConnectionSpec};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeExecutionWiring {
    pub providers: RuntimeProviderRegistry,
}

impl RuntimeExecutionWiring {
    pub fn resolve_chain_connection(
        &self,
        chain_scope: &str,
    ) -> Result<Option<RuntimeChainConnectionRef<'_>>, String> {
        let Some(_prefix) = chain_scope_family_prefix(chain_scope) else {
            return Err(format!(
                "unsupported chain scope `{chain_scope}`; expected canonical scope such as `eip155:8453` or `solana:mainnet`"
            ));
        };

        Ok(self
            .providers
            .chains
            .get(&chain_scope.to_ascii_lowercase())
            .map(|entry| match &entry.connection {
                RuntimeChainConnection::Evm(connection) => {
                    RuntimeChainConnectionRef::Evm(connection)
                }
                RuntimeChainConnection::Solana(connection) => {
                    RuntimeChainConnectionRef::Solana(connection)
                }
            }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeProviderRegistry {
    pub chains: BTreeMap<String, RuntimeChainProviderEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeChainProviderEntry {
    pub chain: String,
    pub labels: Vec<String>,
    pub connection: RuntimeChainConnection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeChainConnection {
    Evm(EvmConnectionSpec),
    Solana(SolanaConnectionSpec),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChainConnectionRef<'a> {
    Evm(&'a EvmConnectionSpec),
    Solana(&'a SolanaConnectionSpec),
}

fn chain_scope_family_prefix(value: &str) -> Option<&str> {
    let (prefix, suffix) = value.split_once(':')?;
    if prefix.is_empty() || suffix.trim().is_empty() {
        return None;
    }
    match prefix {
        "eip155" | "solana" => Some(prefix),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_exact_chain_provider() {
        let wiring = RuntimeExecutionWiring {
            providers: RuntimeProviderRegistry {
                chains: BTreeMap::from([(
                    "eip155:8453".to_owned(),
                    RuntimeChainProviderEntry {
                        chain: "eip155:8453".to_owned(),
                        labels: vec!["base".to_owned()],
                        connection: RuntimeChainConnection::Evm(EvmConnectionSpec {
                            http_url: "https://base.example".to_owned(),
                            ws_url: Some("wss://base.example/ws".to_owned()),
                        }),
                    },
                )]),
            },
        };

        let resolved = wiring
            .resolve_chain_connection("eip155:8453")
            .expect("resolve")
            .expect("connection");
        match resolved {
            RuntimeChainConnectionRef::Evm(connection) => {
                assert_eq!(connection.http_url, "https://base.example");
                assert_eq!(connection.ws_url.as_deref(), Some("wss://base.example/ws"));
            }
            other => panic!("unexpected connection: {other:?}"),
        }
    }

    #[test]
    fn returns_none_when_no_provider_exists_for_chain_scope() {
        let wiring = RuntimeExecutionWiring::default();
        assert_eq!(
            wiring
                .resolve_chain_connection("eip155:8453")
                .expect("resolve"),
            None
        );
    }

    #[test]
    fn rejects_non_canonical_chain_scope() {
        let wiring = RuntimeExecutionWiring::default();
        let error = wiring
            .resolve_chain_connection("base")
            .expect_err("invalid chain scope");
        assert!(error.contains("unsupported chain scope"));
    }
}
