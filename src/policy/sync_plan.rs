#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncBlockReason {
    GuestDirty,
    GuestNotOnReviewBranch,
    ReviewBranchDiverged,
    SessionHeadRewritten,
}

pub fn detect_dirty_sync_block(guest_dirty: bool) -> Vec<SyncBlockReason> {
    if guest_dirty {
        vec![SyncBlockReason::GuestDirty]
    } else {
        Vec::new()
    }
}

pub fn blocked_reason_summary(reasons: &[SyncBlockReason]) -> String {
    if reasons == [SyncBlockReason::GuestDirty] {
        return "guest worktree has uncommitted changes".to_owned();
    }
    reasons
        .iter()
        .map(|reason| match reason {
            SyncBlockReason::GuestDirty => "guest worktree has uncommitted changes",
            SyncBlockReason::GuestNotOnReviewBranch => {
                "guest HEAD is not on the session review branch"
            }
            SyncBlockReason::ReviewBranchDiverged => "review branch diverged on host",
            SyncBlockReason::SessionHeadRewritten => "guest rewrote already imported history",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_guest_repo_blocks_git_native_sync() {
        let reasons = detect_dirty_sync_block(true);
        assert_eq!(
            blocked_reason_summary(&reasons),
            "guest worktree has uncommitted changes"
        );
    }

    #[test]
    fn guest_head_mismatch_has_specific_block_reason() {
        let summary = blocked_reason_summary(&[SyncBlockReason::GuestNotOnReviewBranch]);
        assert_eq!(summary, "guest HEAD is not on the session review branch");
    }
}
