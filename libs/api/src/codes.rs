//! hawkBit-compatible integer codes for DB enum columns.
//!
//! NARROW: these are the exact hand-maintained maps from hawkBit's JPA
//! `AttributeConverter`s (NOT declaration-order ordinals; `RolloutStatus`
//! has gaps at 7 and 8). The DB stores bare INTEGERs with no CHECK
//! constraints — kept byte-compatible with hawkBit 1.0.0 for the dev
//! stage. Pre-production hawier should move to PG enums or lookup tables
//! (see SCHEMA_NOTES.md).

use thiserror::Error;

#[derive(Debug, Error)]
#[error("unknown {kind} code {code}")]
pub struct UnknownCode {
    pub kind: &'static str,
    pub code: i32,
}

macro_rules! int_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $code:expr),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(i32)]
        pub enum $name {
            $($variant = $code),+
        }

        impl $name {
            pub const fn code(self) -> i32 {
                self as i32
            }
        }

        impl TryFrom<i32> for $name {
            type Error = UnknownCode;

            fn try_from(code: i32) -> Result<Self, UnknownCode> {
                match code {
                    $($code => Ok(Self::$variant),)+
                    _ => Err(UnknownCode { kind: stringify!($name), code }),
                }
            }
        }

        impl From<$name> for i32 {
            fn from(v: $name) -> i32 {
                v.code()
            }
        }
    };
}

int_enum! {
    /// `sp_action.status` / `sp_action_status.status` (hawkBit `Action.Status`).
    ActionStatus {
        Finished = 0,
        Error = 1,
        Warning = 2,
        Running = 3,
        Canceled = 4,
        Canceling = 5,
        Retrieved = 6,
        Download = 7,
        Scheduled = 8,
        CancelRejected = 9,
        Downloaded = 10,
        WaitForConfirmation = 11,
    }
}

impl ActionStatus {
    /// Java enum constant name, as hawkBit prints it (DDI `actionHistory.status`).
    pub const fn name(self) -> &'static str {
        match self {
            ActionStatus::Finished => "FINISHED",
            ActionStatus::Error => "ERROR",
            ActionStatus::Warning => "WARNING",
            ActionStatus::Running => "RUNNING",
            ActionStatus::Canceled => "CANCELED",
            ActionStatus::Canceling => "CANCELING",
            ActionStatus::Retrieved => "RETRIEVED",
            ActionStatus::Download => "DOWNLOAD",
            ActionStatus::Scheduled => "SCHEDULED",
            ActionStatus::CancelRejected => "CANCEL_REJECTED",
            ActionStatus::Downloaded => "DOWNLOADED",
            ActionStatus::WaitForConfirmation => "WAIT_FOR_CONFIRMATION",
        }
    }

    /// Inverse of [`name`](Self::name) (`Status.valueOf` in hawkBit — used by
    /// the `action.cleanup.auto.status` tenant config).
    pub fn from_name(name: &str) -> Option<Self> {
        const ALL: [ActionStatus; 12] = [
            ActionStatus::Finished,
            ActionStatus::Error,
            ActionStatus::Warning,
            ActionStatus::Running,
            ActionStatus::Canceled,
            ActionStatus::Canceling,
            ActionStatus::Retrieved,
            ActionStatus::Download,
            ActionStatus::Scheduled,
            ActionStatus::CancelRejected,
            ActionStatus::Downloaded,
            ActionStatus::WaitForConfirmation,
        ];
        ALL.into_iter().find(|s| s.name() == name)
    }
}

int_enum! {
    /// `sp_action.action_type` / `sp_rollout.action_type` /
    /// `sp_target_filter_query.auto_assign_action_type` (hawkBit `ActionType`).
    ActionType {
        Forced = 0,
        Soft = 1,
        TimeForced = 2,
        DownloadOnly = 3,
    }
}

int_enum! {
    /// `sp_rollout.status` (hawkBit `RolloutStatus`; codes 7 and 8 are holes).
    RolloutStatus {
        Creating = 0,
        Ready = 1,
        Paused = 2,
        Starting = 3,
        Stopped = 4,
        Running = 5,
        Finished = 6,
        Deleting = 9,
        Deleted = 10,
        WaitingForApproval = 11,
        ApprovalDenied = 12,
        Stopping = 13,
    }
}

int_enum! {
    /// `sp_rollout_group.status` (hawkBit `RolloutGroupStatus`).
    RolloutGroupStatus {
        Ready = 0,
        Scheduled = 1,
        Finished = 2,
        Error = 3,
        Running = 4,
        Creating = 5,
    }
}

int_enum! {
    /// `sp_target.update_status` (hawkBit `TargetUpdateStatus`).
    TargetUpdateStatus {
        Unknown = 0,
        InSync = 1,
        Pending = 2,
        Error = 3,
        Registered = 4,
    }
}

// The four rollout-group condition/action columns use raw JPA ordinals
// (no converter in hawkBit) — modeled as plain consts to make that visible.
pub mod rollout_group_ordinals {
    /// `sp_rollout_group.success_condition`: THRESHOLD is the only variant.
    pub const SUCCESS_CONDITION_THRESHOLD: i32 = 0;
    /// `sp_rollout_group.error_condition`: THRESHOLD is the only variant.
    pub const ERROR_CONDITION_THRESHOLD: i32 = 0;
    /// `sp_rollout_group.error_action`: PAUSE is the only variant.
    pub const ERROR_ACTION_PAUSE: i32 = 0;
    /// `sp_rollout_group.success_action`.
    pub const SUCCESS_ACTION_NEXTGROUP: i32 = 0;
    pub const SUCCESS_ACTION_PAUSE: i32 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_status_codes_match_hawkbit_converter() {
        assert_eq!(ActionStatus::Finished.code(), 0);
        assert_eq!(ActionStatus::CancelRejected.code(), 9);
        assert_eq!(ActionStatus::Downloaded.code(), 10);
        assert_eq!(ActionStatus::WaitForConfirmation.code(), 11);
        assert_eq!(ActionStatus::try_from(6).unwrap(), ActionStatus::Retrieved);
    }

    #[test]
    fn rollout_status_has_holes_at_7_and_8() {
        assert!(RolloutStatus::try_from(7).is_err());
        assert!(RolloutStatus::try_from(8).is_err());
        assert_eq!(RolloutStatus::Deleting.code(), 9);
        assert_eq!(RolloutStatus::Stopping.code(), 13);
    }

    #[test]
    fn target_update_status_roundtrip() {
        for code in 0..=4 {
            assert_eq!(TargetUpdateStatus::try_from(code).unwrap().code(), code);
        }
    }
}
