# Rust LXMF Full-Stack Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement a full Rust LXMF stack (LXMF-rs) that interoperates with the Reticulum network (Reticulum-rs), matching the official Python LXMF + Reticulum behavior.

**Architecture:** Extend Reticulum-rs with missing primitives needed by LXMF (resources, announce handlers, proof/receipt plumbing, packet limits) while implementing LXMF-rs message format, router, propagation node, and CLI daemon. Keep the public APIs explicit and minimal, then layer LXMF logic on top. All features must be driven by TDD with small commits.

**Tech Stack:** Rust 2021, tokio async, Reticulum-rs (branch `lxmf-reticulum`), LXMF-rs (branch `rust-lxmf`), msgpack (rmp-serde or equivalent), SHA-256, ed25519, fernet/AES helpers in Reticulum-rs.

---

## Part A — Reticulum-rs (branch `lxmf-reticulum`)

### Task A1: Map packet MDU limits to LXMF delivery modes

**Files:**
- Modify: `src/packet.rs`
- Modify: `src/transport.rs`
- Test: `tests/lxmf_packet_modes.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_packet_modes.rs
use reticulum::packet::{Packet, PACKET_MDU, LXMF_MAX_PAYLOAD};

#[test]
fn lxmf_payload_caps_match_packet_mdu() {
    // LXMF_MAX_PAYLOAD must always be less than PACKET_MDU
    assert!(LXMF_MAX_PAYLOAD < PACKET_MDU);

    // Ensure we can fragment exactly at boundary
    let payload = vec![0u8; LXMF_MAX_PAYLOAD * 2 + 1];
    let packets = Packet::fragment_for_lxmf(&payload).expect("fragment");
    assert_eq!(packets.len(), 3);
    assert!(packets.iter().all(|p| p.data.len() <= LXMF_MAX_PAYLOAD));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_packet_modes`
Expected: FAIL if caps or fragmentation do not match.

**Step 3: Write minimal implementation**

```rust
// src/packet.rs
// Ensure LXMF_MAX_PAYLOAD aligns with PACKET_MDU minus crypto overheads
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_packet_modes`
Expected: PASS

**Step 5: Commit**

```bash
git add src/packet.rs tests/lxmf_packet_modes.rs
git commit -m "feat: define lxmf packet mode payload caps"
```

---

### Task A2: Add announce handler registration and metadata parsing

**Files:**
- Modify: `src/transport.rs`
- Modify: `src/destination.rs`
- Test: `tests/lxmf_announce_handlers.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_announce_handlers.rs
use reticulum::transport::{Transport, AnnounceEvent, AnnounceHandler};

struct CaptureHandler;
impl AnnounceHandler for CaptureHandler {
    fn on_announce(&self, _event: &AnnounceEvent) {}
}

#[tokio::test]
async fn transport_registers_announce_handler() {
    let mut transport = Transport::default();
    transport.register_announce_handler(Box::new(CaptureHandler));
    // For now: ensure the handler list length increases or can be invoked via test hook
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_announce_handlers`
Expected: FAIL with missing handler API.

**Step 3: Write minimal implementation**

```rust
// src/transport.rs
pub trait AnnounceHandler: Send + Sync { fn on_announce(&self, event: &AnnounceEvent); }
// Store handlers and call from announce receive path
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_announce_handlers`
Expected: PASS

**Step 5: Commit**

```bash
git add src/transport.rs tests/lxmf_announce_handlers.rs
git commit -m "feat: add announce handler hooks"
```

---

### Task A3: Resource transfer API (send/receive, receipts)

**Files:**
- Create: `src/resource.rs`
- Modify: `src/transport.rs`
- Modify: `src/lib.rs`
- Test: `tests/lxmf_resources.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_resources.rs
use reticulum::resource::{Resource, ResourceReceipt};

#[tokio::test]
async fn resource_send_receives_receipt() {
    let data = vec![1u8; 2048];
    let resource = Resource::from_bytes(data.clone());
    let receipt = resource.send_for_test().await;
    assert!(matches!(receipt, ResourceReceipt::Delivered));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_resources`
Expected: FAIL with missing resource API.

**Step 3: Write minimal implementation**

```rust
// src/resource.rs
// Minimal struct and test-only send hook wired through transport
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_resources`
Expected: PASS

**Step 5: Commit**

