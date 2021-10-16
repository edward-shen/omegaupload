use std::time::Duration;

use lazy_static::lazy_static;

lazy_static! {
    pub static ref FIVE_MINUTES: Duration = Duration::from_secs(5 * 60);
    pub static ref TEN_MINUTES: Duration = Duration::from_secs(5 * 60);
    pub static ref ONE_HOUR: Duration = Duration::from_secs(5 * 60);
    pub static ref ONE_DAY: Duration = Duration::from_secs(5 * 60);
}
