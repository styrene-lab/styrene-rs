use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use minicbor::Encoder;
use serde_json::{json, Value};
use styrene_identity::{derive::KeyDeriver, IdentityId, RepositorySignerBinding};

const PURPOSE: &str = "styrene-repository-signing-v1";
const SIGNING_DOMAIN: &[u8] = b"styrene-repository-signer-binding-v1";

// DANGER: These fixed seeds are public test material. NEVER use them for a real identity or
// repository.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = guarded_output_path(output_path()?)?;
    let vectors = vectors()?;
    let document = json!({
        "profile": "styrene-repository-signing-v1",
        "corpus": "negative-conformance",
        "warning": "DANGER: All private seeds used to generate this corpus are public test material and MUST NEVER be used for real identities or repositories.",
        "vectors": vectors,
    });
    std::fs::write(output, format!("{}\n", serde_json::to_string_pretty(&document)?))?;
    Ok(())
}

fn vectors() -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let deriver = KeyDeriver::new(&[0x42; 32]);
    let identity_key = SigningKey::from_bytes(&deriver.signing_seed());
    let identity_public_key = identity_key.verifying_key().to_bytes();
    let identity_id = IdentityId::from_public_key(&identity_public_key);
    let repository_public_key = SigningKey::from_bytes(&deriver.derive_repository_signing_key(1))
        .verifying_key()
        .to_bytes();
    let canonical_binding =
        RepositorySignerBinding::issue(&identity_key, repository_public_key, 1)?;
    let canonical = canonical_binding.canonical_bytes()?;
    let protected = canonical_binding.protected_bytes();
    let signature = canonical_binding.signature();
    let canonical_claims = Claims {
        version: 1,
        identity_id: *identity_id.as_bytes(),
        identity_public_key,
        repository_public_key,
        epoch: 1,
        purpose: PURPOSE,
        suite: 0,
    };

    let mut vectors = Vec::new();
    let mut add =
        |name: &str, category: &str, protected: Option<&[u8]>, input: Vec<u8>, class: &str| {
            vectors.push(json!({
                "name": name,
                "category": category,
                "protected_hex": protected.map(hex::encode),
                "input_hex": chunk_long_hex(hex::encode(input)),
                "expected_error_class": class,
            }));
        };

    let field_mutations = [
        ("field-version", encode_claims(Claims { version: 2, ..canonical_claims }), "Semantic"),
        (
            "field-identity-id",
            encode_claims(Claims { identity_id: [0; 16], ..canonical_claims }),
            "IdentityMismatch",
        ),
        (
            "field-identity-key",
            encode_claims(Claims {
                identity_public_key: SigningKey::from_bytes(&[0x24; 32]).verifying_key().to_bytes(),
                ..canonical_claims
            }),
            "IdentityMismatch",
        ),
        (
            "field-repository-key",
            encode_claims(Claims {
                repository_public_key: SigningKey::from_bytes(&[0x25; 32])
                    .verifying_key()
                    .to_bytes(),
                ..canonical_claims
            }),
            "Signature",
        ),
        ("field-epoch", encode_claims(Claims { epoch: 2, ..canonical_claims }), "Signature"),
        (
            "field-purpose",
            encode_claims(Claims { purpose: "ordinary-git-signing", ..canonical_claims }),
            "Semantic",
        ),
        ("field-suite", encode_claims(Claims { suite: 1, ..canonical_claims }), "Semantic"),
    ];
    for (name, mutated_protected, class) in field_mutations {
        let input = match name {
            "field-repository-key" | "field-epoch" => encode_outer(&mutated_protected, signature),
            _ => signed_outer(&identity_key, &mutated_protected),
        };
        add(name, "field-mutation", Some(&mutated_protected), input, class);
    }

    add(
        "truncated-outer",
        "malformed",
        Some(protected),
        canonical[..canonical.len() - 1].to_vec(),
        "Format",
    );
    add("wrong-outer-arity", "malformed", None, vec![0xa1, 0x00, 0x40], "Format");
    add("wrong-outer-type", "malformed", None, vec![0x82, 0x40, 0x40], "Format");
    add(
        "trailing-byte",
        "malformed",
        Some(protected),
        [canonical.as_slice(), &[0]].concat(),
        "Format",
    );
    add(
        "short-signature",
        "malformed-length",
        Some(protected),
        encode_outer(protected, &[0; 63]),
        "Format",
    );
    add(
        "long-signature",
        "malformed-length",
        Some(protected),
        encode_outer(protected, &[0; 65]),
        "Format",
    );
    for (name, claims) in [
        ("short-identity-id", encode_claim_parts(&canonical_claims, Some(&[0; 15]), None, None)),
        ("short-identity-key", encode_claim_parts(&canonical_claims, None, Some(&[0; 31]), None)),
        ("short-repository-key", encode_claim_parts(&canonical_claims, None, None, Some(&[0; 31]))),
    ] {
        add(
            name,
            "malformed-length",
            Some(&claims),
            signed_outer(&identity_key, &claims),
            "Format",
        );
    }

    add(
        "unknown-outer-key",
        "map-keys",
        Some(protected),
        outer_with_keys(2, 1, protected, signature),
        "Format",
    );
    add(
        "duplicate-outer-key",
        "map-keys",
        Some(protected),
        outer_with_keys(0, 0, protected, signature),
        "Format",
    );
    add(
        "reordered-outer-keys",
        "map-keys",
        Some(protected),
        outer_with_keys(1, 0, protected, signature),
        "Format",
    );
    let mut non_shortest_outer = vec![0xb8, 0x02];
    non_shortest_outer.extend_from_slice(&canonical[1..]);
    add(
        "non-shortest-outer-map",
        "non-canonical-cbor",
        Some(protected),
        non_shortest_outer,
        "Canonical",
    );
    let mut indefinite_outer = vec![0xbf];
    indefinite_outer.extend_from_slice(&canonical[1..]);
    indefinite_outer.push(0xff);
    add(
        "indefinite-outer-map",
        "non-canonical-cbor",
        Some(protected),
        indefinite_outer,
        "Canonical",
    );
    add(
        "tagged-outer",
        "non-canonical-cbor",
        Some(protected),
        [vec![0xc0], canonical.clone()].concat(),
        "Format",
    );
    add("floating-point-outer", "non-canonical-cbor", None, vec![0xf9, 0, 0], "Format");

    let mut non_shortest_protected = vec![0xb8, 0x07];
    non_shortest_protected.extend_from_slice(&protected[1..]);
    add_signed_protected(
        &mut add,
        &identity_key,
        "non-shortest-protected-map",
        "non-canonical-cbor",
        non_shortest_protected,
        "Canonical",
    );
    let mut indefinite_protected = vec![0xbf];
    indefinite_protected.extend_from_slice(&protected[1..]);
    indefinite_protected.push(0xff);
    add_signed_protected(
        &mut add,
        &identity_key,
        "indefinite-protected-map",
        "non-canonical-cbor",
        indefinite_protected,
        "Canonical",
    );
    for (name, mutated_protected, class) in [
        ("indefinite-byte-string", vec![0x5f, 0xff], "Format"),
        ("truncated-protected", protected[..protected.len() - 1].to_vec(), "Format"),
        ("wrong-protected-arity", vec![0xa6], "Format"),
        ("wrong-protected-type", vec![0x87], "Format"),
        ("tagged-protected", [vec![0xc0], protected.to_vec()].concat(), "Format"),
        ("floating-point-protected", vec![0xf9, 0, 0], "Format"),
    ] {
        add_signed_protected(&mut add, &identity_key, name, "malformed", mutated_protected, class);
    }
    let mut unknown_key = protected.to_vec();
    unknown_key[protected.len() - 2] = 7;
    add_signed_protected(
        &mut add,
        &identity_key,
        "unknown-protected-key",
        "map-keys",
        unknown_key,
        "Format",
    );
    let mut duplicate_key = protected.to_vec();
    duplicate_key[protected.len() - 2] = 5;
    add_signed_protected(
        &mut add,
        &identity_key,
        "duplicate-protected-key",
        "map-keys",
        duplicate_key,
        "Format",
    );
    let reordered = reordered_claims(&canonical_claims);
    add_signed_protected(
        &mut add,
        &identity_key,
        "reordered-protected-keys",
        "map-keys",
        reordered,
        "Format",
    );

    for length in [255, 256, 257] {
        let mut boundary_protected = vec![0; length];
        boundary_protected[0] = 0xa0;
        let class = if length > 256 { "TooLarge" } else { "Format" };
        add(
            &format!("protected-size-{length}"),
            "protected-size-boundary",
            Some(&boundary_protected),
            encode_outer(&boundary_protected, signature),
            class,
        );
    }
    for length in [383, 384, 385] {
        let mut boundary_binding = vec![0; length];
        boundary_binding[0] = 0xa0;
        let class = if length > 384 { "TooLarge" } else { "Format" };
        add(&format!("outer-size-{length}"), "outer-size-boundary", None, boundary_binding, class);
    }

    Ok(vectors)
}

