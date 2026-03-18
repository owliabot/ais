use ais_agent_control::ids::{RunId, SignerRequestId};

use crate::signer::{HostSignerResolution, HostSignerResolutionKind};

#[test]
fn host_signer_resolution_converts_to_runtime_resolution() {
    let runtime = HostSignerResolution {
        run_id: RunId("run-1".to_owned()),
        request_id: SignerRequestId("signer-1".to_owned()),
        kind: HostSignerResolutionKind::Signed,
        resolved_at_ms: Some(100),
        tx_hash: None,
        signed_payload: Some(
            serde_json::json!({ "kind": "evm_signed_transaction", "raw_tx": "0xabc" }),
        ),
        details: Default::default(),
    }
    .into_runtime_resolution();

    assert_eq!(runtime.request_id.0, "signer-1");
    assert!(runtime.tx_hash.is_none());
    assert_eq!(
        runtime.signed_payload,
        Some(serde_json::json!({ "kind": "evm_signed_transaction", "raw_tx": "0xabc" }))
    );
}
