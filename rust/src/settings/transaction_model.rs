//! Pure planning model for one settings transaction; no filesystem operations.
//!
//! This is deliberately not wired into loading, saving, or recovery. Callers
//! must bind an intent to one operation, its paths, and exact expected bytes,
//! and verify observations under appropriate held handles. Explicit user
//! approval is required before any suggested recovery action is performed.
//! `Verified` never means merely that a name exists or that JSON parses.
//! Parents must be trusted
//! and stable. Plans are not permission to act on stale observations: every
//! filesystem operation still needs a no-replace/held-object contract.
//!
//! Markers can lag the actual rename operations after process termination.
//! Decisions therefore inspect artifacts as well as the last recorded stage.
//! This model makes no power-loss durability or filesystem-ordering promise.
//! Original and replacement bytes must differ; ambiguous identity fails closed.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Known vacant, still requiring an exclusive claim by the filesystem layer.
    Vacant,
    /// Includes completed, pending, invalid, and unrecognized material.
    Occupied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapacityPlan {
    /// A candidate only; failed exclusive reservation requires a fresh inventory.
    ClaimExclusively {
        index: usize,
    },
    Full,
}

/// The caller supplies the finite slot limit N and inventories exactly those
/// slots, counting unknown entries as occupied. Successful and failed attempts
/// both consume slots; this model never releases, recycles, or deletes one.
/// N is policy supplied by the integrating application, not a product default.
/// Zero capacity is full. This does not enforce a byte budget or inventory
/// legacy files outside the slots.
pub fn capacity_plan<const N: usize>(slots: &[Slot; N]) -> CapacityPlan {
    match slots.iter().position(|slot| *slot == Slot::Vacant) {
        Some(index) => CapacityPlan::ClaimExclusively { index },
        None => CapacityPlan::Full,
    }
}

/// Last verified marker. Every transition is recorded only after its operation;
/// a crash may leave the preceding marker with the following artifact layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Intent exists; original has not been moved. Replacement may be preparing.
    Reserved,
    /// Complete replacement was verified/flushed before moving the original.
    Prepared,
    /// Original was moved to its unique backup without replacement.
    BackedUp,
    /// Prepared file was published to the vacant target without replacement.
    Published,
    /// Publication was verified and its completion marker recorded.
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Missing,
    Invalid,
    Unreadable,
    /// Caller verified operation identity, paths, distinct expected bytes, and
    /// marker consistency. A stage parsed from arbitrary JSON is insufficient.
    Verified(Stage),
}

/// Observation of a role-specific artifact: backup must match original bytes;
/// prepared must match replacement bytes. Verification includes operation
/// binding and the caller's file-safety checks, not filename inference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Artifact {
    Missing,
    Verified,
    Different,
    Unreadable,
}

/// Target classifications require the same operation binding, exact-byte
/// verification, and file-safety checks as `Artifact::Verified`. A filename or
/// an equal JSON interpretation alone cannot establish either expected role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Missing,
    Original,
    Replacement,
    /// Includes ambiguous classification, directories, and unexpected content.
    Different,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub intent: Intent,
    pub target: Target,
    pub backup: Artifact,
    pub prepared: Artifact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    MissingIntent,
    InvalidIntent,
    UnreadableIntent,
    UnreadableArtifact,
    UnexpectedArtifact,
    /// Includes a foreign target appearing in the destination-name gap.
    TargetConflict,
    /// Recognized pieces do not establish a permitted operation history.
    InconsistentState,
}

