//! Authoritative runtime profile resolution and presentation.

use std::path::{Component, Path, PathBuf};

use crate::StyrenePaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeHost {
    Embedded,
    ExternalService,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeProfile {
    Standard,
    Portable { root: PathBuf },
    Ghost,
    PortableGhost { root: PathBuf },
}

impl RuntimeProfile {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Standard => "STANDARD",
            Self::Portable { .. } => "PORTABLE",
            Self::Ghost => "GHOST",
            Self::PortableGhost { .. } => "PORTABLE GHOST",
        }
    }

    pub fn is_ephemeral(&self) -> bool {
        matches!(self, Self::Ghost | Self::PortableGhost { .. })
    }

    pub fn portable_root(&self) -> Option<&Path> {
        match self {
            Self::Portable { root } | Self::PortableGhost { root } => Some(root),
            Self::Standard | Self::Ghost => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub profile: RuntimeProfile,
    pub host: RuntimeHost,
    pub paths: StyrenePaths,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeOverrides {
    pub ghost: bool,
    pub portable: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeEnvironment {
    pub mode: Option<String>,
    pub home: Option<PathBuf>,
}

impl RuntimeEnvironment {
    pub fn from_process() -> Self {
        Self {
            mode: std::env::var("STYRENE_MODE").ok(),
            home: std::env::var_os("STYRENE_HOME").map(PathBuf::from),
        }
    }
}

impl RuntimeContext {
    pub fn resolve(overrides: RuntimeOverrides) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        Self::resolve_with(overrides, RuntimeEnvironment::from_process(), &executable)
    }

    pub fn resolve_with(
        overrides: RuntimeOverrides,
        environment: RuntimeEnvironment,
        executable: &Path,
    ) -> Result<Self, String> {
        let executable_dir = executable.parent().unwrap_or_else(|| Path::new("."));
        let environment_mode = environment.mode.as_deref().map(str::trim);
        let environment_ghost = matches!(environment_mode, Some("ghost" | "portable-ghost"));
        let marker = executable_dir.join("styrene.portable").is_file();

        let portable_root = if let Some(root) = overrides.portable {
            Some(root)
        } else if let Some(root) = environment.home {
            Some(root)
        } else if matches!(environment_mode, Some("portable" | "portable-ghost")) || marker {
            Some(executable_dir.to_path_buf())
        } else {
            None
        };

        if let Some(mode) = environment_mode
            && !matches!(mode, "standard" | "portable" | "ghost" | "portable-ghost")
        {
            return Err(format!("invalid STYRENE_MODE {mode:?}"));
        }

        let ghost = overrides.ghost || environment_ghost;
        let profile = match (portable_root, ghost) {
            (Some(root), true) => RuntimeProfile::PortableGhost { root: validated_root(root)? },
            (Some(root), false) => RuntimeProfile::Portable { root: validated_root(root)? },
            (None, true) => RuntimeProfile::Ghost,
            (None, false) => RuntimeProfile::Standard,
        };

        let paths = StyrenePaths::for_profile(&profile)?;
        Ok(Self { profile, host: RuntimeHost::Embedded, paths })
    }
}

fn validated_root(root: PathBuf) -> Result<PathBuf, String> {
    if root.as_os_str().is_empty() {
        return Err("portable root cannot be empty".into());
    }
    if root.components().any(|component| component == Component::ParentDir) {
        return Err("portable root cannot contain '..'".into());
    }
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("create portable root {}: {error}", root.display()))?;
    root.canonicalize()
        .map_err(|error| format!("resolve portable root {}: {error}", root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> PathBuf {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("styrene-runtime-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn standard_is_default() {
        let executable = root().join("bin/styrene");
        let context = RuntimeContext::resolve_with(
            RuntimeOverrides::default(),
            RuntimeEnvironment::default(),
            &executable,
        )
        .unwrap();
        assert_eq!(context.profile, RuntimeProfile::Standard);
    }

    #[test]
    fn explicit_portable_and_ghost_compose() {
        let root = root();
        let context = RuntimeContext::resolve_with(
            RuntimeOverrides { ghost: true, portable: Some(root.clone()) },
            RuntimeEnvironment::default(),
            &root.join("styrene"),
        )
        .unwrap();
        assert!(matches!(context.profile, RuntimeProfile::PortableGhost { .. }));
        assert!(context.paths.config_dir.starts_with(root.canonicalize().unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cli_portable_root_wins_over_environment() {
        let cli = root();
        let env = root();
        let context = RuntimeContext::resolve_with(
            RuntimeOverrides { ghost: false, portable: Some(cli.clone()) },
            RuntimeEnvironment { mode: Some("portable".into()), home: Some(env) },
            &cli.join("styrene"),
        )
        .unwrap();
        assert_eq!(context.profile.portable_root(), Some(cli.canonicalize().unwrap().as_path()));
        std::fs::remove_dir_all(cli).unwrap();
    }

    #[test]
    fn parent_escape_is_rejected() {
        let error = RuntimeContext::resolve_with(
            RuntimeOverrides { ghost: false, portable: Some(PathBuf::from("root/../escape")) },
            RuntimeEnvironment::default(),
            Path::new("/bin/styrene"),
        )
        .unwrap_err();
        assert!(error.contains("cannot contain '..'"));
    }
}
