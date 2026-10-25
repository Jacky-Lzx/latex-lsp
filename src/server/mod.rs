//! LSP handlers, ordered dispatch and external task lifecycle.

mod backend;
pub mod client;
pub mod dispatch;
pub(crate) mod output;
pub mod protocol;
pub(crate) mod tasks;

pub use backend::Backend;
