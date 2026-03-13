use serde::{Deserialize, Serialize};

use crate::ids::{ClaimId, RunId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunClaimOwnerKind {
    InteractiveHost,
    BackgroundWorker,
    RecoveryWorker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunClaimMode {
    ExclusiveMutation,
    ObserverOnly,
}

impl RunClaimMode {
    pub fn allows_mutation(&self) -> bool {
        matches!(self, Self::ExclusiveMutation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunClaimStatus {
    Active,
    Expired,
    Released,
    Superseded,
}

impl RunClaimStatus {
    pub fn allows_renew(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn allows_release(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn allows_expire(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn allows_successor_acquire(&self) -> bool {
        matches!(self, Self::Expired | Self::Released | Self::Superseded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimTransitionKind {
    ClaimAcquired,
    ClaimRenewed,
    ClaimReleased,
    ClaimExpired,
    ClaimSuperseded,
    ClaimDenied,
    ClaimRelinked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipErrorCode {
    ClaimRequired,
    ClaimConflict,
    ClaimExpired,
    ClaimNotOwner,
    ClaimEpochStale,
    ClaimTransferRequired,
    ObserverOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunClaim {
    pub claim_id: ClaimId,
    pub run_id: RunId,
    pub host_session_id: String,
    pub owner_kind: RunClaimOwnerKind,
    pub owner_instance_id: String,
    pub lease_started_at_ms: u64,
    pub lease_expires_at_ms: Option<u64>,
    pub last_renewed_at_ms: Option<u64>,
    pub claim_epoch: u64,
    pub mode: RunClaimMode,
    pub status: RunClaimStatus,
}

impl RunClaim {
    pub fn validate(&self) -> Result<(), String> {
        if self.claim_id.0.trim().is_empty() {
            return Err("run_claim.claim_id must not be empty".to_owned());
        }
        if self.run_id.0.trim().is_empty() {
            return Err("run_claim.run_id must not be empty".to_owned());
        }
        if self.host_session_id.trim().is_empty() {
            return Err("run_claim.host_session_id must not be empty".to_owned());
        }
        if self.owner_instance_id.trim().is_empty() {
            return Err("run_claim.owner_instance_id must not be empty".to_owned());
        }
        if self.claim_epoch == 0 {
            return Err("run_claim.claim_epoch must be >= 1".to_owned());
        }
        if let Some(lease_expires_at_ms) = self.lease_expires_at_ms {
            if lease_expires_at_ms < self.lease_started_at_ms {
                return Err(
                    "run_claim.lease_expires_at_ms must be >= lease_started_at_ms".to_owned(),
                );
            }
        }
        if let Some(last_renewed_at_ms) = self.last_renewed_at_ms {
            if last_renewed_at_ms < self.lease_started_at_ms {
                return Err(
                    "run_claim.last_renewed_at_ms must be >= lease_started_at_ms".to_owned(),
                );
            }
            if let Some(lease_expires_at_ms) = self.lease_expires_at_ms {
                if last_renewed_at_ms > lease_expires_at_ms {
                    return Err(
                        "run_claim.last_renewed_at_ms must be <= lease_expires_at_ms".to_owned(),
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipVisibility {
    SameSessionOnly,
    ObserverReadAllowed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOwnershipSnapshot {
    pub run_id: RunId,
    pub current_claim: Option<RunClaim>,
    pub last_terminal_claim_id: Option<ClaimId>,
    pub last_claim_transition: Option<ClaimTransitionKind>,
    pub claim_required_for_mutation: bool,
    pub owner_visibility: OwnershipVisibility,
}

impl RunOwnershipSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.run_id.0.trim().is_empty() {
            return Err("run_ownership_snapshot.run_id must not be empty".to_owned());
        }
        if let Some(claim) = self.current_claim.as_ref() {
            claim.validate()?;
            if claim.run_id != self.run_id {
                return Err(
                    "run_ownership_snapshot.current_claim.run_id must match snapshot.run_id"
                        .to_owned(),
                );
            }
        }
        if !self.claim_required_for_mutation
            && self.owner_visibility == OwnershipVisibility::SameSessionOnly
        {
            return Err(
                "run_ownership_snapshot.same_session_only visibility requires claim_required_for_mutation"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClaimTransitionKind, OwnershipErrorCode, OwnershipVisibility, RunClaim, RunClaimMode,
        RunClaimOwnerKind, RunClaimStatus, RunOwnershipSnapshot,
    };
    use crate::ids::{ClaimId, RunId};

    fn sample_claim() -> RunClaim {
        RunClaim {
            claim_id: ClaimId("claim-1".to_owned()),
            run_id: RunId("run-1".to_owned()),
            host_session_id: "session-1".to_owned(),
            owner_kind: RunClaimOwnerKind::InteractiveHost,
            owner_instance_id: "worker-a".to_owned(),
            lease_started_at_ms: 10,
            lease_expires_at_ms: Some(20),
            last_renewed_at_ms: Some(15),
            claim_epoch: 1,
            mode: RunClaimMode::ExclusiveMutation,
            status: RunClaimStatus::Active,
        }
    }

    #[test]
    fn run_claim_validate_rejects_empty_host_session() {
        let mut claim = sample_claim();
        claim.host_session_id.clear();

        assert_eq!(
            claim.validate(),
            Err("run_claim.host_session_id must not be empty".to_owned())
        );
    }

    #[test]
    fn run_claim_validate_rejects_non_monotonic_lease_times() {
        let mut claim = sample_claim();
        claim.last_renewed_at_ms = Some(9);

        assert_eq!(
            claim.validate(),
            Err("run_claim.last_renewed_at_ms must be >= lease_started_at_ms".to_owned())
        );
    }

    #[test]
    fn run_ownership_snapshot_validate_requires_matching_run_id() {
        let mut claim = sample_claim();
        claim.run_id = RunId("run-2".to_owned());

        let snapshot = RunOwnershipSnapshot {
            run_id: RunId("run-1".to_owned()),
            current_claim: Some(claim),
            last_terminal_claim_id: None,
            last_claim_transition: Some(ClaimTransitionKind::ClaimAcquired),
            claim_required_for_mutation: true,
            owner_visibility: OwnershipVisibility::SameSessionOnly,
        };

        assert_eq!(
            snapshot.validate(),
            Err(
                "run_ownership_snapshot.current_claim.run_id must match snapshot.run_id".to_owned()
            )
        );
    }

    #[test]
    fn ownership_error_codes_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&OwnershipErrorCode::ClaimTransferRequired).unwrap(),
            "\"claim_transfer_required\""
        );
    }

    #[test]
    fn exclusive_claim_mode_allows_mutation_but_observer_only_does_not() {
        assert!(RunClaimMode::ExclusiveMutation.allows_mutation());
        assert!(!RunClaimMode::ObserverOnly.allows_mutation());
    }

    #[test]
    fn active_claim_status_allows_renew_release_and_expire() {
        assert!(RunClaimStatus::Active.allows_renew());
        assert!(RunClaimStatus::Active.allows_release());
        assert!(RunClaimStatus::Active.allows_expire());
        assert!(!RunClaimStatus::Active.allows_successor_acquire());
    }

    #[test]
    fn inactive_claim_statuses_allow_successor_acquire() {
        assert!(RunClaimStatus::Expired.allows_successor_acquire());
        assert!(RunClaimStatus::Released.allows_successor_acquire());
        assert!(RunClaimStatus::Superseded.allows_successor_acquire());
    }
}
