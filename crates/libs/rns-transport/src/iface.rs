#[derive(Clone, Debug, Default)]
pub struct TcpClient {
    pub endpoint: String,
}

impl TcpClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into() }
    }

    pub fn spawn() {}
}

#[derive(Clone, Debug, Default)]
pub struct TcpServer {
    pub endpoint: String,
}

impl TcpServer {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into() }
    }

    pub fn spawn() {}
}

#[derive(Clone, Debug, Default)]
pub struct UdpInterface {
    pub bind: String,
}

impl UdpInterface {
    pub fn new(bind: impl Into<String>) -> Self {
        Self { bind: bind.into() }
    }

    pub fn spawn() {}
}
