use lxmf::message::{Payload, WireMessage};
use lxmf::router::Router;

#[test]
fn e2e_send_receive() {
    let mut router = Router::default();
    let payload = Payload::new(1_700_000_003.0, Some("test".into()), None, None);
    let msg = WireMessage::new([1u8; 16], [2u8; 16], payload);

    router.send_for_test(msg.clone()).expect("send");
    let received = router.recv_for_test().expect("recv");

    assert_eq!(received.payload.to_msgpack().unwrap(), msg.payload.to_msgpack().unwrap());
}
