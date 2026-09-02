//! Routed channel probe: two Rust nodes reach each other only through a
//! transport hop, open a link and a reliable channel across it, and exchange
//! echoed messages both ways.
//!
//! The pinned Python applications speak no channel protocol the daemon
//! exposes, so routed channel evidence is Rust to Rust across a pinned Python
//! transport instance. The harness starts that hop and runs this probe, which
//! writes a JSON proof the interoperability runner validates.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rns_core::destination::{DestinationDesc, DestinationName};
use rns_core::hash::AddressHash;
use rns_core::transport::channel::MessageState;
use rns_core::transport::core_transport::LinkDispatch;
use rns_core::transport::destination_ext::link::{LinkEvent, LinkStatus};
use styrene_e2e::node::{TestNode, TestNodeBuilder};
use tokio_util::sync::CancellationToken;

const PROBE_MESSAGE: u16 = 0x0201;
const ECHO_MESSAGE: u16 = 0x0202;
const POLL: Duration = Duration::from_millis(100);
/// Announces per destination stay far below the pinned transport's hourly
/// rate target; the grace window admits a handful, so re-announce slowly.
const ANNOUNCE_RETRY: Duration = Duration::from_secs(10);

struct Args {
    hop: SocketAddr,
    messages: usize,
    payload_bytes: usize,
    timeout: Duration,
    proof: String,
    correlation_id: String,
}

fn parse_args() -> Result<Args, String> {
    let mut hop = None;
    let mut messages = 6usize;
    let mut payload_bytes = 512usize;
    let mut timeout = Duration::from_secs(60);
    let mut proof = None;
    let mut correlation_id = "routed-channel".to_string();
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let value = iter.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--hop" => hop = Some(value.parse().map_err(|e| format!("--hop: {e}"))?),
            "--messages" => messages = value.parse().map_err(|e| format!("--messages: {e}"))?,
            "--payload" => payload_bytes = value.parse().map_err(|e| format!("--payload: {e}"))?,
            "--timeout" => {
                timeout = Duration::from_secs(value.parse().map_err(|e| format!("--timeout: {e}"))?)
            }
            "--proof" => proof = Some(value),
            "--correlation" => correlation_id = value,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if !(1..=64).contains(&messages) {
        return Err("--messages must be within 1..=64".into());
    }
    Ok(Args {
        hop: hop.ok_or("--hop host:port is required")?,
        messages,
        payload_bytes,
        timeout,
        proof: proof.ok_or("--proof PATH is required")?,
        correlation_id,
    })
}

fn hex(hash: &AddressHash) -> String {
    ::hex::encode(hash.as_slice())
}

fn probe_payload(index: usize, size: usize) -> Vec<u8> {
    (0..size).map(|offset| ((index * 31 + offset * 7) % 251) as u8).collect()
}

/// The path table entry for a destination: hops, the transport identity the
/// announce was received from (the next hop), and the interface.
async fn route_entry(
    node: &TestNode,
    destination: &AddressHash,
) -> Option<(u8, AddressHash, AddressHash)> {
    node.transport
        .path_table_entries()
        .await
        .into_iter()
        .find(|(entry, _, _, _)| entry == destination)
        .map(|(_, hops, received_from, iface)| (hops, received_from, iface))
}

async fn wait_for_routes(a: &TestNode, b: &TestNode, deadline: Instant) -> Result<(), String> {
    let mut next_announce = Instant::now();
    loop {
        if Instant::now() >= next_announce {
            a.announce().await;
            b.announce().await;
            next_announce = Instant::now() + ANNOUNCE_RETRY;
        }
        let a_to_b = a.transport.path_info(&b.delivery_addr).await;
        let b_to_a = b.transport.path_info(&a.delivery_addr).await;
        let a_knows_b = a.transport.destination_identity(&b.delivery_addr).await.is_some();
        let b_knows_a = b.transport.destination_identity(&a.delivery_addr).await.is_some();
        if a_to_b.is_some() && b_to_a.is_some() && a_knows_b && b_knows_a {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "routes did not converge: a_to_b={a_to_b:?} b_to_a={b_to_a:?} a_knows_b={a_knows_b} b_knows_a={b_knows_a}"
            ));
        }
        tokio::time::sleep(POLL).await;
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("styrene-channel-probe: {error}");
            std::process::exit(2);
        }
    };
    let started = Instant::now();
    let mut proof = serde_json::json!({
        "scenario": "routed_channel",
        "correlation_id": args.correlation_id,
        "hop": args.hop.to_string(),
        "status": "failed",
    });
    let outcome = run(&args, &mut proof).await;
    if let Err(error) = &outcome {
        proof["failure"] = serde_json::Value::String(error.clone());
        eprintln!("styrene-channel-probe: {error}");
    } else {
        proof["status"] = serde_json::Value::String("passed".into());
    }
    proof["elapsed_ms"] = serde_json::json!(started.elapsed().as_millis() as u64);
    let encoded = serde_json::to_string_pretty(&proof).expect("serialize proof");
    if let Err(error) = std::fs::write(&args.proof, format!("{encoded}\n")) {
        eprintln!("styrene-channel-probe: write proof {}: {error}", args.proof);
        std::process::exit(1);
    }
    std::process::exit(if outcome.is_ok() { 0 } else { 1 });
}

