mod payload;
mod state;
mod wire;

pub use payload::Payload;
pub use state::State;
pub use wire::WireMessage;

#[derive(Debug, Clone)]
pub struct Message {
    state: State,
}

impl Message {
    pub fn new() -> Self {
        Self {
            state: State::Generating,
        }
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub fn is_outbound(&self) -> bool {
        self.state == State::Outbound
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::new()
    }
}