fn chunk_long_hex(hex: String) -> Value {
    if hex.len() <= 600 {
        return Value::String(hex);
    }
    Value::Array(
        hex.as_bytes()
            .chunks(64)
            .map(|chunk| Value::String(String::from_utf8_lossy(chunk).into_owned()))
            .collect(),
    )
}

#[derive(Clone, Copy)]
struct Claims<'a> {
    version: u16,
    identity_id: [u8; 16],
    identity_public_key: [u8; 32],
    repository_public_key: [u8; 32],
    epoch: u32,
    purpose: &'a str,
    suite: u8,
}

fn encode_claims(claims: Claims<'_>) -> Vec<u8> {
    encode_claim_parts(&claims, None, None, None)
}

fn encode_claim_parts(
    claims: &Claims<'_>,
    identity_id: Option<&[u8]>,
    identity_key: Option<&[u8]>,
    repository_key: Option<&[u8]>,
) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .map(7)
        .and_then(|encoder| encoder.u8(0))
        .and_then(|encoder| encoder.u16(claims.version))
        .and_then(|encoder| encoder.u8(1))
        .and_then(|encoder| encoder.bytes(identity_id.unwrap_or(&claims.identity_id)))
        .and_then(|encoder| encoder.u8(2))
        .and_then(|encoder| encoder.bytes(identity_key.unwrap_or(&claims.identity_public_key)))
        .and_then(|encoder| encoder.u8(3))
        .and_then(|encoder| encoder.bytes(repository_key.unwrap_or(&claims.repository_public_key)))
        .and_then(|encoder| encoder.u8(4))
        .and_then(|encoder| encoder.u32(claims.epoch))
        .and_then(|encoder| encoder.u8(5))
        .and_then(|encoder| encoder.str(claims.purpose))
        .and_then(|encoder| encoder.u8(6))
        .and_then(|encoder| encoder.u8(claims.suite))
        .expect("encoding into Vec cannot fail");
    encoder.into_writer()
}

