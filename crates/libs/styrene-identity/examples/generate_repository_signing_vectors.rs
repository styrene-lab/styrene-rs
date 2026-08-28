use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use styrene_identity::{IdentityId, RepositorySignerBinding, derive::KeyDeriver};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = output_path()?;
    let committed_tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    if output.starts_with(committed_tests) {
        return Err("generator refuses to write into the committed tests directory".into());
    }

    let root = [0x42; 32];
    let deriver = KeyDeriver::new(&root);
    let identity_seed = deriver.signing_seed();
    let identity_key = SigningKey::from_bytes(&identity_seed);
    let identity_public_key = identity_key.verifying_key().to_bytes();
    let identity_id = IdentityId::from_public_key(&identity_public_key);
    let mut vectors = Vec::new();
    for epoch in [0, 1, u32::MAX] {
        let repository_seed = deriver.derive_repository_signing_key(epoch);
        let repository_public_key =
            SigningKey::from_bytes(&repository_seed).verifying_key().to_bytes();
        let binding = RepositorySignerBinding::issue(&identity_key, repository_public_key, epoch)?;
        vectors.push(vector(
            format!("derived-epoch-{epoch}"),
            "derived",
            epoch,
            Some(repository_seed),
            repository_public_key,
            &binding,
            root,
            identity_seed,
            identity_public_key,
            identity_id,
        )?);
    }

    let external_seed = [0xa5; 32];
    let external_public_key = SigningKey::from_bytes(&external_seed).verifying_key().to_bytes();
    let external = RepositorySignerBinding::issue(&identity_key, external_public_key, 7)?;
    vectors.push(vector(
        "external-repository-key".into(),
        "externally-supplied",
        7,
        Some(external_seed),
        external_public_key,
        &external,
        root,
        identity_seed,
        identity_public_key,
        identity_id,
    )?);

    let document = json!({
        "profile": "styrene-repository-signing-v1",
        "warning": "All roots and private seeds are public test material and MUST NOT be used for real identities or repositories.",
        "vectors": vectors,
    });
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, format!("{}\n", serde_json::to_string_pretty(&document)?))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn vector(
    name: String,
    repository_key_source: &str,
    epoch: u32,
    repository_seed: Option<[u8; 32]>,
    repository_public_key: [u8; 32],
    binding: &RepositorySignerBinding,
    root: [u8; 32],
    identity_seed: [u8; 32],
    identity_public_key: [u8; 32],
    identity_id: IdentityId,
) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(json!({
        "name": name,
        "repository_key_source": repository_key_source,
        "root_secret_hex": hex::encode(root),
        "identity_seed_hex": hex::encode(identity_seed),
        "identity_public_key_hex": hex::encode(identity_public_key),
        "identity_id_hex": identity_id.to_string(),
        "repository_seed_hex": repository_seed.map(hex::encode),
        "repository_public_key_hex": hex::encode(repository_public_key),
        "epoch": epoch,
        "protected_hex": hex::encode(binding.protected_bytes()),
        "signing_frame_hex": hex::encode(binding.signing_frame()?),
        "signature_hex": hex::encode(binding.signature()),
        "binding_hex": hex::encode(binding.canonical_bytes()?),
        "binding_digest_hex": hex::encode(binding.digest()?),
    }))
}

fn output_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--output")) {
        return Err("usage: generate_repository_signing_vectors --output <candidate-file>".into());
    }
    let output = arguments.next().ok_or("missing candidate output path")?;
    if arguments.next().is_some() {
        return Err("unexpected generator argument".into());
    }
    Ok(PathBuf::from(output))
}
