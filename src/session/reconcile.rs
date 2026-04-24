use crate::db::models::LifecycleState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairAction {
    Reclone,
    Reseed,
    Restart,
    Restage,
    RollbackOrResume,
    FinishDestroy,
    ResumeRepair,
    InspectLastTransition,
    RebuildBase,
    Noop,
}

impl RepairAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RepairAction::Reclone => "reclone",
            RepairAction::Reseed => "reseed",
            RepairAction::Restart => "restart",
            RepairAction::Restage => "restage",
            RepairAction::RollbackOrResume => "rollback_or_resume",
            RepairAction::FinishDestroy => "finish_destroy",
            RepairAction::ResumeRepair => "resume_repair",
            RepairAction::InspectLastTransition => "inspect_last_transition",
            RepairAction::RebuildBase => "rebuild_base",
            RepairAction::Noop => "noop",
        }
    }
}

pub fn repair_action_for_state(state: LifecycleState) -> RepairAction {
    match state {
        LifecycleState::Cloning => RepairAction::Reclone,
        LifecycleState::Seeding => RepairAction::Reseed,
        LifecycleState::Starting => RepairAction::Restart,
        LifecycleState::Syncing | LifecycleState::Staging => RepairAction::Restage,
        LifecycleState::Applying => RepairAction::RollbackOrResume,
        LifecycleState::Destroying => RepairAction::FinishDestroy,
        LifecycleState::Repairing => RepairAction::ResumeRepair,
        LifecycleState::Error => RepairAction::InspectLastTransition,
        LifecycleState::PreparingBase => RepairAction::RebuildBase,
        LifecycleState::Running | LifecycleState::Stopped | LifecycleState::Closed => {
            RepairAction::Noop
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destroying_always_finishes_forward() {
        assert_eq!(
            repair_action_for_state(LifecycleState::Destroying),
            RepairAction::FinishDestroy
        );
    }

    #[test]
    fn cloning_requests_reclone() {
        assert_eq!(
            repair_action_for_state(LifecycleState::Cloning),
            RepairAction::Reclone
        );
    }

    #[test]
    fn applying_requires_manual_rollback_or_resume() {
        assert_eq!(
            repair_action_for_state(LifecycleState::Applying),
            RepairAction::RollbackOrResume
        );
    }

    #[test]
    fn running_is_a_noop() {
        assert_eq!(
            repair_action_for_state(LifecycleState::Running),
            RepairAction::Noop
        );
    }
}
