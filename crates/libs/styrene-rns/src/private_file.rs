//! Private, atomic persistence for raw secret material.
//!
//! Every raw key or private ratchet file in this crate is written through
//! [`write_private_atomic`]: the bytes go to an exclusively created,
//! unpredictably named temporary sibling, are synchronized, and then replace
//! the destination in one rename. A predictable temporary path is never
//! opened, so a planted symlink there redirects nothing. On failure the
//! previous complete file stays in place and the abandoned temporary file is
//! removed. On Unix the parent directory is `0700` and the file `0600`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use rand_core::{OsRng, RngCore};

/// Create `dir` (and its parents) as an owner-private directory.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Tighten group and world access; never widen what the owner set.
        let mode = fs::metadata(dir)?.permissions().mode();
        if mode & 0o077 != 0 {
            fs::set_permissions(dir, fs::Permissions::from_mode(mode & 0o700))?;
        }
    }
    Ok(())
}

/// Write `bytes` to `path` privately and atomically.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    ensure_private_dir(parent)?;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("secret");
    let (temporary_path, mut file) = loop {
        let mut nonce = [0_u8; 8];
        OsRng.fill_bytes(&mut nonce);
        let candidate = parent.join(format!(".{name}.{}.tmp", hex_lower(&nonce)));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = write_and_publish(&mut file, &temporary_path, path, bytes, parent);
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn write_and_publish(
    file: &mut File,
    temporary_path: &Path,
    path: &Path,
    bytes: &[u8],
    parent: &Path,
) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary_path, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        let _ = File::open(parent).and_then(|dir| dir.sync_all());
    }
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let mut nonce = [0_u8; 8];
        OsRng.fill_bytes(&mut nonce);
        let dir = std::env::temp_dir().join(format!("styrene-rns-{name}-{}", hex_lower(&nonce)));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn round_trip_and_replacement() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("secret.key");
        write_private_atomic(&path, b"first").expect("write");
        assert_eq!(fs::read(&path).expect("read"), b"first");
        write_private_atomic(&path, b"second").expect("replace");
        assert_eq!(fs::read(&path).expect("read"), b"second");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .expect("dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temporary files survive success");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn unix_permissions_are_owner_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perms");
        let nested = dir.join("keys");
        let path = nested.join("secret.key");
        write_private_atomic(&path, b"material").expect("write");
        assert_eq!(fs::metadata(&nested).expect("dir").permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&path).expect("file").permissions().mode() & 0o777, 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn predictable_temporary_symlink_is_not_followed() {
        let dir = temp_dir("symlink");
        let path = dir.join("secret.key");
        let victim = dir.join("victim");
        fs::write(&victim, b"untouched").expect("victim");
        // Both legacy predictable temporary names point at the victim.
        std::os::unix::fs::symlink(&victim, path.with_extension("tmp")).expect("link");
        std::os::unix::fs::symlink(&victim, dir.join(".secret.key.tmp")).expect("link");
        write_private_atomic(&path, b"material").expect("write");
        assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
        assert_eq!(fs::read(&path).expect("secret"), b"material");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn failed_replacement_keeps_the_previous_secret_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("failure");
        let path = dir.join("secret.key");
        write_private_atomic(&path, b"previous").expect("write");
        // A read-only parent makes the exclusive temporary create fail.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).expect("lock dir");
        let error = write_private_atomic(&path, b"next").expect_err("write must fail");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("unlock dir");
        assert_eq!(fs::read(&path).expect("previous"), b"previous");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .expect("dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "abandoned temporary material is removed");
        let _ = fs::remove_dir_all(dir);
    }
}