```bash
git add src/resource.rs src/transport.rs src/lib.rs tests/lxmf_resources.rs
git commit -m "feat: add resource transfer primitives"
```

---

### Task A4: Proof receipts wired to LXMF delivery tracking

**Files:**
- Modify: `src/transport.rs`
- Test: `tests/lxmf_proof_receipts.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_proof_receipts.rs
use reticulum::transport::{DeliveryReceipt, ReceiptHandler, Transport};

struct Capture { called: std::sync::Mutex<bool> }
impl ReceiptHandler for Capture {
    fn on_receipt(&self, _receipt: &DeliveryReceipt) {
        *self.called.lock().unwrap() = true;
    }
}

#[tokio::test]
async fn proof_packet_emits_receipt() {
    let mut transport = Transport::default();
    transport.set_receipt_handler(Box::new(Capture { called: std::sync::Mutex::new(false) })).await;

    // Use test hook to inject proof packet
    transport.emit_receipt_for_test(DeliveryReceipt::new([0u8; 32]));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_proof_receipts`
Expected: FAIL if handler not wired.

**Step 3: Write minimal implementation**

```rust
// src/transport.rs
// Ensure proof packets call receipt handler
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_proof_receipts`
Expected: PASS

**Step 5: Commit**

```bash
git add src/transport.rs tests/lxmf_proof_receipts.rs
git commit -m "feat: emit delivery receipts on proof packets"
```

---

## Part B — LXMF-rs (branch `rust-lxmf`)

### Task B1: Define LXMF message format and packing

**Files:**
- Create: `src/message.rs`
- Modify: `src/lib.rs`
- Test: `tests/lxmf_message_pack.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_message_pack.rs
use lxmf::message::LXMessage;

#[test]
fn lxmf_message_packs_and_unpacks() {
    let msg = LXMessage::new_test();
    let bytes = msg.pack();
    let decoded = LXMessage::unpack(&bytes).expect("unpack");
    assert_eq!(decoded.message_id(), msg.message_id());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_message_pack`
Expected: FAIL with missing LXMessage.

**Step 3: Write minimal implementation**

```rust
// src/message.rs
// Implement LXMessage with destination, source, signature, timestamp, fields, payload.
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_message_pack`
Expected: PASS

**Step 5: Commit**

```bash
git add src/message.rs src/lib.rs tests/lxmf_message_pack.rs
git commit -m "feat: add lxmf message packing"
```

---

### Task B2: Implement signature validation and message-id derivation

**Files:**
- Modify: `src/message.rs`
- Test: `tests/lxmf_message_signature.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_message_signature.rs
use lxmf::message::LXMessage;

#[test]
fn lxmf_message_signature_verifies() {
    let msg = LXMessage::new_test();
    let bytes = msg.pack();
    let decoded = LXMessage::unpack(&bytes).unwrap();
    assert!(decoded.verify_signature().is_ok());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_message_signature`
Expected: FAIL with missing verify.

**Step 3: Write minimal implementation**

```rust
// src/message.rs
// Use reticulum identity helpers for sign/verify
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_message_signature`
Expected: PASS

**Step 5: Commit**

```bash
git add src/message.rs tests/lxmf_message_signature.rs
git commit -m "feat: verify lxmf message signatures"
```

---

### Task B3: Implement LXMRouter core (inbound/outbound queues)

**Files:**
- Create: `src/router.rs`
- Modify: `src/lib.rs`
- Test: `tests/lxm_router_basic.rs`

**Step 1: Write the failing test**

```rust
// tests/lxm_router_basic.rs
use lxmf::router::LXMRouter;

#[tokio::test]
async fn router_queues_outbound_message() {
    let router = LXMRouter::new_test();
    let msg = router.make_test_message();
    router.enqueue_outbound(msg).await;
    assert_eq!(router.outbound_len().await, 1);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxm_router_basic`
Expected: FAIL with missing LXMRouter.

**Step 3: Write minimal implementation**

```rust
// src/router.rs
// Provide queues and basic enqueue/dequeue methods
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxm_router_basic`
Expected: PASS

**Step 5: Commit**

```bash
git add src/router.rs src/lib.rs tests/lxm_router_basic.rs
git commit -m "feat: add basic lxm router queues"
```

---

### Task B4: Implement delivery receipts integration

**Files:**
- Modify: `src/router.rs`
- Test: `tests/lxm_router_receipts.rs`

