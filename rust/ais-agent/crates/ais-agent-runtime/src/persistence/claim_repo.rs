use ais_agent_control::{
    ids::{ClaimId, RunId},
    ownership::{RunClaim, RunClaimStatus},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRenewRequest {
    pub run_id: RunId,
    pub claim_id: ClaimId,
    pub claim_epoch: u64,
    pub renewed_at_ms: u64,
    pub lease_expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReleaseRequest {
    pub run_id: RunId,
    pub claim_id: ClaimId,
    pub claim_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimExpireRequest {
    pub run_id: RunId,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimSupersedeRequest {
    pub run_id: RunId,
    pub predecessor_claim_id: ClaimId,
    pub predecessor_claim_epoch: u64,
    pub successor_claim: RunClaim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimSupersedeResult {
    pub predecessor: RunClaim,
    pub successor: RunClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunClaimRepositoryError {
    #[error("claim `{claim_id}` not found")]
    ClaimNotFound { claim_id: String },
    #[error("active claim conflict for run `{run_id}` with claim `{claim_id}`")]
    ActiveClaimConflict { run_id: String, claim_id: String },
    #[error(
        "claim epoch conflict for claim `{claim_id}`: expected `{expected_claim_epoch}`, actual `{actual_claim_epoch}`"
    )]
    ClaimEpochConflict {
        claim_id: String,
        expected_claim_epoch: u64,
        actual_claim_epoch: u64,
    },
    #[error("claim `{claim_id}` has invalid status `{status:?}` for this transition")]
    InvalidStatus {
        claim_id: String,
        status: RunClaimStatus,
    },
    #[error("run claim repository rejected invalid claim: {message}")]
    InvalidClaim { message: String },
    #[error("run claim repository storage error: {message}")]
    Storage { message: String },
}

pub trait RunClaimRepository {
    fn acquire(&mut self, claim: RunClaim) -> Result<RunClaim, RunClaimRepositoryError>;

    fn renew(&mut self, request: ClaimRenewRequest) -> Result<RunClaim, RunClaimRepositoryError>;

    fn release(
        &mut self,
        request: ClaimReleaseRequest,
    ) -> Result<RunClaim, RunClaimRepositoryError>;

    fn load_active(&self, run_id: &RunId) -> Result<Option<RunClaim>, RunClaimRepositoryError>;

    fn load_latest_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError>;

    fn load_claim(&self, claim_id: &ClaimId) -> Result<RunClaim, RunClaimRepositoryError>;

    fn expire_stale(
        &mut self,
        request: ClaimExpireRequest,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError>;

    fn supersede(
        &mut self,
        request: ClaimSupersedeRequest,
    ) -> Result<ClaimSupersedeResult, RunClaimRepositoryError>;
}

impl<T> RunClaimRepository for &mut T
where
    T: RunClaimRepository + ?Sized,
{
    fn acquire(&mut self, claim: RunClaim) -> Result<RunClaim, RunClaimRepositoryError> {
        (**self).acquire(claim)
    }

    fn renew(&mut self, request: ClaimRenewRequest) -> Result<RunClaim, RunClaimRepositoryError> {
        (**self).renew(request)
    }

    fn release(
        &mut self,
        request: ClaimReleaseRequest,
    ) -> Result<RunClaim, RunClaimRepositoryError> {
        (**self).release(request)
    }

    fn load_active(&self, run_id: &RunId) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        (**self).load_active(run_id)
    }

    fn load_latest_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        (**self).load_latest_for_run(run_id)
    }

    fn load_claim(&self, claim_id: &ClaimId) -> Result<RunClaim, RunClaimRepositoryError> {
        (**self).load_claim(claim_id)
    }

    fn expire_stale(
        &mut self,
        request: ClaimExpireRequest,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        (**self).expire_stale(request)
    }

    fn supersede(
        &mut self,
        request: ClaimSupersedeRequest,
    ) -> Result<ClaimSupersedeResult, RunClaimRepositoryError> {
        (**self).supersede(request)
    }
}
