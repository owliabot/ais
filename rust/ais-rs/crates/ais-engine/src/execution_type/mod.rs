use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionTypeKind {
    Core,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionTypeRoutePreset {
    EvmCore,
    EvmPlugin,
    SolanaCore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionTypeCapabilities {
    pub execution_type: &'static str,
    pub kind: ExecutionTypeKind,
    pub is_write: bool,
    pub requires_confirm_default: bool,
    pub supports_side_effect_adapter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PluginExecutionTypeCapabilities {
    pub is_write: bool,
    pub requires_confirm_default: bool,
    pub supports_side_effect_adapter: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeExecutionTypeRegistry {
    plugin_capabilities: BTreeMap<String, PluginExecutionTypeCapabilities>,
}

impl RuntimeExecutionTypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_plugin(
        &mut self,
        execution_type: impl Into<String>,
        capabilities: PluginExecutionTypeCapabilities,
    ) {
        let execution_type = execution_type.into().trim().to_string();
        if execution_type.is_empty() {
            return;
        }
        self.plugin_capabilities
            .insert(execution_type, capabilities);
    }

    pub fn plugin_capabilities(
        &self,
        execution_type: &str,
    ) -> Option<PluginExecutionTypeCapabilities> {
        self.plugin_capabilities.get(execution_type).copied()
    }

    pub fn plugin_execution_types(&self) -> Vec<String> {
        self.plugin_capabilities.keys().cloned().collect()
    }
}

const EXECUTION_TYPE_CAPABILITIES: &[ExecutionTypeCapabilities] = &[
    ExecutionTypeCapabilities {
        execution_type: "evm_read",
        kind: ExecutionTypeKind::Core,
        is_write: false,
        requires_confirm_default: false,
        supports_side_effect_adapter: false,
    },
    ExecutionTypeCapabilities {
        execution_type: "evm_call",
        kind: ExecutionTypeKind::Core,
        is_write: true,
        requires_confirm_default: false,
        supports_side_effect_adapter: true,
    },
    ExecutionTypeCapabilities {
        execution_type: "evm_multicall",
        kind: ExecutionTypeKind::Plugin,
        is_write: true,
        requires_confirm_default: false,
        supports_side_effect_adapter: false,
    },
    ExecutionTypeCapabilities {
        execution_type: "evm_rpc",
        kind: ExecutionTypeKind::Plugin,
        is_write: false,
        requires_confirm_default: false,
        supports_side_effect_adapter: false,
    },
    ExecutionTypeCapabilities {
        execution_type: "solana_read",
        kind: ExecutionTypeKind::Core,
        is_write: false,
        requires_confirm_default: false,
        supports_side_effect_adapter: false,
    },
    ExecutionTypeCapabilities {
        execution_type: "solana_instruction",
        kind: ExecutionTypeKind::Core,
        is_write: true,
        requires_confirm_default: false,
        supports_side_effect_adapter: true,
    },
    ExecutionTypeCapabilities {
        execution_type: "bitcoin_psbt",
        kind: ExecutionTypeKind::Plugin,
        is_write: true,
        requires_confirm_default: false,
        supports_side_effect_adapter: false,
    },
];

const EVM_CORE_EXECUTION_TYPES: &[&str] = &["evm_read", "evm_call"];
const EVM_PLUGIN_EXECUTION_TYPES: &[&str] = &["evm_rpc"];
const SOLANA_CORE_EXECUTION_TYPES: &[&str] = &["solana_read", "solana_instruction"];

pub fn execution_type_capabilities(execution_type: &str) -> Option<ExecutionTypeCapabilities> {
    EXECUTION_TYPE_CAPABILITIES
        .iter()
        .copied()
        .find(|item| item.execution_type == execution_type)
}

pub fn execution_type_kind(execution_type: &str) -> ExecutionTypeKind {
    execution_type_capabilities(execution_type)
        .map(|item| item.kind)
        .unwrap_or(ExecutionTypeKind::Plugin)
}

pub fn is_core_execution_type(execution_type: &str) -> bool {
    execution_type_kind(execution_type) == ExecutionTypeKind::Core
}

pub fn is_write_execution_type(execution_type: &str) -> bool {
    execution_type_capabilities(execution_type)
        .map(|item| item.is_write)
        .unwrap_or(false)
}

pub fn execution_types_for_route_preset(
    preset: ExecutionTypeRoutePreset,
) -> &'static [&'static str] {
    match preset {
        ExecutionTypeRoutePreset::EvmCore => EVM_CORE_EXECUTION_TYPES,
        ExecutionTypeRoutePreset::EvmPlugin => EVM_PLUGIN_EXECUTION_TYPES,
        ExecutionTypeRoutePreset::SolanaCore => SOLANA_CORE_EXECUTION_TYPES,
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
