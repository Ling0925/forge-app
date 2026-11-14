//! Wall clock + local UTC offset for the state layer (relative times, day
//! buckets, "可在 14:05 后重试"). `forge-state` itself stays clock-free.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch (UTC).
pub fn wall_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Local time zone offset from UTC at `utc_ms`, in ms (UTC+8 → 28 800 000).
pub fn local_utc_offset_ms(utc_ms: u64) -> i64 {
    #[cfg(unix)]
    {
        let secs = (utc_ms / 1000) as libc::time_t;
        // SAFETY: `localtime_r` only writes into the zeroed `tm` we own.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&secs, &mut tm).is_null() {
                return 0;
            }
            tm.tm_gmtoff as i64 * 1000
        }
    }
    #[cfg(not(unix))]
    {
        let _ = utc_ms;
        0
    }
}
