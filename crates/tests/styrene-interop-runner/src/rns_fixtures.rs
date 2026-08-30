use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
pub struct RnsFixtureIndex {
    pub schema_version: u32,
    pub authorities: BTreeMap<String, RnsAuthority>,
    pub vectors: Vec<RnsVector>,
    #[serde(skip)]
    root: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct RnsAuthority {
    pub repository: String,
    pub revision: String,
    pub release: String,
}

#[derive(Debug, Deserialize)]
pub struct RnsVector {
    pub id: String,
    pub authority_id: String,
    pub kind: String,
    pub artifact: String,
    pub sha256: String,
    pub generator: String,
    pub source_symbols: Vec<String>,
    pub expected: serde_json::Value,
}

fn is_repository_relative(path: &Path) -> bool {
    !path.is_absolute() && path.components().all(|part| matches!(part, Component::Normal(_)))
}

fn repository_file(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let resolved = std::fs::canonicalize(path)
        .map_err(|error| format!("{} cannot be resolved: {error}", path.display()))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(format!("{} is not a file within the repository root", path.display()));
    }
    Ok(resolved)
}

pub fn load_rns_index(root: &Path) -> Result<RnsFixtureIndex, Vec<String>> {
    load_rns_index_from(root, &root.join("tests/interop/fixtures/rns/index-v2.json"))
}

pub fn load_rns_index_from(root: &Path, path: &Path) -> Result<RnsFixtureIndex, Vec<String>> {
    let root = std::fs::canonicalize(root)
        .map_err(|error| vec![format!("failed to resolve {}: {error}", root.display())])?;
    let path = repository_file(&root, path).map_err(|error| vec![error])?;
    let data = std::fs::read_to_string(&path)
        .map_err(|error| vec![format!("failed to read {}: {error}", path.display())])?;
    let mut index: RnsFixtureIndex = serde_json::from_str(&data)
        .map_err(|error| vec![format!("failed to parse {}: {error}", path.display())])?;
    let mut errors = Vec::new();
    if index.schema_version != 2 {
        errors.push(format!("{}: unsupported schema version", path.display()));
    }
    if index.authorities.is_empty() {
        errors.push(format!("{}: authorities must not be empty", path.display()));
    }
    for (id, authority) in &index.authorities {
        if authority.repository.is_empty() || authority.release.is_empty() {
            errors.push(format!("{id}: authority metadata must not be empty"));
        }
        if authority.revision.len() != 40
            || !authority
                .revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            errors.push(format!("{id}: revision must be a lowercase full commit SHA"));
        }
    }
    let mut ids = HashSet::new();
    for vector in &index.vectors {
        if vector.id.is_empty() || !ids.insert(vector.id.as_str()) {
            errors.push(format!("{}: missing or duplicate vector id", vector.id));
        }
        if !index.authorities.contains_key(&vector.authority_id) {
            errors.push(format!("{}: unknown authority {}", vector.id, vector.authority_id));
        }
        if vector.kind.is_empty() {
            errors.push(format!("{}: kind must not be empty", vector.id));
        }
        let artifact = Path::new(&vector.artifact);
        if !is_repository_relative(artifact) {
            errors.push(format!("{}: artifact must be repository-relative", vector.id));
        } else {
            match repository_file(&root, &root.join(artifact))
                .and_then(|path| std::fs::read(path).map_err(|error| error.to_string()))
            {
                Ok(bytes) => {
                    if hex::encode(Sha256::digest(bytes)) != vector.sha256 {
                        errors.push(format!("{}: artifact digest mismatch", vector.id));
                    }
                }
                Err(error) => {
                    errors.push(format!("{}: artifact cannot be read: {error}", vector.id));
                }
            }
        }
        let generator = Path::new(&vector.generator);
        if vector.generator != "manual-copy"
            && (!is_repository_relative(generator)
                || repository_file(&root, &root.join(generator)).is_err())
        {
            errors.push(format!("{}: generator does not exist within the repository", vector.id));
        }
        if vector.source_symbols.is_empty()
            || vector.source_symbols.iter().any(|symbol| symbol.is_empty())
        {
            errors.push(format!("{}: source symbols must not be empty", vector.id));
        }
        if vector
            .expected
            .as_object()
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
            .is_none_or(str::is_empty)
        {
            errors.push(format!("{}: expected outcome must have a type", vector.id));
        }
    }
    if index.vectors.is_empty() {
        errors.push(format!("{}: vectors must not be empty", path.display()));
    }
    if errors.is_empty() {
        index.root = root;
        Ok(index)
    } else {
        Err(errors)
    }
}

pub fn rns_vector<'a>(index: &'a RnsFixtureIndex, id: &str) -> Result<&'a RnsVector, String> {
    index
        .vectors
        .iter()
        .find(|vector| vector.id == id)
        .ok_or_else(|| format!("unknown RNS fixture vector {id}"))
}

pub fn load_rns_vector_bytes(index: &RnsFixtureIndex, id: &str) -> Result<Vec<u8>, String> {
    let vector = rns_vector(index, id)?;
    let path = repository_file(&index.root, &index.root.join(&vector.artifact))
        .map_err(|error| format!("failed to resolve vector {id}: {error}"))?;
    let bytes =
        std::fs::read(path).map_err(|error| format!("failed to read vector {id}: {error}"))?;
    let digest = hex::encode(Sha256::digest(&bytes));
    if digest != vector.sha256 {
        return Err(format!("fixture digest mismatch for {id}"));
    }
    Ok(bytes)
}