async fn run(args: &Args, proof: &mut serde_json::Value) -> Result<(), String> {
    let deadline = Instant::now() + args.timeout;
    let a = TestNodeBuilder::new("channel-probe-a").tcp_client(args.hop).build().await;
    let b = TestNodeBuilder::new("channel-probe-b").tcp_client(args.hop).build().await;
    proof["nodes"] = serde_json::json!({
        "a": {"identity": a.identity_hash, "delivery": a.delivery_hash},
        "b": {"identity": b.identity_hash, "delivery": b.delivery_hash},
    });
    println!("probe: nodes up");

    let result = exchange(args, &a, &b, deadline, proof).await;
    a.shutdown().await;
    b.shutdown().await;
    result
}

async fn exchange(
    args: &Args,
    a: &TestNode,
    b: &TestNode,
    deadline: Instant,
    proof: &mut serde_json::Value,
) -> Result<(), String> {
    wait_for_routes(a, b, deadline).await?;
    let (a_hops, a_next, a_iface) = route_entry(a, &b.delivery_addr).await.ok_or("a route")?;
    let (b_hops, b_next, b_iface) = route_entry(b, &a.delivery_addr).await.ok_or("b route")?;
    proof["route"] = serde_json::json!({
        "a_to_b": {"hops": a_hops, "next_hop": hex(&a_next), "interface": hex(&a_iface)},
        "b_to_a": {"hops": b_hops, "next_hop": hex(&b_next), "interface": hex(&b_iface)},
    });
    println!("probe: routes a_to_b={a_hops} hops b_to_a={b_hops} hops");
    if a_hops != 2 || b_hops != 2 {
        return Err(format!("expected two hops each way, got {a_hops} and {b_hops}"));
    }
    if a_next != b_next {
        return Err("the two nodes disagree on the transport hop identity".into());
    }

    // B must observe the inbound link before A's channel traffic arrives.
    let mut inbound_links = b.transport.in_link_events();

    let identity = a.transport.destination_identity(&b.delivery_addr).await.ok_or("b identity")?;
    let dispatch = a
        .transport
        .link_cancellable(
            DestinationDesc {
                identity,
                address_hash: b.delivery_addr,
                name: DestinationName::new("lxmf", "delivery"),
            },
            CancellationToken::new(),
        )
        .await
        .ok_or("link dispatch was not accepted")?;
    let link = match dispatch {
        LinkDispatch::Created(link) | LinkDispatch::Reused(link) => link,
    };
    let link_id = loop {
        let guard = link.lock().await;
        match guard.status() {
            LinkStatus::Active => break *guard.id(),
            LinkStatus::Closed => return Err("link closed before activation".into()),
            _ => {}
        }
        drop(guard);
        if Instant::now() >= deadline {
            return Err("link did not activate before the deadline".into());
        }
        tokio::time::sleep(POLL).await;
    };
    println!("probe: link active {}", hex(&link_id));

    // Wait until B has the same link active on its delivery destination.
    let b_activation = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
        loop {
            match inbound_links.recv().await {
                Ok(event) if event.id == link_id && matches!(event.event, LinkEvent::Activated) => {
                    break Ok::<Option<Duration>, String>(event.rtt);
                }
                Ok(_) => {}
                Err(error) => break Err(format!("inbound link events: {error}")),
            }
        }
    })
    .await
    .map_err(|_| "B never saw the inbound link activate".to_string())??;
    let rtt_ms = b_activation.map(|rtt| rtt.as_secs_f64() * 1000.0);
    proof["link"] = serde_json::json!({"id": hex(&link_id), "rtt_ms": rtt_ms});

    let a_channel = a.transport.channel(link_id);
    let b_channel = b.transport.channel(link_id);
    a_channel.open().await.map_err(|e| format!("open channel on A: {e:?}"))?;
    b_channel.open().await.map_err(|e| format!("open channel on B: {e:?}"))?;
    let mdu = a_channel.mdu().await.map_err(|e| format!("channel mdu: {e:?}"))?;
    let payload_bytes = args.payload_bytes.min(mdu);

    // B echoes every probe payload back on the same channel.
    let received_by_b: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let (echo_tx, mut echo_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    {
        let received = Arc::clone(&received_by_b);
        b_channel
            .register_handler(PROBE_MESSAGE, move |envelope| {
                received
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(envelope.payload.clone());
                let _ = echo_tx.send(envelope.payload);
                true
            })
            .await
            .map_err(|e| format!("register B handler: {e:?}"))?;
    }
    let echo_channel = b.transport.channel(link_id);
    let echo_sequences: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(Vec::new()));
    let echo_task = {
        let echo_sequences = Arc::clone(&echo_sequences);
        tokio::spawn(async move {
            while let Some(payload) = echo_rx.recv().await {
                // Respect B's send window: wait for room before each echo.
                let window_deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    match echo_channel.is_ready_to_send().await {
                        Ok(true) => break,
                        Ok(false) if Instant::now() < window_deadline => {
                            tokio::time::sleep(POLL).await;
                        }
                        Ok(false) => {
                            eprintln!("probe: echo window never opened");
                            return;
                        }
                        Err(error) => {
                            eprintln!("probe: echo channel unavailable: {error:?}");
                            return;
                        }
                    }
                }
                match echo_channel.send(ECHO_MESSAGE, payload).await {
                    Ok(sequence) => echo_sequences
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(sequence),
                    Err(error) => eprintln!("probe: echo send failed: {error:?}"),
                }
            }
        })
    };
    let echoes: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let echoes = Arc::clone(&echoes);
        a_channel
            .register_handler(ECHO_MESSAGE, move |envelope| {
                echoes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(envelope.payload);
                true
            })
            .await
            .map_err(|e| format!("register A handler: {e:?}"))?;
    }

    // Send with the channel's window: wait for readiness before each message.
    let mut sent: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    let send_started = Instant::now();
    for index in 0..args.messages {
        let payload = probe_payload(index, payload_bytes);
        loop {
            if a_channel.is_ready_to_send().await.map_err(|e| format!("{e:?}"))? {
                break;
            }
            if Instant::now() >= deadline {
                return Err("channel window never opened".into());
            }
            tokio::time::sleep(POLL).await;
        }
        let sequence = a_channel
            .send(PROBE_MESSAGE, payload.clone())
            .await
            .map_err(|e| format!("send message {index}: {e:?}"))?;
        sent.insert(sequence, payload);
    }
    println!("probe: sent {} messages", sent.len());

    // Every send must be proved by B, and every payload must come back.
    let delivered = loop {
        let mut delivered = 0usize;
        let mut failed = Vec::new();
        for sequence in sent.keys() {
            match a_channel.state(*sequence).await.map_err(|e| format!("{e:?}"))? {
                MessageState::Delivered => delivered += 1,
                MessageState::Failed => failed.push(*sequence),
                MessageState::New | MessageState::Sent => {}
            }
        }
        if !failed.is_empty() {
            return Err(format!("channel messages failed: {failed:?}"));
        }
        let echoed = echoes.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len();
        if delivered == sent.len() && echoed >= sent.len() {
            break delivered;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "channel did not complete: delivered {delivered}/{} echoed {echoed}/{}",
                sent.len(),
                sent.len()
            ));
        }
        tokio::time::sleep(POLL).await;
    };
    let elapsed_ms = send_started.elapsed().as_millis() as u64;

    // Payload integrity: B received exactly what A sent, and A got it back.
    let mut expected: Vec<Vec<u8>> = sent.values().cloned().collect();
    expected.sort();
    let mut received =
        received_by_b.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    received.sort();
    let mut echoed = echoes.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    echoed.sort();
    let integrity = received == expected && echoed == expected;

    // B's echoes must be proved by A as well.
    let echo_wait = Instant::now() + Duration::from_secs(10);
    let echo_sequences_snapshot =
        echo_sequences.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let mut echo_delivered = 0usize;
    while Instant::now() < echo_wait {
        echo_delivered = 0;
        for sequence in &echo_sequences_snapshot {
            if matches!(
                b_channel.state(*sequence).await.map_err(|e| format!("{e:?}"))?,
                MessageState::Delivered
            ) {
                echo_delivered += 1;
            }
        }
        if echo_delivered == echo_sequences_snapshot.len() {
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    echo_task.abort();

    proof["channel"] = serde_json::json!({
        "mdu": mdu,
        "payload_bytes": payload_bytes,
        "messages": args.messages,
        "sent": sent.len(),
        "delivered_to_b": delivered,
        "received_by_b": received.len(),
        "echoed_to_a": echoed.len(),
        "echoes_delivered_to_a": echo_delivered,
        "integrity_verified": integrity,
        "elapsed_ms": elapsed_ms,
    });
    println!(
        "probe: channel delivered {delivered}/{} echoed {}/{} integrity={integrity}",
        sent.len(),
        echoed.len(),
        sent.len()
    );
    if !integrity {
        return Err("channel payloads did not round-trip intact".into());
    }
    if echo_delivered != echo_sequences_snapshot.len() {
        return Err(format!(
            "B's echoes were not all proved: {echo_delivered}/{}",
            echo_sequences_snapshot.len()
        ));
    }
    a_channel.close().await.map_err(|e| format!("close channel: {e:?}"))?;
    Ok(())
}
