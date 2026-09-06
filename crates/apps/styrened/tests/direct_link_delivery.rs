use rand_core::OsRng;
use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::identity::PrivateIdentity;
use rns_core::packet::{DestinationType, Packet, PacketType};
use rns_core::transport::core_transport::{Transport, TransportConfig};
use rns_core::transport::delivery::{LinkSendResult, send_via_link};
use rns_core::transport::destination_ext::link::{Link, LinkHandleResult};
use rns_core::transport::iface::{Interface, InterfaceContext};
use tokio::time::Duration;

struct SinkInterface {
    observed: tokio::sync::mpsc::UnboundedSender<Packet>,
}

impl Interface for SinkInterface {
    fn mtu() -> usize {
        1500
    }
}

async fn sink_worker(context: InterfaceContext<SinkInterface>) {
    let observed = context.inner.lock().expect("interface state").observed.clone();
    let (_rx_channel, mut tx_channel) = context.channel.split();
    while let Some(queued) = tx_channel.recv().await {
        if observed.send(queued.message.packet).is_err() {
            break;
        }
    }
}

#[tokio::test]
async fn direct_send_uses_link_payloads() {
    let sender = PrivateIdentity::new_from_rand(OsRng);
    let receiver = PrivateIdentity::new_from_rand(OsRng);

    let sender = rns_core::transport::identity_bridge::to_transport_private_identity(&sender);
    let receiver = rns_core::transport::identity_bridge::to_transport_private_identity(&receiver);

    let transport = Transport::new(TransportConfig::new("test", &sender, true));
    let (observed, mut packets) = tokio::sync::mpsc::unbounded_channel();
    let iface =
        transport.iface_manager().lock().await.spawn(SinkInterface { observed }, sink_worker);

    let destination = DestinationDesc {
        identity: *receiver.as_identity(),
        address_hash: *receiver.address_hash(),
        name: DestinationName::new("lxmf", "delivery"),
    };

    let link = transport.link(destination).await;
    let request = link.lock().await.request();

    let (event_tx, _) = tokio::sync::broadcast::channel(16);
    let mut input_link =
        Link::new_from_request(&request, receiver.sign_key().clone(), destination, event_tx)
            .expect("input link");
    let proof = input_link.prove();

    assert!(matches!(link.lock().await.handle_packet(&proof, iface), LinkHandleResult::Activated));

    let result = send_via_link(&transport, destination, b"hello link", Duration::from_secs(1))
        .await
        .expect("send via link");
    let LinkSendResult::Packet(packet) = result else {
        panic!("expected packet delivery for small payload")
    };

    assert_eq!(packet.header.destination_type, DestinationType::Link);
    assert_eq!(packet.header.packet_type, PacketType::Data);
    let emitted = tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(emitted) = packets.recv().await {
            if emitted == *packet {
                return emitted;
            }
        }
        panic!("interface closed before payload emission")
    })
    .await
    .expect("payload reaches the bound interface");
    let mut plaintext = [0u8; 256];
    assert_eq!(
        input_link.decrypt(emitted.data.as_slice(), &mut plaintext).expect("decrypt payload"),
        b"hello link"
    );
    transport.iface_manager().lock().await.stop_interface(&iface);
}
