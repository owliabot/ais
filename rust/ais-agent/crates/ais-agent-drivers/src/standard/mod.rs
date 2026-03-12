//! Standard protocol-driver interface.

mod fragment;
mod request;
mod trait_def;

pub use fragment::{ActionGraphFragment, StandardDriverOutput};
pub use request::StandardDriverRequest;
pub use trait_def::{StandardDriver, StandardDriverError};

#[cfg(test)]
mod tests;
