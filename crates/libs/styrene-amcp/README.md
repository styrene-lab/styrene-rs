# styrene-amcp

Async Rust client for the CasparCG AMCP (Advanced Media Control Protocol).

## Usage

```rust
use styrene_amcp::{AmcpClient, AmcpCommand};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = AmcpClient::connect("127.0.0.1:5250").await?;

    // Load and play a template
    let cmd = AmcpCommand::cg_add_with_data(
        1, 10,                              // channel, layer
        "lower_third",                       // template name
        r#"{"name":"Wilson"}"#,             // JSON data
    );
    let resp = client.send(cmd).await?;
    println!("Response: {}", resp.code);    // 202 = CG OK

    // Update template data
    let cmd = AmcpCommand::cg_update(1, 10, r#"{"name":"Chris"}"#);
    client.send(cmd).await?;

    // Stop template
    client.send(AmcpCommand::cg_stop(1, 10)).await?;

    Ok(())
}
```

## Features

- **Async** — Tokio-based, non-blocking TCP
- **Type-safe commands** — `AmcpCommand` enum with builder helpers
- **Correct response parsing** — 200 (multiline), 201 (single-line), 202 (no body) semantics
- **Timeouts** — configurable per-connection, defaults to 5 seconds
- **Wire escaping** — JSON data properly escaped for AMCP quoted strings

## AMCP Commands Supported

| Command | Builder | Wire format |
|---------|---------|-------------|
| CG ADD | `AmcpCommand::cg_add()` / `cg_add_with_data()` | `CG 1-10 ADD 0 "template" 1 "data"` |
| CG UPDATE | `AmcpCommand::cg_update()` | `CG 1-10 UPDATE 0 "data"` |
| CG PLAY | `AmcpCommand::CgPlay { .. }` | `CG 1-10 PLAY 0` |
| CG STOP | `AmcpCommand::cg_stop()` | `CG 1-10 STOP 0` |
| CG CLEAR | `AmcpCommand::CgClear { .. }` | `CG 1-10 CLEAR` |
| PLAY | `AmcpCommand::Play { .. }` | `PLAY 1-1 "clip"` |
| LOAD | `AmcpCommand::Load { .. }` | `LOAD 1-1 "clip"` |
| STOP | `AmcpCommand::Stop { .. }` | `STOP 1-1` |
| CLEAR | `AmcpCommand::Clear { .. }` | `CLEAR 1` |
| Raw | `AmcpCommand::Raw(s)` | Any AMCP string |

## Response Codes

| Code | Meaning | Body |
|------|---------|------|
| 200 | OK | Multiline (terminated by blank line) |
| 201 | OK | Single line of data |
| 202 | OK | No body (CG commands) |
| 400-502 | Error | No body |

## License

Proprietary — Styrene Lab
