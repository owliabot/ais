//! Driver layer for the greenfield `ais-agent`.

pub mod api_native;
pub mod reflect;
pub mod registry;
pub mod standard;

pub use registry::{
    route_driver_candidates, DriverCandidate, DriverCapability, DriverPathKind, DriverRegistry,
};
