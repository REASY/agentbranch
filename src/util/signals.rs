use crate::error::AppError;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub fn install_interrupt_flag() -> Result<Arc<AtomicBool>, AppError> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let clone = Arc::clone(&interrupted);
    ctrlc::set_handler(move || {
        clone.store(true, Ordering::SeqCst);
    })
    .map_err(|err| AppError::Blocked(format!("failed to install interrupt handler: {err}")))?;
    Ok(interrupted)
}
