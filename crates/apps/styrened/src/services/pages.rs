//! PageService — NomadNet-compatible page serving over RNS links.
//!
//! Serves `.mu` (Micron markup) files from a configurable pages directory.
//! Registers the `("nomadnetwork", "node")` destination aspect so that
//! NomadNet, MeshChat, and Sideband can browse pages hosted by this node.
//!
//! ## Page Directory Structure
//!
//! ```text
//! ~/.config/styrene/pages/
//!   index.mu          ← default landing page
//!   status.mu         ← node status (could be dynamic)
//!   about.mu          ← static about page
//! ```
//!
//! ## Request Protocol
//!
//! Pages are served via RNS link requests. A client establishes a link to
//! the `("nomadnetwork", "node")` destination and sends a request with
//! path `/page/<filename>`. The server responds with the file content.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Default pages directory.
pub fn default_pages_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config").join("styrene").join("pages")
}

/// A served page entry.
#[derive(Debug, Clone)]
pub struct PageEntry {
    /// Request path (e.g., "/page/index.mu").
    pub request_path: String,
    /// Filesystem path to the .mu file.
    pub file_path: PathBuf,
    /// Whether this is a dynamic (executable) page.
    pub dynamic: bool,
}

/// Service managing NomadNet-compatible page hosting.
pub struct PageService {
    pages_dir: PathBuf,
    pages: Mutex<HashMap<String, PageEntry>>,
    node_name: Mutex<String>,
}

impl PageService {
    pub fn new(pages_dir: PathBuf) -> Self {
        let svc = Self {
            pages_dir,
            pages: Mutex::new(HashMap::new()),
            node_name: Mutex::new("Styrene Node".to_string()),
        };
        svc.scan_pages();
        svc
    }

    pub fn with_default_dir() -> Self {
        Self::new(default_pages_dir())
    }

    pub fn set_node_name(&self, name: &str) {
        *self.node_name.lock().unwrap() = name.to_string();
    }

    /// Scan the pages directory and register all .mu files.
    pub fn scan_pages(&self) {
        let mut pages = self.pages.lock().unwrap();
        pages.clear();

        if !self.pages_dir.exists() {
            return;
        }

        self.scan_dir(&self.pages_dir, &self.pages_dir, &mut pages);

        eprintln!("[pages] scanned {} pages from {}", pages.len(), self.pages_dir.display());
    }

    fn scan_dir(&self, dir: &Path, root: &Path, pages: &mut HashMap<String, PageEntry>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_dir(&path, root, pages);
            } else if path.extension().is_some_and(|ext| ext == "mu") {
                let relative =
                    path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_string();
                let request_path = format!("/page/{relative}");

                #[cfg(unix)]
                let dynamic = {
                    use std::os::unix::fs::PermissionsExt;
                    path.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
                };
                #[cfg(not(unix))]
                let dynamic = false;

                pages.insert(
                    request_path.clone(),
                    PageEntry { request_path, file_path: path, dynamic },
                );
            }
        }
    }

    /// List all registered pages.
    pub fn list_pages(&self) -> Vec<PageEntry> {
        self.pages.lock().unwrap().values().cloned().collect()
    }

    /// Serve a page request. Returns the page content bytes, or None if not found.
    pub fn serve_page(&self, request_path: &str) -> Option<Vec<u8>> {
        let pages = self.pages.lock().unwrap();
        let entry = pages.get(request_path)?;

        if entry.dynamic {
            self.serve_dynamic(&entry.file_path)
        } else {
            self.serve_static(&entry.file_path)
        }
    }

    fn serve_static(&self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    #[cfg(unix)]
    fn serve_dynamic(&self, path: &Path) -> Option<Vec<u8>> {
        use std::process::Command;
        let output = Command::new(path)
            .env("NODE_NAME", self.node_name.lock().unwrap().as_str())
            .output()
            .ok()?;

        if output.status.success() {
            Some(output.stdout)
        } else {
            eprintln!(
                "[pages] dynamic page {} failed: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            None
        }
    }

    #[cfg(not(unix))]
    fn serve_dynamic(&self, _path: &Path) -> Option<Vec<u8>> {
        None // Dynamic pages only on Unix
    }

    /// Serve the default index page if no index.mu exists.
    pub fn serve_default_index(&self) -> Vec<u8> {
        let name = self.node_name.lock().unwrap().clone();
        let page_list: Vec<String> =
            self.pages.lock().unwrap().keys().map(|p| format!("`F444`[{p}]`{p}`f")).collect();

        let pages_section = if page_list.is_empty() {
            "No pages available.".to_string()
        } else {
            page_list.join("\n")
        };

        format!(
            ">Welcome to {name}\n\n\
             This node is running Styrene.\n\n\
             >Pages\n\n\
             {pages_section}\n"
        )
        .into_bytes()
    }

    /// Handle a page request by path, with fallback to default index.
    pub fn handle_request(&self, path: &str) -> Vec<u8> {
        // Normalize path
        let path = if path.is_empty() || path == "/" || path == "/page/" {
            "/page/index.mu"
        } else {
            path
        };

        if let Some(content) = self.serve_page(path) {
            content
        } else if path == "/page/index.mu" {
            self.serve_default_index()
        } else {
            format!("`F900`Page not found: {path}`f\n").into_bytes()
        }
    }

    /// Number of registered pages.
    pub fn page_count(&self) -> usize {
        self.pages.lock().unwrap().len()
    }

    /// Pages directory path.
    pub fn pages_dir(&self) -> &Path {
        &self.pages_dir
    }
}

impl Default for PageService {
    fn default() -> Self {
        Self::with_default_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dir_serves_default_index() {
        let dir = tempfile::tempdir().unwrap();
        let svc = PageService::new(dir.path().to_path_buf());
        assert_eq!(svc.page_count(), 0);

        let content = svc.handle_request("/page/index.mu");
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("Welcome to"));
        assert!(text.contains("Styrene"));
    }

    #[test]
    fn serves_static_page() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.mu"), b">Test Page\nHello!").unwrap();

        let svc = PageService::new(dir.path().to_path_buf());
        assert_eq!(svc.page_count(), 1);

        let content = svc.handle_request("/page/test.mu");
        assert_eq!(content, b">Test Page\nHello!");
    }

    #[test]
    fn not_found_returns_error_page() {
        let dir = tempfile::tempdir().unwrap();
        let svc = PageService::new(dir.path().to_path_buf());

        let content = svc.handle_request("/page/nonexistent.mu");
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("not found"));
    }

    #[test]
    fn nested_pages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/deep.mu"), b">Deep Page").unwrap();

        let svc = PageService::new(dir.path().to_path_buf());
        assert_eq!(svc.page_count(), 1);

        let content = svc.handle_request("/page/sub/deep.mu");
        assert_eq!(content, b">Deep Page");
    }

    #[test]
    fn list_pages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.mu"), b"A").unwrap();
        std::fs::write(dir.path().join("b.mu"), b"B").unwrap();

        let svc = PageService::new(dir.path().to_path_buf());
        let pages = svc.list_pages();
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn custom_node_name_in_default_index() {
        let dir = tempfile::tempdir().unwrap();
        let svc = PageService::new(dir.path().to_path_buf());
        svc.set_node_name("My Hub");

        let content = svc.handle_request("/page/index.mu");
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("My Hub"));
    }
}
