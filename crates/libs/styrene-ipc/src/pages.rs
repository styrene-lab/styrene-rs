use std::fmt;

const DESTINATION_HASH_HEX_LEN: usize = 32;
const DEFAULT_PAGE_PATH: &str = "/page/index.mu";

/// A canonical Reticulum destination hash used to address a NomadNet host.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NomadNetHost(String);

impl NomadNetHost {
    pub fn parse(input: &str) -> Result<Self, PageAddressError> {
        let input = input.trim();
        if input.len() != DESTINATION_HASH_HEX_LEN
            || !input.as_bytes().iter().all(u8::is_ascii_hexdigit)
        {
            return Err(PageAddressError::InvalidHost);
        }
        Ok(Self(input.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NomadNetHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A native NomadNet request path, preserving page and file semantics.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NomadNetPath {
    Page(String),
    File(String),
}

impl NomadNetPath {
    pub fn parse(input: &str) -> Result<Self, PageAddressError> {
        let normalized = if input == "/" { DEFAULT_PAGE_PATH } else { input };
        validate_path(normalized)?;
        if normalized.starts_with("/page/") {
            Ok(Self::Page(normalized.to_string()))
        } else if normalized.starts_with("/file/") {
            Ok(Self::File(normalized.to_string()))
        } else {
            Err(PageAddressError::InvalidRequestPath)
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Page(path) | Self::File(path) => path,
        }
    }

    pub fn is_page(&self) -> bool {
        matches!(self, Self::Page(_))
    }
}

/// A validated native NomadNet page address.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PageAddress {
    host: Option<NomadNetHost>,
    path: NomadNetPath,
}

impl PageAddress {
    /// Validate the split host/path representation used by the DaemonPages IPC trait.
    pub fn from_request_parts(host: &str, path: &str) -> Result<Self, PageAddressError> {
        let host = host.trim();
        let host = if host.is_empty() { None } else { Some(NomadNetHost::parse(host)?) };
        Self::from_parts(host, NomadNetPath::parse(path.trim())?)
    }

    pub fn parse(input: &str) -> Result<Self, PageAddressError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(PageAddressError::Empty);
        }

        let (host, path) = if let Some((host, path)) = input.split_once(":/") {
            (Some(NomadNetHost::parse(host)?), format!("/{path}"))
        } else if input.starts_with('/') {
            (None, input.to_string())
        } else {
            return Err(PageAddressError::Ambiguous);
        };
        Self::from_parts(host, NomadNetPath::parse(&path)?)
    }

    /// Resolve a link against the current page. `:/...`, `/...`, and bare
    /// paths remain on the current host; a hash-qualified address changes host.
    pub fn resolve(input: &str, current: &Self) -> Result<Self, PageAddressError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(PageAddressError::Empty);
        }
        if input.starts_with(":/") {
            let path = NomadNetPath::parse(&input[1..])?;
            return Self::from_parts(current.host.clone(), path);
        }
        if input.contains(":/") {
            return Self::parse(input);
        }
        if input.starts_with('/') {
            return Self::from_parts(current.host.clone(), NomadNetPath::parse(input)?);
        }
        if input.contains(':') {
            return Err(PageAddressError::Ambiguous);
        }

        let current_path = current.path.as_str();
        let parent = current_path.rsplit_once('/').map_or("/page", |(parent, _)| parent);
        let resolved = normalize_relative_path(&format!("{parent}/{input}"))?;
        Self::from_parts(current.host.clone(), NomadNetPath::parse(&resolved)?)
    }

    pub fn local_index() -> Self {
        Self { host: None, path: NomadNetPath::Page(DEFAULT_PAGE_PATH.to_string()) }
    }

    pub fn remote_index(host: &str) -> Result<Self, PageAddressError> {
        Self::from_parts(
            Some(NomadNetHost::parse(host)?),
            NomadNetPath::Page(DEFAULT_PAGE_PATH.to_string()),
        )
    }

    pub fn host(&self) -> Option<&NomadNetHost> {
        self.host.as_ref()
    }

    pub fn path(&self) -> &str {
        self.path.as_str()
    }

    pub fn parts(&self) -> (&str, &str) {
        (self.host.as_ref().map_or("", NomadNetHost::as_str), self.path())
    }

    fn from_parts(
        host: Option<NomadNetHost>,
        path: NomadNetPath,
    ) -> Result<Self, PageAddressError> {
        if !path.is_page() {
            return Err(PageAddressError::FileIsNotPage);
        }
        Ok(Self { host, path })
    }
}

