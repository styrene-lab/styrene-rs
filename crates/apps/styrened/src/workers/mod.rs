//! Background worker tasks for the daemon.
//!
//! Workers are spawned tokio tasks that bridge transport events to the
//! service layer. They subscribe to transport broadcast channels and
//! feed decoded data into services.

pub mod announce;
pub mod inbound;
pub mod link;
pub mod native_nomadnet;
pub mod page_handler;
pub mod propagation_handler;
pub mod route;
pub mod router;
pub mod rpc_request;
pub mod rpc_response;
pub mod standard_propagation;

use std::sync::Arc;

use crate::app_context::AppContext;
use rns_core::identity::PrivateIdentity;

/// Register Styrene RPC response and request handlers in dispatch order.
pub async fn register_styrene_rpc_handlers(app_context: &AppContext, signer: Arc<PrivateIdentity>) {
    app_context
        .protocol()
        .register(Arc::new(rpc_response::RpcResponseHandler::new(app_context.fleet_arc())))
        .await;
    app_context
        .protocol()
        .register(Arc::new(rpc_request::RpcRequestHandler::new(
            app_context.transport_arc(),
            signer,
            app_context.policy_arc(),
        )))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::messages::MessagesStore;
    use crate::transport::null_transport::NullTransport;
    use std::sync::Mutex;

    #[tokio::test]
    async fn rpc_handlers_share_one_protocol_registration() {
        let identity = PrivateIdentity::new_from_name("rpc-registration-order");
        let identity_hash = hex::encode(identity.address_hash().as_slice());
        let store = Arc::new(Mutex::new(MessagesStore::in_memory().unwrap()));
        let app_context = AppContext::new(Arc::new(NullTransport::new()), identity_hash, store);

        register_styrene_rpc_handlers(&app_context, Arc::new(identity)).await;

        assert_eq!(app_context.protocol().handler_count().await, 2);
        assert_eq!(app_context.protocol().registered_protocols().await, ["styrene"]);
    }
}
