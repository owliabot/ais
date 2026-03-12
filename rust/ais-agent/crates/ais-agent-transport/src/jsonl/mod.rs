//! JSONL transport for host command/control contracts.

mod codec;
mod frames;
mod server;

#[cfg(test)]
mod tests;

pub use codec::{decode_inbound_line, encode_outbound_frame};
pub use frames::{
    JsonlInboundFrame, JsonlOutboundFrame, JsonlResponseFrame, JsonlServerErrorFrame,
};
pub use server::JsonlServer;