fn normalize_relative_path(path: &str) -> Result<String, PageAddressError> {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.len() <= 1 {
                    return Err(PageAddressError::InvalidPathSegment);
                }
                segments.pop();
            }
            value => segments.push(value),
        }
    }
    Ok(format!("/{}", segments.join("/")))
}

impl fmt::Display for PageAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(host) = &self.host {
            write!(formatter, "{host}:{}", self.path())
        } else {
            formatter.write_str(self.path())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAddressError {
    Empty,
    Ambiguous,
    InvalidHost,
    InvalidRequestPath,
    InvalidPathSegment,
    FileIsNotPage,
}

impl fmt::Display for PageAddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "page address is empty",
            Self::Ambiguous => "page address must be /page/... or <destination hash>:/page/...",
            Self::InvalidHost => "NomadNet host must be a 16-byte destination hash in hexadecimal",
            Self::InvalidRequestPath => "native request path must start with /page/ or /file/",
            Self::InvalidPathSegment => "native request path contains an invalid path segment",
            Self::FileIsNotPage => "a /file/... target is not a page address",
        })
    }
}

impl std::error::Error for PageAddressError {}

fn validate_path(path: &str) -> Result<(), PageAddressError> {
    if path.contains(['\\', '\0', '?', '#', ':'])
        || path.chars().any(char::is_whitespace)
        || path.split('/').any(|segment| segment == "." || segment == "..")
        || path.ends_with('/')
        || path.contains("//")
    {
        return Err(PageAddressError::InvalidPathSegment);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn validates_and_canonicalizes_native_page_addresses() {
        let remote = PageAddress::parse("0123456789ABCDEF0123456789ABCDEF:/page/docs/start.mu")
            .expect("valid native page address");
        assert_eq!(remote.host().map(NomadNetHost::as_str), Some(HOST));
        assert_eq!(remote.path(), "/page/docs/start.mu");
        assert_eq!(PageAddress::parse("/").expect("local index"), PageAddress::local_index());
    }

    #[test]
    fn rejects_ambiguous_hosts_and_non_native_paths() {
        for input in [
            "docs/start.mu",
            "abcdef:/page/start.mu",
            "not-a-hash:/page/start.mu",
            "/docs/start.mu",
            "/page/../secret.mu",
            "/page/two words.mu",
            "/page/other:/start.mu",
        ] {
            assert!(PageAddress::parse(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn validates_split_ipc_page_request_parts() {
        let remote = PageAddress::from_request_parts(HOST, "/page/index.mu").unwrap();
        assert_eq!(remote.parts(), (HOST, "/page/index.mu"));
        assert_eq!(
            PageAddress::from_request_parts("not-a-hash", "/page/index.mu"),
            Err(PageAddressError::InvalidHost)
        );
        assert_eq!(
            PageAddress::from_request_parts(HOST, "/file/secret"),
            Err(PageAddressError::FileIsNotPage)
        );
        assert!(PageAddress::from_request_parts(HOST, "/page/../secret").is_err());
    }

    #[test]
    fn keeps_file_paths_typed_but_out_of_page_navigation() {
        assert!(matches!(NomadNetPath::parse("/file/manual.pdf"), Ok(NomadNetPath::File(_))));
        assert_eq!(
            PageAddress::parse(&format!("{HOST}:/file/manual.pdf")),
            Err(PageAddressError::FileIsNotPage)
        );
    }

    #[test]
    fn resolves_current_host_and_relative_page_links() {
        let current =
            PageAddress::parse(&format!("{HOST}:/page/docs/start.mu")).expect("current page");
        assert_eq!(
            PageAddress::resolve(":/page/about.mu", &current)
                .expect("host-relative link")
                .to_string(),
            format!("{HOST}:/page/about.mu")
        );
        assert_eq!(
            PageAddress::resolve("next.mu", &current).expect("path-relative link").to_string(),
            format!("{HOST}:/page/docs/next.mu")
        );
        assert_eq!(
            PageAddress::resolve("../about.mu", &current)
                .expect("parent-relative link")
                .to_string(),
            format!("{HOST}:/page/about.mu")
        );
    }
}