/// These are proposed next steps, never authorizations. Acting on any proposal
/// requires explicit user approval and fresh verification under protected held
/// handles. None authorizes an automatic retry, overwriting a destination, or
/// deleting files. This pure model cannot verify approval or filesystem locks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPlan {
    /// No settings file has been moved. A new preparation can be considered.
    PrepareReplacement,
    /// Re-verify/flush the complete prepared file and record Prepared first.
    RecordPrepared,
    /// Move the held original to the vacant backup, without replacement.
    BackUpOriginal,
    /// Backup is verified and target vacant. Re-verify/flush the prepared file,
    /// record BackedUp if necessary, then publish without replacement.
    PublishPrepared,
    /// Target matches replacement, prepared name vanished, backup matches old
    /// bytes. Record missing progress/completion markers; do not write settings.
    RecordCompletion,
    Complete,
    Blocked(BlockReason),
}

/// Fail-closed restart classification for a transaction that started with an
/// existing original. A missing target here is never a default/first-run load.
/// Missing-original initial saves need a separate explicitly modeled protocol.
pub fn restart_plan(snapshot: Snapshot) -> RestartPlan {
    use Artifact::{Missing, Verified};
    use RestartPlan::*;
    let stage = match snapshot.intent {
        Intent::Missing => return Blocked(BlockReason::MissingIntent),
        Intent::Invalid => return Blocked(BlockReason::InvalidIntent),
        Intent::Unreadable => return Blocked(BlockReason::UnreadableIntent),
        Intent::Verified(stage) => stage,
    };
    if snapshot.target == Target::Unreadable
        || snapshot.backup == Artifact::Unreadable
        || snapshot.prepared == Artifact::Unreadable
    {
        return Blocked(BlockReason::UnreadableArtifact);
    }
    if snapshot.target == Target::Different {
        return Blocked(BlockReason::TargetConflict);
    }
    if snapshot.backup == Artifact::Different || snapshot.prepared == Artifact::Different {
        return Blocked(BlockReason::UnexpectedArtifact);
    }
    match (stage, snapshot.target, snapshot.backup, snapshot.prepared) {
        (Stage::Reserved, Target::Original, Missing, Missing) => PrepareReplacement,
        (Stage::Reserved, Target::Original, Missing, Verified) => RecordPrepared,
        (Stage::Prepared, Target::Original, Missing, Verified) => BackUpOriginal,
        (Stage::Prepared | Stage::BackedUp, Target::Missing, Verified, Verified) => PublishPrepared,
        (Stage::BackedUp | Stage::Published, Target::Replacement, Verified, Missing) => {
            RecordCompletion
        }
        (Stage::Complete, Target::Replacement, Verified, Missing) => Complete,
        _ => Blocked(BlockReason::InconsistentState),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(stage: Stage, target: Target, backup: Artifact, prepared: Artifact) -> Snapshot {
        Snapshot {
            intent: Intent::Verified(stage),
            target,
            backup,
            prepared,
        }
    }

    #[test]
    fn process_exit_after_each_operation_preserves_explicit_restart_path() {
        use Artifact::{Missing, Verified};
        let cases = [
            (
                Stage::Reserved,
                Target::Original,
                Missing,
                Missing,
                RestartPlan::PrepareReplacement,
            ),
            (
                Stage::Reserved,
                Target::Original,
                Missing,
                Verified,
                RestartPlan::RecordPrepared,
            ),
            (
                Stage::Prepared,
                Target::Original,
                Missing,
                Verified,
                RestartPlan::BackUpOriginal,
            ),
            (
                Stage::Prepared,
                Target::Missing,
                Verified,
                Verified,
                RestartPlan::PublishPrepared,
            ),
            (
                Stage::BackedUp,
                Target::Missing,
                Verified,
                Verified,
                RestartPlan::PublishPrepared,
            ),
            (
                Stage::BackedUp,
                Target::Replacement,
                Verified,
                Missing,
                RestartPlan::RecordCompletion,
            ),
            (
                Stage::Published,
                Target::Replacement,
                Verified,
                Missing,
                RestartPlan::RecordCompletion,
            ),
            (
                Stage::Complete,
                Target::Replacement,
                Verified,
                Missing,
                RestartPlan::Complete,
            ),
        ];
        for (stage, target, backup, prepared, expected) in cases {
            assert_eq!(
                restart_plan(snapshot(stage, target, backup, prepared)),
                expected
            );
        }
    }

    #[test]
    fn missing_invalid_or_unreadable_intent_never_authorizes_adoption() {
        for (intent, reason) in [
            (Intent::Missing, BlockReason::MissingIntent),
            (Intent::Invalid, BlockReason::InvalidIntent),
            (Intent::Unreadable, BlockReason::UnreadableIntent),
        ] {
            let observed = Snapshot {
                intent,
                target: Target::Missing,
                backup: Artifact::Verified,
                prepared: Artifact::Verified,
            };
            assert_eq!(restart_plan(observed), RestartPlan::Blocked(reason));
        }
    }

    #[test]
    fn competing_target_and_changed_artifacts_are_preserved_by_blocking() {
        let observed = snapshot(
            Stage::BackedUp,
            Target::Different,
            Artifact::Verified,
            Artifact::Verified,
        );
        assert_eq!(
            restart_plan(observed),
            RestartPlan::Blocked(BlockReason::TargetConflict)
        );
        for observed in [
            Snapshot {
                backup: Artifact::Different,
                target: Target::Missing,
                ..observed
            },
            Snapshot {
                prepared: Artifact::Different,
                target: Target::Missing,
                ..observed
            },
        ] {
            assert_eq!(
                restart_plan(observed),
                RestartPlan::Blocked(BlockReason::UnexpectedArtifact)
            );
        }
    }

    #[test]
    fn unreadable_artifacts_are_not_treated_as_missing() {
        let observed = snapshot(
            Stage::Prepared,
            Target::Missing,
            Artifact::Verified,
            Artifact::Verified,
        );
        for observed in [
            Snapshot {
                target: Target::Unreadable,
                ..observed
            },
            Snapshot {
                backup: Artifact::Unreadable,
                ..observed
            },
            Snapshot {
                prepared: Artifact::Unreadable,
                ..observed
            },
        ] {
            assert_eq!(
                restart_plan(observed),
                RestartPlan::Blocked(BlockReason::UnreadableArtifact)
            );
        }
    }

    #[test]
    fn missing_target_without_both_verified_recovery_pieces_is_blocked() {
        for (backup, prepared) in [
            (Artifact::Missing, Artifact::Missing),
            (Artifact::Verified, Artifact::Missing),
            (Artifact::Missing, Artifact::Verified),
        ] {
            assert_eq!(
                restart_plan(snapshot(Stage::BackedUp, Target::Missing, backup, prepared)),
                RestartPlan::Blocked(BlockReason::InconsistentState)
            );
        }
    }

    #[test]
    fn impossible_marker_order_and_duplicate_published_copy_are_blocked() {
        use Artifact::{Missing, Verified};
        for observed in [
            snapshot(Stage::Reserved, Target::Missing, Verified, Verified),
            snapshot(Stage::Prepared, Target::Replacement, Verified, Missing),
            snapshot(Stage::Published, Target::Missing, Verified, Verified),
            snapshot(Stage::Complete, Target::Original, Missing, Verified),
            snapshot(Stage::Published, Target::Replacement, Verified, Verified),
        ] {
            assert_eq!(
                restart_plan(observed),
                RestartPlan::Blocked(BlockReason::InconsistentState)
            );
        }
    }

    #[test]
    fn capacity_is_fixed_and_occupied_slots_are_never_recycled() {
        let mut slots = [Slot::Occupied; 3];
        assert_eq!(capacity_plan(&slots), CapacityPlan::Full);
        slots[2] = Slot::Vacant;
        assert_eq!(
            capacity_plan(&slots),
            CapacityPlan::ClaimExclusively { index: 2 }
        );
        assert_eq!(slots[2], Slot::Vacant);
        slots[2] = Slot::Occupied;
        assert_eq!(capacity_plan(&slots), CapacityPlan::Full);
        assert_eq!(capacity_plan(&[]), CapacityPlan::Full);
    }
}
