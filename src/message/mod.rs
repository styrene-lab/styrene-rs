mod container;
mod payload;
mod state;
mod types;
mod wire;

pub use container::MessageContainer;
pub use payload::Payload;
pub use state::State;
pub use types::{MessageMethod, MessageState, TransportMethod, UnverifiedReason};
pub use wire::WireMessage;

#[derive(Debug, Clone)]
pub struct Message {
    pub destination_hash: Option<[u8; 16]>,
    pub source_hash: Option<[u8; 16]>,
    pub content: Vec<u8>,
    pub title: Vec<u8>,
    pub fields: Option<rmpv::Value>,
    pub timestamp: Option<f64>,
    state: State,
}

impl Message {
    pub fn new() -> Self {
        Self {
            destination_hash: None,
            source_hash: None,
            content: Vec::new(),
            title: Vec::new(),
            fields: None,
            timestamp: None,
            state: State::Generating,
        }
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub fn is_outbound(&self) -> bool {
        self.state == State::Outbound
    }

    pub fn set_title_from_string(&mut self, title: &str) {
        self.title = title.as_bytes().to_vec();
    }

    pub fn set_title_from_bytes(&mut self, title: &[u8]) {
        self.title = title.to_vec();
    }

    pub fn title_as_string(&self) -> Option<String> {
        String::from_utf8(self.title.clone()).ok()
    }

    pub fn set_content_from_string(&mut self, content: &str) {
        self.content = content.as_bytes().to_vec();
    }

    pub fn set_content_from_bytes(&mut self, content: &[u8]) {
        self.content = content.to_vec();
    }

    pub fn content_as_string(&self) -> Option<String> {
        String::from_utf8(self.content.clone()).ok()
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::new()
    }
}
