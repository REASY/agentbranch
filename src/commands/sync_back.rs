use crate::cli::SyncBackArgs;
use crate::error::AppError;

pub fn run(args: SyncBackArgs) -> Result<(), AppError> {
    crate::sync::run_git_native(args)
}
