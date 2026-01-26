#[derive(Debug, Clone, PartialEq)]
pub struct Ticket {
    pub expires: f64,
    pub token: Vec<u8>,
}

impl Ticket {
    pub fn new(expires: f64, token: Vec<u8>) -> Self {
        Self { expires, token }
    }

    pub fn is_valid(&self, now: f64) -> bool {
        now <= self.expires
    }
}
