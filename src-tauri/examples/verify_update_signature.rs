//! Offline verifier for key-preparation probes. Reads public material only.
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use std::{env, fs, process};

fn decoded(path: &str) -> Result<String, &'static str> {
    let encoded = fs::read_to_string(path).map_err(|_| "verification input unavailable")?;
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "invalid base64 input")?;
    String::from_utf8(bytes).map_err(|_| "invalid verification input")
}
fn verify() -> Result<(), &'static str> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 4 || args[3].is_empty() {
        return Err("usage: verify_update_signature FILE SIGNATURE PUBLIC_KEY EXPECTED_VERSION");
    }
    if fs::metadata(&args[0])
        .map_err(|_| "payload unavailable")?
        .len()
        > 128 * 1024 * 1024
    {
        return Err("payload too large");
    }
    let public = PublicKey::decode(&decoded(&args[2])?).map_err(|_| "invalid public key")?;
    let signature = Signature::decode(&decoded(&args[1])?).map_err(|_| "invalid signature")?;
    let payload = fs::read(&args[0]).map_err(|_| "payload unavailable")?;
    public
        .verify(&payload, &signature, true)
        .map_err(|_| "signature rejected")?;
    let versions: Vec<_> = signature
        .trusted_comment()
        .split('\t')
        .filter_map(|v| v.strip_prefix("version:"))
        .collect();
    if versions.len() != 1 || versions[0] != args[3] {
        return Err("signed version rejected");
    }
    Ok(())
}
fn main() {
    if let Err(reason) = verify() {
        eprintln!("{reason}");
        process::exit(1);
    }
    println!("Signature and signed version verified.");
}
