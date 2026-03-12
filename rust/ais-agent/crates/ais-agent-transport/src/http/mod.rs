//! Thin HTTP routes over the host command service contract.

mod error;
mod router;
mod state;

#[cfg(test)]
mod tests;

pub use router::build_http_router;
