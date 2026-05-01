//! HTTP proxy server — captures .i2p requests and forwards them.
//!
//! In direct mode, forwards to a local i2pd HTTP proxy.
//! In mesh mode (future), serializes as I2pProxyRequest over RNS.

use crate::mesh_client::MeshClient;
use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Run the proxy in direct mode — forward .i2p requests to a local i2pd.
pub async fn run_direct(bind: &str, i2pd_addr: &str) -> Result<()> {
    let addr: SocketAddr = bind.parse()?;
    let listener = TcpListener::bind(addr).await?;
    let i2pd_addr = Arc::new(i2pd_addr.to_string());

    eprintln!("[proxy] listening on {addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let i2pd = i2pd_addr.clone();

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req| {
                let i2pd = i2pd.clone();
                handle_request(req, i2pd, peer)
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                if !e.to_string().contains("connection closed") {
                    eprintln!("[proxy] connection error from {peer}: {e}");
                }
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    i2pd_addr: Arc<String>,
    peer: SocketAddr,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let host = uri
        .host()
        .or_else(|| {
            req.headers()
                .get("host")
                .and_then(|h| h.to_str().ok())
                .map(|h| h.split(':').next().unwrap_or(h))
        })
        .unwrap_or("");

    // Only handle .i2p domains
    if !host.ends_with(".i2p") {
        eprintln!("[proxy] rejected non-.i2p request from {peer}: {uri}");
        let body = "This proxy only handles .i2p domains.\n";
        return Ok(Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header("Content-Type", "text/plain")
            .body(Full::new(Bytes::from(body)))
            .unwrap());
    }

    eprintln!("[proxy] {method} {uri} from {peer}");

    // Collect request headers
    let mut headers = Vec::new();
    for (name, value) in req.headers() {
        if let Ok(v) = value.to_str() {
            headers.push((name.to_string(), v.to_string()));
        }
    }

    // Collect request body
    let body_bytes = match req.collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            eprintln!("[proxy] failed to read request body: {e}");
            return Ok(error_response(502, "Failed to read request body"));
        }
    };

    // Forward to i2pd via reqwest
    let proxy = match reqwest::Proxy::all(&*i2pd_addr) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[proxy] i2pd proxy config error: {e}");
            return Ok(error_response(502, "i2pd proxy configuration error"));
        }
    };

    let client = match reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[proxy] failed to build HTTP client: {e}");
            return Ok(error_response(502, "Failed to build HTTP client"));
        }
    };

    let reqwest_method = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "HEAD" => reqwest::Method::HEAD,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => {
            return Ok(error_response(400, "Unsupported HTTP method"));
        }
    };

    let url = uri.to_string();
    let mut req_builder = client.request(reqwest_method, &url);
    for (name, value) in &headers {
        // Skip hop-by-hop headers
        if !matches!(
            name.to_lowercase().as_str(),
            "connection" | "proxy-connection" | "keep-alive" | "transfer-encoding"
        ) {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }
    }

    if !body_bytes.is_empty() {
        req_builder = req_builder.body(body_bytes.to_vec());
    }

    let response = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            let code = if e.is_timeout() { 504 } else { 502 };
            let msg = if e.is_timeout() {
                "i2pd request timed out"
            } else {
                "i2pd proxy unreachable"
            };
            eprintln!("[proxy] i2pd error for {url}: {e}");
            return Ok(error_response(code, msg));
        }
    };

    // Build the response
    let status = response.status();
    let mut builder = Response::builder().status(status);

    for (name, value) in response.headers() {
        // Skip hop-by-hop headers
        if !matches!(
            name.as_str(),
            "connection" | "transfer-encoding" | "keep-alive"
        ) {
            builder = builder.header(name, value);
        }
    }

    let response_body = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[proxy] failed to read i2pd response body: {e}");
            return Ok(error_response(502, "Failed to read response from i2pd"));
        }
    };

    eprintln!("[proxy] {status} {} bytes for {url}", response_body.len());

    Ok(builder
        .body(Full::new(Bytes::from(response_body.to_vec())))
        .unwrap())
}

fn error_response(code: u16, message: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY))
        .header("Content-Type", "text/plain")
        .body(Full::new(Bytes::from(format!("{message}\n"))))
        .unwrap()
}

/// Run the proxy in mesh mode — forward .i2p requests to the hub via RNS.
pub async fn run_mesh(bind: &str, client: Arc<MeshClient>) -> Result<()> {
    let addr: SocketAddr = bind.parse()?;
    let listener = TcpListener::bind(addr).await?;

    eprintln!("[proxy] mesh mode listening on {addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let client = client.clone();

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req| {
                let client = client.clone();
                handle_mesh_request(req, client, peer)
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                if !e.to_string().contains("connection closed") {
                    eprintln!("[proxy] connection error from {peer}: {e}");
                }
            }
        });
    }
}

async fn handle_mesh_request(
    req: Request<hyper::body::Incoming>,
    client: Arc<MeshClient>,
    peer: SocketAddr,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let host = uri
        .host()
        .or_else(|| {
            req.headers()
                .get("host")
                .and_then(|h| h.to_str().ok())
                .map(|h| h.split(':').next().unwrap_or(h))
        })
        .unwrap_or("");

    if !host.ends_with(".i2p") {
        return Ok(error_response(502, "This proxy only handles .i2p domains."));
    }

    eprintln!("[proxy] mesh {method} {uri} from {peer}");

    // Collect headers
    let mut headers = Vec::new();
    for (name, value) in req.headers() {
        if let Ok(v) = value.to_str() {
            if !matches!(
                name.to_string().to_lowercase().as_str(),
                "connection" | "proxy-connection" | "keep-alive" | "transfer-encoding"
            ) {
                headers.push((name.to_string(), v.to_string()));
            }
        }
    }

    // Collect body
    let body_bytes = match req.collect().await {
        Ok(b) => {
            let b = b.to_bytes();
            if b.is_empty() { None } else { Some(b.to_vec()) }
        }
        Err(e) => {
            eprintln!("[proxy] failed to read request body: {e}");
            return Ok(error_response(502, "Failed to read request body"));
        }
    };

    // Send via mesh
    let url = uri.to_string();
    match client
        .proxy_request(method.as_str(), &url, headers, body_bytes)
        .await
    {
        Ok(resp) => {
            let mut builder = Response::builder().status(resp.status);
            for (name, value) in &resp.headers {
                if !matches!(name.as_str(), "connection" | "transfer-encoding" | "keep-alive") {
                    builder = builder.header(name.as_str(), value.as_str());
                }
            }
            eprintln!("[proxy] mesh {}: {} bytes for {url}", resp.status, resp.body.len());
            Ok(builder
                .body(Full::new(Bytes::from(resp.body)))
                .unwrap())
        }
        Err(e) => {
            eprintln!("[proxy] mesh error for {url}: {e}");
            Ok(error_response(502, &format!("Mesh proxy error: {e}")))
        }
    }
}
