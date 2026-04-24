use crate::types::Timestamp;

pub fn utc_now() -> Timestamp {
    Timestamp::now_utc()
}
