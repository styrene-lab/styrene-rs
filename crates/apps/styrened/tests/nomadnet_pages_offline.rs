use styrened::services::pages::PageService;

#[test]
fn static_nomadnet_pages_are_inventoried_and_served_offline() {
    let root = tempfile::tempdir().expect("temporary page root");
    let pages = root.path().join("pages");
    let files = root.path().join("files");
    std::fs::create_dir_all(&pages).expect("create page directory");
    std::fs::write(pages.join("index.mu"), b">Offline NomadNet\nFixture page")
        .expect("write page fixture");

    let service = PageService::with_storage_dirs(pages, files);
    let entries = service.native_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].request_path, "/page/index.mu");
    assert_eq!(service.handle_request("/page/index.mu"), b">Offline NomadNet\nFixture page");
}
