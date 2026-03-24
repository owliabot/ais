use ais_agent_control::ids::{ChainSubmissionId, RunId, SignerRequestId};

use crate::signer::{HostSignerResolution, HostSignerResolutionKind};

#[test]
fn host_signer_resolution_converts_to_runtime_resolution() {
    let runtime = HostSignerResolution {
        run_id: RunId("run-1".to_owned()),
        request_id: SignerRequestId("signer-1".to_owned()),
        kind: HostSignerResolutionKind::Signed,
        resolved_at_ms: Some(100),
        submission_id: None,
        signed_payload: Some(
            serde_json::json!({ "kind": "evm_signed_transaction", "raw_tx": "0xabc" }),
        ),
        details: Default::default(),
    }
    .into_runtime_resolution();

    assert_eq!(runtime.request_id.0, "signer-1");
    assert!(runtime.submission_id.is_none());
    assert_eq!(
        runtime.signed_payload,
        Some(serde_json::json!({ "kind": "evm_signed_transaction", "raw_tx": "0xabc" }))
    );
}

#[test]
fn host_signer_resolution_maps_submission_id_into_runtime_submission_id() {
    let runtime = HostSignerResolution {
        run_id: RunId("run-2".to_owned()),
        request_id: SignerRequestId("signer-2".to_owned()),
        kind: HostSignerResolutionKind::Submitted,
        resolved_at_ms: None,
        submission_id: Some(ChainSubmissionId("0xdef".to_owned())),
        signed_payload: None,
        details: Default::default(),
    }
    .into_runtime_resolution();

    assert_eq!(runtime.submission_id.as_deref(), Some("0xdef"));
}
