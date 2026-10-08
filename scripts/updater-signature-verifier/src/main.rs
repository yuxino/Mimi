use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use std::fs::File;
use std::io::Read;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let public_key = args.next().ok_or("missing encoded public key")?;
    let signature_path = args.next().ok_or("missing signature path")?;
    let artifact_path = args.next().ok_or("missing artifact path")?;
    if args.next().is_some() {
        return Err("unexpected verifier argument".into());
    }

    let public_key = decode_text(&public_key)?;
    let signature = decode_text(&std::fs::read_to_string(&signature_path)?)?;
    let public_key = PublicKey::decode(&public_key)?;
    let signature = Signature::decode(&signature)?;
    let mut verifier = public_key.verify_stream(&signature)?;
    let mut artifact = File::open(Path::new(&artifact_path))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = artifact.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier.finalize()?;
    Ok(())
}

fn decode_text(value: &str) -> Result<String, Box<dyn std::error::Error>> {
    let decoded = base64::engine::general_purpose::STANDARD.decode(value.trim())?;
    Ok(String::from_utf8(decoded)?)
}
