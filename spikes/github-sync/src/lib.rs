//! Spike: offline-first multi-device sync for xpass.
//!
//! - [`store`]: `RemoteStore` trait (GitHub Contents API semantics) + in-memory mock.
//! - [`github`]: real GitHub Contents API backend.
//! - [`merge`]: per-entry last-writer-wins merge with tombstones.
//! - [`engine`]: a device = local encrypted copy + sync state + sync loop.

pub mod engine;
pub mod github;
pub mod merge;
pub mod store;
