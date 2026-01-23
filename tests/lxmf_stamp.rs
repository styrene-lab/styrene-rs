use lxmf::stamp::Stamp;

#[test]
fn stamp_roundtrip() {
    let data = b"hello";
    let stamp = Stamp::generate(data);
    assert!(stamp.verify(data));
}
