use crate::db::models::LifecycleState;

pub fn transition_after_open() -> LifecycleState {
    LifecycleState::Running
}

pub fn transition_after_stop() -> LifecycleState {
    LifecycleState::Stopped
}

pub fn transition_after_start() -> LifecycleState {
    LifecycleState::Running
}
