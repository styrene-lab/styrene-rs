use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use styrene_identity::derive::{KeyDeriver, KeyPurpose};
use styrene_identity::pubkey::{ed25519_verifying_key, x25519_public_key};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = output_path()?;
    let committed_tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    if output.starts_with(committed_tests) {
        return Err("generator refuses to write into the committed tests directory".into());
    }

    let root = [0x42u8; 32];
    let deriver = KeyDeriver::new(&root);
    let mut vectors = serde_json::Map::new();
    vectors.insert("root_secret_hex".into(), hex::encode(root).into());
    vectors.insert("hkdf_salt".into(), "styrene-identity-v1".into());

    let mut flat = serde_json::Map::new();
    for purpose in KeyPurpose::all() {
        let seed = deriver.derive(*purpose);
        let mut entry = serde_json::Map::new();
        entry.insert("info".into(), String::from_utf8_lossy(purpose.info()).to_string().into());
        entry.insert("seed_hex".into(), hex::encode(seed).into());
        match purpose {
            KeyPurpose::Signing
            | KeyPurpose::SshHost
            | KeyPurpose::Yggdrasil
            | KeyPurpose::I2pSigning
            | KeyPurpose::Tor => {
                entry.insert(
                    "pubkey_hex".into(),
                    hex::encode(ed25519_verifying_key(&seed).as_bytes()).into(),
                );
                entry.insert("curve".into(), "ed25519".into());
            }
            KeyPurpose::RnsEncryption
            | KeyPurpose::Age
            | KeyPurpose::I2pEncryption
            | KeyPurpose::WireGuard => {
                entry.insert(
                    "pubkey_hex".into(),
                    hex::encode(x25519_public_key(&seed).as_bytes()).into(),
                );
                entry.insert("curve".into(), "x25519".into());
            }
            _ => {}
        }
        flat.insert(format!("{purpose:?}"), entry.into());
    }
    vectors.insert("flat_purposes".into(), flat.into());

    let identity_key = ed25519_verifying_key(&deriver.signing_seed());
    let identity_digest = Sha256::digest(identity_key.as_bytes());
    vectors.insert("identity_hash".into(), hex::encode(&identity_digest[..16]).into());

    let mut parameterized = serde_json::Map::new();
    for label in ["github", "work", "forge", "wiki", "omegon-primary", "auspex-deploy"] {
        parameterized.insert(
            format!("ssh_user/{label}"),
            hex::encode(deriver.derive_ssh_user_key(label)?).into(),
        );
        parameterized
            .insert(format!("agent/{label}"), hex::encode(deriver.derive_agent_key(label)?).into());
    }
    for service in ["forge", "wiki", "chat"] {
        let (signing, encryption) = deriver.derive_i2p_service(service)?;
        parameterized.insert(format!("i2p/{service}/signing"), hex::encode(signing).into());
        parameterized.insert(format!("i2p/{service}/encryption"), hex::encode(encryption).into());
        parameterized.insert(
            format!("onion/{service}"),
            hex::encode(deriver.derive_onion_service(service)?).into(),
        );
    }
    vectors.insert("parameterized".into(), parameterized.into());
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(vectors))?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, json)?;
    Ok(())
}

fn output_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--output")) {
        return Err("usage: generate_derivation_vectors --output <candidate-file>".into());
    }
    let output = arguments.next().ok_or("missing candidate output path")?;
    if arguments.next().is_some() {
        return Err("unexpected generator argument".into());
    }
    Ok(PathBuf::from(output))
}
