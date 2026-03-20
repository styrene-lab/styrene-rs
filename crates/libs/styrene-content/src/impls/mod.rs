#[cfg(feature = "alloc")]
pub mod ram;

#[cfg(feature = "tokio")]
pub mod tokio_fs;

#[cfg(feature = "embedded-storage")]
pub mod flash;

#[cfg(feature = "alloc")]
pub use ram::RamChunkStore;

#[cfg(feature = "tokio")]
pub use tokio_fs::TokioFsChunkStore;

#[cfg(feature = "embedded-storage")]
pub use flash::FlashChunkStore;
