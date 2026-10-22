//! 会话协议与传输抽象（合并原 app/protocol + app/transport）。

pub mod protocol;
pub mod transport;
pub use protocol::*;
pub use transport::*;
