use super::{
    execution_type_capabilities, execution_type_kind, execution_types_for_route_preset,
    is_core_execution_type, is_write_execution_type, ExecutionTypeKind, ExecutionTypeRoutePreset,
    PluginExecutionTypeCapabilities, RuntimeExecutionTypeRegistry,
};

#[test]
fn capabilities_lookup_reports_known_execution_type() {
    let capabilities = execution_type_capabilities("evm_call").expect("must exist");
    assert_eq!(capabilities.kind, ExecutionTypeKind::Core);
    assert!(capabilities.is_write);
    assert!(capabilities.supports_side_effect_adapter);
}

#[test]
fn unknown_execution_type_defaults_to_plugin_kind() {
    assert_eq!(execution_type_kind("sui_tx"), ExecutionTypeKind::Plugin);
    assert!(!is_core_execution_type("sui_tx"));
    assert!(!is_write_execution_type("sui_tx"));
}

#[test]
fn route_presets_return_expected_execution_types() {
    assert_eq!(
        execution_types_for_route_preset(ExecutionTypeRoutePreset::EvmCore),
        &["evm_read", "evm_call"]
    );
    assert_eq!(
        execution_types_for_route_preset(ExecutionTypeRoutePreset::EvmPlugin),
        &["evm_rpc"]
    );
    assert_eq!(
        execution_types_for_route_preset(ExecutionTypeRoutePreset::SolanaCore),
        &["solana_read", "solana_instruction"]
    );
}

#[test]
fn runtime_registry_registers_plugin_capabilities() {
    let mut registry = RuntimeExecutionTypeRegistry::new();
    registry.register_plugin(
        "offchain_apy_query",
        PluginExecutionTypeCapabilities {
            is_write: false,
            requires_confirm_default: false,
            supports_side_effect_adapter: true,
        },
    );
    let capabilities = registry
        .plugin_capabilities("offchain_apy_query")
        .expect("plugin capabilities");
    assert!(capabilities.supports_side_effect_adapter);
    assert_eq!(
        registry.plugin_execution_types(),
        vec!["offchain_apy_query".to_string()]
    );
}
