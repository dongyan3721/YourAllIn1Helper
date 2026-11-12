//! Standalone artifact QA without loading Tauri/WebView2 or touching an installation.
//! Inputs: MSI, decoded minisign public key, decoded minisign signature, expected SemVer.
use minisign_verify::{PublicKey, Signature};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        5,
        "expected MSI, public key, signature, version"
    );
    let mut bytes = std::fs::read(&args[1]).expect("MSI");
    let public = PublicKey::decode(&std::fs::read_to_string(&args[2]).expect("public key"))
        .expect("decode public key");
    let signature = Signature::decode(&std::fs::read_to_string(&args[3]).expect("signature"))
        .expect("decode signature");
    public
        .verify(&bytes, &signature, true)
        .expect("verify original MSI");
    let comment = signature.trusted_comment();
    let signed_version = comment
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"));
    assert_eq!(
        signed_version,
        Some(args[4].as_str()),
        "signature must bind the expected application version"
    );
    bytes[1024] ^= 1;
    assert!(
        public.verify(&bytes, &signature, true).is_err(),
        "modified MSI must be rejected"
    );
    println!("PASS: MSI signature, signed version, tamper rejection");
}
