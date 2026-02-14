use super::{LocalPrivateKeySigner, SolanaTransactionSigner};
use crate::types::{SolanaInstructionAccount, SolanaInstructionRequest};

#[test]
fn local_private_key_signer_rejects_empty_key() {
    let error = LocalPrivateKeySigner::from_config("").expect_err("must reject empty");
    assert!(error.to_string().contains("invalid key"));
}

#[test]
fn local_private_key_signer_signs_instruction() {
    let signer = LocalPrivateKeySigner::from_config("dev-private-key").expect("valid key");
    let request = SolanaInstructionRequest {
        tx_version: "v0".to_string(),
        program: "JUP6Lkb...".to_string(),
        instruction: "swap".to_string(),
        accounts: vec![SolanaInstructionAccount {
            name: "payer".to_string(),
            pubkey: "abc".to_string(),
            signer: true,
            writable: true,
        }],
        data: "base64:AAA=".to_string(),
        compute_units: Some(200_000),
        lookup_tables: None,
    };
    let signed = signer.sign_instruction(&request).expect("must sign");
    assert!(signed.raw_tx.starts_with("base64:"));
    assert!(signed.tx_hash.starts_with("0x"));
}