fn reordered_claims(claims: &Claims<'_>) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .map(7)
        .and_then(|encoder| encoder.u8(1))
        .and_then(|encoder| encoder.bytes(&claims.identity_id))
        .and_then(|encoder| encoder.u8(0))
        .and_then(|encoder| encoder.u16(claims.version))
        .and_then(|encoder| encoder.u8(2))
        .and_then(|encoder| encoder.bytes(&claims.identity_public_key))
        .and_then(|encoder| encoder.u8(3))
        .and_then(|encoder| encoder.bytes(&claims.repository_public_key))
        .and_then(|encoder| encoder.u8(4))
        .and_then(|encoder| encoder.u32(claims.epoch))
        .and_then(|encoder| encoder.u8(5))
        .and_then(|encoder| encoder.str(claims.purpose))
        .and_then(|encoder| encoder.u8(6))
        .and_then(|encoder| encoder.u8(claims.suite))
        .expect("encoding into Vec cannot fail");
    encoder.into_writer()
}

fn signing_frame(protected: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(2 + SIGNING_DOMAIN.len() + 2 + 4 + protected.len());
    frame.extend_from_slice(&(SIGNING_DOMAIN.len() as u16).to_be_bytes());
    frame.extend_from_slice(SIGNING_DOMAIN);
    frame.extend_from_slice(&1_u16.to_be_bytes());
    frame.extend_from_slice(&(protected.len() as u32).to_be_bytes());
    frame.extend_from_slice(protected);
    frame
}

fn signed_outer(identity_key: &SigningKey, protected: &[u8]) -> Vec<u8> {
    encode_outer(protected, &identity_key.sign(&signing_frame(protected)).to_bytes())
}

fn encode_outer(protected: &[u8], signature: &[u8]) -> Vec<u8> {
    outer_with_keys(0, 1, protected, signature)
}

fn outer_with_keys(first_key: u8, second_key: u8, protected: &[u8], signature: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .map(2)
        .and_then(|encoder| encoder.u8(first_key))
        .and_then(|encoder| encoder.bytes(protected))
        .and_then(|encoder| encoder.u8(second_key))
        .and_then(|encoder| encoder.bytes(signature))
        .expect("encoding into Vec cannot fail");
    encoder.into_writer()
}

fn add_signed_protected(
    add: &mut impl FnMut(&str, &str, Option<&[u8]>, Vec<u8>, &str),
    identity_key: &SigningKey,
    name: &str,
    category: &str,
    protected: Vec<u8>,
    class: &str,
) {
    let input = signed_outer(identity_key, &protected);
    add(name, category, Some(&protected), input, class);
}

fn output_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--output")) {
        return Err(
            "usage: generate_repository_signing_negative_vectors --output <candidate-file>".into(),
        );
    }
    let output = arguments.next().ok_or("missing candidate output path")?;
    if arguments.next().is_some() {
        return Err("unexpected generator argument".into());
    }
    Ok(PathBuf::from(output))
}

fn guarded_output_path(output: PathBuf) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let absolute =
        if output.is_absolute() { output } else { std::env::current_dir()?.join(output) };
    let file_name = absolute.file_name().ok_or("output path must name a file")?;
    let parent = absolute.parent().ok_or("output path must have a parent directory")?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        format!("output parent must already exist so it can be checked safely: {error}")
    })?;
    let guarded = canonical_parent.join(file_name);
    let committed_tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").canonicalize()?;
    if guarded.starts_with(&committed_tests) {
        return Err("generator refuses to write into the committed tests directory".into());
    }
    if guarded.symlink_metadata().is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("generator refuses to write through a symbolic link".into());
    }
    Ok(guarded)
}
