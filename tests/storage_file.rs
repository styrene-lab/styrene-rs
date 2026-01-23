use lxmf::message::{Payload, WireMessage};
use lxmf::storage::{FileStore, Store};

#[test]
fn file_store_can_save_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(dir.path());
    let msg = WireMessage::new(
        [1u8; 16],
        [2u8; 16],
        Payload::new(1.0, Some("hi".into()), None, None),
    );
    store.save(&msg).unwrap();
    let loaded = store.get(&msg.message_id()).unwrap();
    assert_eq!(loaded.source, msg.source);
}
