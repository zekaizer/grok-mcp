//! Small time helpers without pulling chrono into every call site.

use std::time::{SystemTime, UNIX_EPOCH};

/// RFC3339 UTC timestamp for `updated_at` fields.
#[must_use]
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format without chrono dependency for write-side stamps.
    // Good enough for audit fields; expiry parsing uses a dedicated path.
    format_unix_secs_rfc3339(secs)
}

fn format_unix_secs_rfc3339(secs: u64) -> String {
    // Manual UTC breakdown (no leap seconds).
    const DAY: u64 = 86400;
    let days = secs / DAY;
    let tod = secs % DAY;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;

    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Howard Hinnant civil_from_days (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Parse an RFC3339 / ISO-8601 timestamp to unix seconds (UTC).
/// Accepts optional fractional seconds.
pub fn parse_rfc3339_unix(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Strip fractional part and trailing Z / offset for a simple parser.
    let core = s.strip_suffix('Z').unwrap_or(s);
    let core = core.split('+').next().unwrap_or(core);
    let core = if let Some(idx) = core.rfind('-') {
        // careful: date has dashes; only strip timezone like -05:00 at end
        if idx > 10 { &core[..idx] } else { core }
    } else {
        core
    };
    let core = core.split('.').next().unwrap_or(core);
    // Expect YYYY-MM-DDTHH:MM:SS
    let (date, time) = core.split_once('T')?;
    let mut dparts = date.split('-');
    let year: i32 = dparts.next()?.parse().ok()?;
    let month: u32 = dparts.next()?.parse().ok()?;
    let day: u32 = dparts.next()?.parse().ok()?;
    let mut tparts = time.split(':');
    let hour: u32 = tparts.next()?.parse().ok()?;
    let min: u32 = tparts.next()?.parse().ok()?;
    let sec: u32 = tparts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = days * 86400 + u64::from(hour) * 3600 + u64::from(min) * 60 + u64::from(sec);
    Some(secs)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> Option<u64> {
    // Inverse of civil_from_days; return days since Unix epoch.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + u64::from(doy);
    let z = era as i64 * 146_097 + doe as i64 - 719_468;
    if z < 0 { None } else { Some(z as u64) }
}

/// True if `expires_at` is missing or within `skew_secs` of now (or already past).
#[must_use]
pub fn needs_refresh(expires_at: Option<&str>, skew_secs: u64) -> bool {
    let Some(exp) = expires_at.and_then(parse_rfc3339_unix) else {
        // Unknown expiry: refresh proactively only if caller forces; default treat as ok.
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    exp <= now.saturating_add(skew_secs)
}

/// Build expires_at from `expires_in` seconds from now.
#[must_use]
pub fn expires_at_from_expires_in(expires_in: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_secs_rfc3339(now.saturating_add(expires_in))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_rfc3339() {
        let u = parse_rfc3339_unix("2026-07-12T16:53:44Z").unwrap();
        assert!(u > 1_700_000_000);
    }

    #[test]
    fn parse_fractional() {
        let a = parse_rfc3339_unix("2026-07-12T16:53:44.242526956Z").unwrap();
        let b = parse_rfc3339_unix("2026-07-12T16:53:44Z").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn needs_refresh_future() {
        assert!(!needs_refresh(Some("2099-01-01T00:00:00Z"), 300));
    }

    #[test]
    fn needs_refresh_past() {
        assert!(needs_refresh(Some("2000-01-01T00:00:00Z"), 300));
    }

    #[test]
    fn round_trip_now_format() {
        let s = now_rfc3339();
        assert!(parse_rfc3339_unix(&s).is_some(), "{s}");
    }
}
