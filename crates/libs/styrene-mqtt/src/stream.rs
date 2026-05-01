use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;

use crate::envelope::Message;
use crate::error::MqttError;

/// A typed subscription that yields deserialized Aether messages.
///
/// Implements [`Stream`] for use with `StreamExt` combinators.
pub struct Subscription<T> {
    rx: mpsc::Receiver<Result<Message<T>, MqttError>>,
}

impl<T> Subscription<T> {
    pub(crate) fn new(rx: mpsc::Receiver<Result<Message<T>, MqttError>>) -> Self {
        Self { rx }
    }

    /// Receive the next message. Returns `None` when the subscription closes.
    pub async fn recv(&mut self) -> Option<Result<Message<T>, MqttError>> {
        self.rx.recv().await
    }
}

impl<T: DeserializeOwned + Send + 'static> Stream for Subscription<T> {
    type Item = Result<Message<T>, MqttError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// A raw MQTT message before envelope decoding.
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retained: bool,
    pub user_properties: Vec<(String, String)>,
}