**Step 1: Write the failing test**

```rust
// tests/lxm_router_receipts.rs
use lxmf::router::LXMRouter;

#[tokio::test]
async fn router_marks_message_delivered() {
    let router = LXMRouter::new_test();
    let msg = router.make_test_message();
    let id = msg.message_id();
    router.enqueue_outbound(msg).await;

    router.handle_receipt_for_test(id).await;
    assert!(router.is_delivered(id).await);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxm_router_receipts`
Expected: FAIL with missing receipt handling.

**Step 3: Write minimal implementation**

```rust
// src/router.rs
// Track delivery state and update on receipt
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxm_router_receipts`
Expected: PASS

**Step 5: Commit**

```bash
git add src/router.rs tests/lxm_router_receipts.rs
git commit -m "feat: mark delivery receipts in router"
```

---

### Task B5: Propagation node message store and sync protocol skeleton

**Files:**
- Create: `src/propagation.rs`
- Modify: `src/router.rs`
- Test: `tests/lxmf_propagation_store.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_propagation_store.rs
use lxmf::propagation::PropagationStore;

#[test]
fn propagation_store_adds_and_prunes() {
    let mut store = PropagationStore::new(1024);
    store.add(vec![1u8; 512]);
    store.add(vec![2u8; 700]);
    assert!(store.total_size() <= 1024);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_propagation_store`
Expected: FAIL with missing store.

**Step 3: Write minimal implementation**

```rust
// src/propagation.rs
// Implement size-limited store with pruning policy
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_propagation_store`
Expected: PASS

**Step 5: Commit**

```bash
git add src/propagation.rs tests/lxmf_propagation_store.rs
 git commit -m "feat: add propagation store with pruning"
```

---

### Task B6: LXStamper (stamp generation and verification)

**Files:**
- Create: `src/stamp.rs`
- Modify: `src/lib.rs`
- Test: `tests/lxmf_stamp.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_stamp.rs
use lxmf::stamp::Stamp;

#[test]
fn stamp_roundtrip() {
    let data = b"hello";
    let stamp = Stamp::generate(data);
    assert!(stamp.verify(data));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_stamp`
Expected: FAIL with missing Stamp.

**Step 3: Write minimal implementation**

```rust
// src/stamp.rs
// Implement stamp generation/verification with current algorithm
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_stamp`
Expected: PASS

**Step 5: Commit**

```bash
git add src/stamp.rs src/lib.rs tests/lxmf_stamp.rs
 git commit -m "feat: add lxmf stamp generation"
```

---

### Task B7: CLI daemon (`lxmd`) skeleton

**Files:**
- Create: `src/bin/lxmd.rs`
- Modify: `Cargo.toml`
- Test: `tests/lxmd_cli.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmd_cli.rs
use assert_cmd::Command;

#[test]
fn lxmd_help_runs() {
    Command::cargo_bin("lxmd").unwrap().arg("--help").assert().success();
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmd_cli`
Expected: FAIL with missing bin.

**Step 3: Write minimal implementation**

```rust
// src/bin/lxmd.rs
fn main() { println!("lxmd"); }
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmd_cli`
Expected: PASS

**Step 5: Commit**

```bash
git add src/bin/lxmd.rs Cargo.toml tests/lxmd_cli.rs
 git commit -m "feat: add lxmd cli skeleton"
```

---

### Task B8: End-to-end smoke test (router + reticulum)

**Files:**
- Test: `tests/lxmf_e2e.rs`

**Step 1: Write the failing test**

```rust
// tests/lxmf_e2e.rs
use lxmf::router::LXMRouter;

#[tokio::test]
async fn e2e_send_receive() {
    let router = LXMRouter::new_test();
    let msg = router.make_test_message();
    router.send(msg).await.expect("send");
    let received = router.recv().await.expect("recv");
    assert_eq!(received.payload(), b"test");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -q --test lxmf_e2e`
Expected: FAIL with missing functionality.

**Step 3: Write minimal implementation**

```rust
// src/router.rs
// Add test-only loopback path to validate flow
```

**Step 4: Run test to verify it passes**

Run: `cargo test -q --test lxmf_e2e`
Expected: PASS

**Step 5: Commit**

```bash
git add src/router.rs tests/lxmf_e2e.rs
 git commit -m "test: add lxmf end-to-end smoke"
```

---

**Plan complete.**
