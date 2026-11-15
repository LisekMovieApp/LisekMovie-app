//! Local clock rules: fixed start times, weekdays, pad boundaries.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

/// How late a `strict` slot may start and still count as on time (the
/// previous entry ends a few milliseconds after the minute).
pub const STRICT_GRACE: Duration = Duration::seconds(60);
/// How late a `flexible` slot may start.
pub const FLEXIBLE_WINDOW: Duration = Duration::minutes(30);

/// An IANA zone name, if it is one.
pub fn parse_zone(name: &str) -> Option<Tz> {
    name.parse::<Tz>().ok()
}

/// Whether the weekday of `at` (local time) is in the Monday-first mask.
pub fn day_allowed(mask: u8, at: DateTime<Utc>, tz: Tz) -> bool {
    mask & (1 << at.with_timezone(&tz).weekday().num_days_from_monday()) != 0
}

/// A local date and time as UTC. A time that does not exist (the hour a
/// clock jumps over) moves forward to the first one that does; a time that
/// happens twice takes the first.
fn local_to_utc(tz: Tz, naive: NaiveDateTime) -> DateTime<Utc> {
    let mut candidate = naive;
    for _ in 0..4 {
        match tz.from_local_datetime(&candidate) {
            LocalResult::Single(t) => return t.with_timezone(&Utc),
            LocalResult::Ambiguous(first, _) => return first.with_timezone(&Utc),
            LocalResult::None => candidate += Duration::minutes(30),
        }
    }
    Utc.from_utc_datetime(&naive)
}

/// When a slot with a fixed `start` (local time of day) starts, given the
/// previous entry ended at `time`: the first allowed day's occurrence at or
/// after `time`, or `time` itself when the slot is only a little late
/// (`STRICT_GRACE`, or `FLEXIBLE_WINDOW` for a flexible slot).
pub fn next_fixed(time: DateTime<Utc>, start: NaiveTime, days: u8, tz: Tz, flexible: bool) -> DateTime<Utc> {
    let tolerance = if flexible { FLEXIBLE_WINDOW } else { STRICT_GRACE };
    let today: NaiveDate = time.with_timezone(&tz).date_naive();
    for offset in 0..=8 {
        let date = today + Duration::days(offset);
        if days & (1 << date.weekday().num_days_from_monday()) == 0 {
            continue;
        }
        let occurrence = local_to_utc(tz, date.and_time(start));
        if occurrence >= time {
            return occurrence;
        }
        if time - occurrence <= tolerance {
            return time;
        }
    }
    time
}

/// The first occurrence of local time `start` on an allowed day at or after
/// `time`, with no tolerance (the end of a flood, not a start).
pub fn next_occurrence(time: DateTime<Utc>, start: NaiveTime, days: u8, tz: Tz) -> DateTime<Utc> {
    let today = time.with_timezone(&tz).date_naive();
    for offset in 0..=8 {
        let date = today + Duration::days(offset);
        if days & (1 << date.weekday().num_days_from_monday()) == 0 {
            continue;
        }
        let occurrence = local_to_utc(tz, date.and_time(start));
        if occurrence >= time {
            return occurrence;
        }
    }
    time
}

/// The latest occurrence of local time `start` on an allowed day at or
/// before `time` (looking back a week).
pub fn prev_occurrence(time: DateTime<Utc>, start: NaiveTime, days: u8, tz: Tz) -> Option<DateTime<Utc>> {
    let today = time.with_timezone(&tz).date_naive();
    (0..=7).find_map(|back| {
        let date = today - Duration::days(back);
        if days & (1 << date.weekday().num_days_from_monday()) == 0 {
            return None;
        }
        let occurrence = local_to_utc(tz, date.and_time(start));
        (occurrence <= time).then_some(occurrence)
    })
}

/// Midnight (local) after `time`.
pub fn next_midnight(time: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let tomorrow = time.with_timezone(&tz).date_naive() + Duration::days(1);
    local_to_utc(tz, tomorrow.and_time(NaiveTime::MIN))
}

