use lxmf::message::{Payload, WireMessage};
use lxmf::router::Router;

#[test]
fn router_marks_message_delivered() {
    let mut router = Router::default();
    let payload = Payload::new(1_700_000_002.0, Some("hi".into()), None, None);
    let msg = WireMessage::new([2u8; 16], [3u8; 16], payload);
    let id = msg.message_id();

    router.enqueue_outbound(msg);
    router.handle_receipt_for_test(id);

    assert!(router.is_delivered(&id));
}
