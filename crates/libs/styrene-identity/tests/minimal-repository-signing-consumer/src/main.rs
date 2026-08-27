use styrene_identity::{verify_repository_signer_binding, RepositorySignerBinding};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let binding = RepositorySignerBinding::issue_derived(&[0x42; 32], 0)?;
    let bytes = binding.canonical_bytes()?;
    let verified = verify_repository_signer_binding(&bytes)?;
    assert_eq!(verified.epoch(), 0);
    Ok(())
}
