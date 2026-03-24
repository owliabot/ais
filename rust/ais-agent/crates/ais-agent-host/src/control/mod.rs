//! Host command result and service contracts.

mod outcome;
mod service;

pub use outcome::{
    HostAcceptedResponse, HostCommandError, HostCommandOutcome, HostCommandResponse,
    HostErrorClass, HostErrorCorrelation, HostErrorRecoveryHints, HostProviderBindingErrorContext,
};
pub use service::HostCommandService;
