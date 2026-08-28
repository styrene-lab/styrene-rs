use alloc::{collections::BTreeSet, string::String, sync::Arc, vec::Vec};
use core::fmt;

use crate::{
    hash::{ADDRESS_HASH_SIZE, AddressHash, address_hash},
    identity::Identity,
};

pub type RequestId = [u8; ADDRESS_HASH_SIZE];
pub type RequestPathHash = [u8; ADDRESS_HASH_SIZE];
pub type RequestHandler =
    Arc<dyn Fn(&[u8], Option<&Identity>, &RequestLinkContext, RequestId) -> Vec<u8> + Send + Sync>;
pub type RequestAccessCallback =
    Arc<dyn Fn(Option<&Identity>, &RequestLinkContext) -> bool + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLinkContext {
    pub link_id: AddressHash,
    pub destination: AddressHash,
}

#[derive(Clone)]
pub enum RequestAccess {
    Public,
    Identified,
    AllowList(BTreeSet<AddressHash>),
    Callback(RequestAccessCallback),
}

impl fmt::Debug for RequestAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => formatter.write_str("Public"),
            Self::Identified => formatter.write_str("Identified"),
            Self::AllowList(identities) => {
                formatter.debug_tuple("AllowList").field(identities).finish()
            }
            Self::Callback(_) => formatter.write_str("Callback(..)"),
        }
    }
}

impl RequestAccess {
    fn allows(&self, remote_identity: Option<&Identity>, link: &RequestLinkContext) -> bool {
        match self {
            Self::Public => true,
            Self::Identified => remote_identity.is_some(),
            Self::AllowList(allowed) => {
                remote_identity.is_some_and(|identity| allowed.contains(&identity.address_hash))
            }
            Self::Callback(callback) => callback(remote_identity, link),
        }
    }
}

pub struct RequestPath {
    path: String,
    path_hash: RequestPathHash,
    access: RequestAccess,
    max_request_size: usize,
    max_response_size: usize,
    handler: RequestHandler,
}

impl fmt::Debug for RequestPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestPath")
            .field("path", &self.path)
            .field("path_hash", &self.path_hash)
            .field("access", &self.access)
            .field("max_request_size", &self.max_request_size)
            .field("max_response_size", &self.max_response_size)
            .finish_non_exhaustive()
    }
}

impl RequestPath {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub const fn path_hash(&self) -> RequestPathHash {
        self.path_hash
    }

    pub const fn max_request_size(&self) -> usize {
        self.max_request_size
    }

    pub const fn max_response_size(&self) -> usize {
        self.max_response_size
    }

    pub fn access(&self) -> &RequestAccess {
        &self.access
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRegistrationError {
    InvalidPath,
    InvalidLimits,
    DuplicatePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestDispatchError {
    PathNotFound,
    RequestTooLarge,
    Unauthorized,
    ResponseTooLarge,
}

#[derive(Default)]
pub(crate) struct RequestRegistry {
    paths: alloc::collections::BTreeMap<RequestPathHash, RequestPath>,
}

impl RequestRegistry {
    pub(crate) fn register(
        &mut self,
        path: &str,
        access: RequestAccess,
        max_request_size: usize,
        max_response_size: usize,
        handler: RequestHandler,
    ) -> Result<RequestPathHash, RequestRegistrationError> {
        if !path.starts_with('/') {
            return Err(RequestRegistrationError::InvalidPath);
        }
        if max_request_size == 0 || max_response_size == 0 {
            return Err(RequestRegistrationError::InvalidLimits);
        }

        let path_hash = request_path_hash(path);
        if self.paths.contains_key(&path_hash) {
            return Err(RequestRegistrationError::DuplicatePath);
        }

        self.paths.insert(
            path_hash,
            RequestPath {
                path: String::from(path),
                path_hash,
                access,
                max_request_size,
                max_response_size,
                handler,
            },
        );
        Ok(path_hash)
    }

    pub(crate) fn get(&self, path_hash: &RequestPathHash) -> Option<&RequestPath> {
        self.paths.get(path_hash)
    }

    pub(crate) fn dispatch(
        &self,
        path_hash: &RequestPathHash,
        data: &[u8],
        packed_request_size: usize,
        remote_identity: Option<&Identity>,
        link: &RequestLinkContext,
        request_id: RequestId,
    ) -> Result<Vec<u8>, RequestDispatchError> {
        let path = self.paths.get(path_hash).ok_or(RequestDispatchError::PathNotFound)?;
        if packed_request_size > path.max_request_size {
            return Err(RequestDispatchError::RequestTooLarge);
        }
        if !path.access.allows(remote_identity, link) {
            return Err(RequestDispatchError::Unauthorized);
        }

        let response = (path.handler)(data, remote_identity, link, request_id);
        if response.len() > path.max_response_size {
            return Err(RequestDispatchError::ResponseTooLarge);
        }
        Ok(response)
    }
}

pub fn request_path_hash(path: &str) -> RequestPathHash {
    address_hash(path.as_bytes())
}
