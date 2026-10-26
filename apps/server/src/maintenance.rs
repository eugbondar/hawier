//! Maintenance windows (hawkBit-compatible): a Quartz-style cron schedule
//! of window openings, a `HH:mm:ss` duration and a fixed `±hh:mm` UTC
//! offset. Devices may download any time but should only APPLY an update
//! while a window is open; DDI reports `maintenanceWindow:
//! available|unavailable` on the deployment.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use std::str::FromStr;

use crate::error::{AppError, AppResult};

/// Evaluated window state for an action at a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub available: bool,
    /// Epoch millis of the next window opening (None if the schedule never
    /// fires again).
    pub next_start: Option<i64>,
}

fn parse_offset(timezone: &str) -> AppResult<FixedOffset> {
    let raw = timezone.trim();
    let (sign, rest) = match raw.as_bytes().first() {
        Some(b'+') => (1, &raw[1..]),
        Some(b'-') => (-1, &raw[1..]),
        _ => {
            return Err(AppError::BadRequest(format!(
                "maintenance timezone must be a ±hh:mm offset, got '{timezone}'"
            )))
        }
    };
    let (hours, minutes) = rest.split_once(':').ok_or_else(|| {
        AppError::BadRequest(format!(
            "maintenance timezone must be a ±hh:mm offset, got '{timezone}'"
        ))
    })?;
    let hours: i32 = hours
        .parse()
        .map_err(|_| AppError::BadRequest(format!("invalid timezone hours '{timezone}'")))?;
    let minutes: i32 = minutes
        .parse()
        .map_err(|_| AppError::BadRequest(format!("invalid timezone minutes '{timezone}'")))?;
    let seconds = sign * (hours * 3600 + minutes * 60);
    FixedOffset::east_opt(seconds)
        .ok_or_else(|| AppError::BadRequest(format!("timezone offset out of range '{timezone}'")))
}

fn parse_duration_millis(duration: &str) -> AppResult<i64> {
    crate::tenant::parse_polling_millis(duration)
        .filter(|d| *d > 0)
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "maintenance duration must be HH:mm:ss, got '{duration}'"
            ))
        })
}

/// Shared with retention rules, which use the same Quartz-style cron so an
/// operator learns one syntax rather than two.
pub fn parse_schedule(schedule: &str) -> AppResult<cron::Schedule> {
    cron::Schedule::from_str(schedule.trim()).map_err(|error| {
        AppError::BadRequest(format!("invalid cron schedule '{schedule}': {error}"))
    })
}

/// Validate the (schedule, duration, timezone) triple like hawkBit does:
/// all empty (no window) or all present and parseable with a future window.
pub fn validate(
    schedule: Option<&str>,
    duration: Option<&str>,
    timezone: Option<&str>,
) -> AppResult<()> {
    let all_empty = [schedule, duration, timezone]
        .iter()
        .all(|v| v.is_none_or(|v| v.trim().is_empty()));
    if all_empty {
        return Ok(());
    }
    let (Some(schedule), Some(duration), Some(timezone)) = (schedule, duration, timezone) else {
        return Err(AppError::BadRequest(
            "maintenance schedule, duration and timezone must all be set together".into(),
        ));
    };
    let state = window_state(schedule, duration, timezone, crate::state::now_millis())?;
    if !state.available && state.next_start.is_none() {
        return Err(AppError::BadRequest(
            "maintenance schedule has no upcoming window".into(),
        ));
    }
    Ok(())
}

/// Evaluate the window state at `now_millis`.
pub fn window_state(
    schedule: &str,
    duration: &str,
    timezone: &str,
    now_millis: i64,
) -> AppResult<WindowState> {
    let offset = parse_offset(timezone)?;
    let duration_ms = parse_duration_millis(duration)?;
    let schedule = parse_schedule(schedule)?;

    let now_utc: DateTime<Utc> = Utc
        .timestamp_millis_opt(now_millis)
        .single()
        .ok_or_else(|| AppError::BadRequest("timestamp out of range".into()))?;
    let now_local = now_utc.with_timezone(&offset);

    // A window opened at fire-time F is open while F <= now < F+duration:
    // look for fires in the last `duration` window.
    let horizon = now_local - chrono::Duration::milliseconds(duration_ms);
    let available = schedule
        .after(&horizon)
        .take_while(|fire| *fire <= now_local)
        .any(|fire| now_local < fire + chrono::Duration::milliseconds(duration_ms));

    let next_start = schedule
        .after(&now_local)
        .next()
        .map(|fire| fire.timestamp_millis());

    Ok(WindowState {
        available,
        next_start,
    })
}

/// Window state for an action row (None = the action has no window).
pub fn action_window_state(
    schedule: &Option<String>,
    duration: &Option<String>,
    timezone: &Option<String>,
    now_millis: i64,
) -> Option<WindowState> {
    let (schedule, duration, timezone) = match (schedule, duration, timezone) {
        (Some(s), Some(d), Some(t)) if !s.trim().is_empty() => (s, d, t),
        _ => return None,
    };
    window_state(schedule, duration, timezone, now_millis).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn window_open_every_minute_long_duration() {
        // Fires at every minute boundary, each window lasts 10 minutes —
        // any point in time is inside some window.
        let state = window_state("0 * * * * *", "00:10:00", "+00:00", 1_700_000_123_456).unwrap();
        assert!(state.available);
        assert!(state.next_start.is_some());
    }

    #[test]
    fn window_closed_between_short_windows() {
        // One-second windows at each minute boundary: pick a time 30s past
        // the boundary → closed, next start within 30s.
        let boundary = 1_700_000_040_000; // divisible by 60_000
        assert_eq!(boundary % MIN, 0);
        let now = boundary + 30_000;
        let state = window_state("0 * * * * *", "00:00:01", "+00:00", now).unwrap();
        assert!(!state.available);
        let next = state.next_start.unwrap();
        assert!(next > now && next <= now + 30_000, "next={next} now={now}");
    }

    #[test]
    fn window_respects_utc_offset() {
        // Daily window at 03:00 LOCAL (+02:00) == 01:00 UTC, 1h duration.
        // 1_700_000_123_456 is 2023-11-14T22:15:23Z → local 00:15 → closed.
        let closed = window_state("0 0 3 * * *", "01:00:00", "+02:00", 1_700_000_123_456).unwrap();
        assert!(!closed.available);
        // 01:30 UTC == 03:30 local → inside the window.
        let inside_utc = 1_700_011_800_000; // 2023-11-15T01:30:00Z
        let open = window_state("0 0 3 * * *", "01:00:00", "+02:00", inside_utc).unwrap();
        assert!(open.available);
    }

    #[test]
    fn validation_rules() {
        assert!(validate(None, None, None).is_ok());
        assert!(validate(Some("0 * * * * *"), Some("00:10:00"), Some("+00:00")).is_ok());
        assert!(validate(Some("0 * * * * *"), None, None).is_err());
        assert!(validate(Some("garbage"), Some("00:10:00"), Some("+00:00")).is_err());
        assert!(validate(Some("0 * * * * *"), Some("bad"), Some("+00:00")).is_err());
        assert!(validate(Some("0 * * * * *"), Some("00:10:00"), Some("CET")).is_err());
    }
}
