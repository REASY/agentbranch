use crate::error::AppError;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

static INTERRUPTED: OnceLock<Arc<AtomicBool>> = OnceLock::new();

pub fn install_interrupt_flag() -> Result<Arc<AtomicBool>, AppError> {
    if let Some(interrupted) = INTERRUPTED.get() {
        return Ok(Arc::clone(interrupted));
    }

    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&interrupted);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::SeqCst))
        .map_err(|err| AppError::Blocked(format!("failed to install interrupt handler: {err}")))?;
    let _ = INTERRUPTED.set(Arc::clone(&interrupted));
    Ok(interrupted)
}

pub fn take_interrupt() -> bool {
    INTERRUPTED
        .get()
        .is_some_and(|interrupted| interrupted.swap(false, Ordering::SeqCst))
}
