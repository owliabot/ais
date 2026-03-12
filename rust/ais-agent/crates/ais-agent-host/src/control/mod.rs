//! Host command result and service contracts.

mod outcome;
mod service;

pub use outcome::{
    HostAcceptedResponse, HostCommandError, HostCommandOutcome, HostCommandResponse,
};
pub use service::HostCommandService;
