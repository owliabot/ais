use super::{EvmTransactionSigner, LocalPrivateKeySigner, SignerError};

#[test]
fn local_private_key_signer_rejects_invalid_hex() {
    let error = LocalPrivateKeySigner::from_hex("0x1234").expect_err("must reject short key");
    assert!(matches!(error, SignerError::InvalidKey(_)));
}

#[test]
fn local_private_key_signer_exposes_address_and_private_key_hex() {
    let signer = LocalPrivateKeySigner::from_hex(
        "0x1111111111111111111111111111111111111111111111111111111111111111",
    )
    .expect("valid key");

    assert_eq!(EvmTransactionSigner::address(&signer), Some(signer.address()));
    assert_eq!(
        EvmTransactionSigner::private_key_hex(&signer).as_deref(),
        Some("0x1111111111111111111111111111111111111111111111111111111111111111")
    );
}