/// The next full `minutes` of the local clock counted from local midnight,
/// strictly after `time`, or `time` when it already is one (within a
/// second).
pub fn next_boundary(time: DateTime<Utc>, minutes: u32, tz: Tz) -> DateTime<Utc> {
    let local = time.with_timezone(&tz).naive_local();
    let midnight = local.date().and_time(NaiveTime::MIN);
    let elapsed = (local - midnight).num_milliseconds().max(0);
    let step = i64::from(minutes.max(1)) * 60_000;
    let past = elapsed % step;
    if past < 1000 {
        return time;
    }
    time + Duration::milliseconds(step - past)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_fixed_start_is_today_or_the_next_allowed_day() {
        let tz: Tz = "Europe/Warsaw".parse().unwrap();
        let start = NaiveTime::from_hms_opt(20, 0, 0).unwrap();
        // 2026-11-16 is a Monday, 19:00 local (winter, UTC+1).
        let time = utc("2026-11-16T18:00:00Z");
        assert_eq!(next_fixed(time, start, 127, tz, false), utc("2026-11-16T19:00:00Z"));
        // Only Wednesday (bit 2): two days later.
        assert_eq!(next_fixed(time, start, 1 << 2, tz, false), utc("2026-11-18T19:00:00Z"));
    }

    #[test]
    fn late_starts_depend_on_the_mode() {
        let tz = Tz::UTC;
        let start = NaiveTime::from_hms_opt(20, 0, 0).unwrap();
        let ten_seconds_late = utc("2026-11-16T20:00:10Z");
        assert_eq!(next_fixed(ten_seconds_late, start, 127, tz, false), ten_seconds_late);
        let ten_minutes_late = utc("2026-11-16T20:10:00Z");
        assert_eq!(next_fixed(ten_minutes_late, start, 127, tz, false), utc("2026-11-17T20:00:00Z"));
        assert_eq!(next_fixed(ten_minutes_late, start, 127, tz, true), ten_minutes_late);
        let hour_late = utc("2026-11-16T21:00:00Z");
        assert_eq!(next_fixed(hour_late, start, 127, tz, true), utc("2026-11-17T20:00:00Z"));
    }

    #[test]
    fn a_time_the_clock_skips_moves_forward() {
        let tz: Tz = "Europe/Warsaw".parse().unwrap();
        // 2026-03-29: 02:00 -> 03:00. 02:30 does not exist.
        let start = NaiveTime::from_hms_opt(2, 30, 0).unwrap();
        let at = next_fixed(utc("2026-03-28T23:30:00Z"), start, 127, tz, false);
        assert_eq!(at, utc("2026-03-29T01:00:00Z"), "03:00 local, the first valid time after the gap");
        // The night the clock goes back, 02:30 happens twice: the first.
        let at = next_fixed(utc("2026-10-24T23:30:00Z"), start, 127, tz, false);
        assert_eq!(at, utc("2026-10-25T00:30:00Z"));
    }

    #[test]
    fn weekdays_use_the_local_day() {
        let tz: Tz = "Pacific/Auckland".parse().unwrap();
        // 2026-11-15 23:00 UTC is already Monday 2026-11-16 in Auckland.
        assert!(day_allowed(1, utc("2026-11-15T23:00:00Z"), tz));
        assert!(!day_allowed(1, utc("2026-11-15T23:00:00Z"), Tz::UTC), "Sunday in UTC");
    }

    #[test]
    fn pad_boundaries_follow_the_local_clock() {
        let tz: Tz = "Europe/Warsaw".parse().unwrap();
        let t = utc("2026-11-16T10:12:30Z");
        assert_eq!(next_boundary(t, 30, tz), utc("2026-11-16T10:30:00Z"));
        assert_eq!(next_boundary(t, 60, tz), utc("2026-11-16T11:00:00Z"));
        let on = utc("2026-11-16T10:30:00Z");
        assert_eq!(next_boundary(on, 30, tz), on);
        // A zone with a 45 minute offset: boundaries follow local midnight.
        let nepal: Tz = "Asia/Kathmandu".parse().unwrap();
        assert_eq!(next_boundary(utc("2026-11-16T10:00:00Z"), 60, nepal), utc("2026-11-16T10:15:00Z"));
    }
}
