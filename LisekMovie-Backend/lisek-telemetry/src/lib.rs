//! Playback telemetry: the progress reports players send while something
//! plays, what is valid in them, and what they change.
//!
//! A report carries the position (for resume and "watched"), the player
//! state (for counting viewing time) and a few quality counters (stalls,
//! dropped frames, bandwidth) that show how well a play method works on a
//! client. Persistence lives in `lisek-db`; this crate is pure so the
//! rules are testable without a database.

pub mod client;
pub mod duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Why the player sent a report. Heartbeats arrive on a timer; the
/// others when the state changes, so the stored position is never more
/// than one heartbeat behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProgressEvent {
    #[default]
    Heartbeat,
    Play,
    Pause,
    Seek,
    /// Played to the end: the item counts as watched.
    Ended,
    /// The viewer left (closed the player, switched tracks, page hidden).
    Stop,
}

/// One progress report (`POST /api/telemetry/playback-progress`).
#[derive(Debug, Clone, PartialEq, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaybackProgress {
    pub session_id: Uuid,
    pub position_seconds: f64,
    pub duration_seconds: f64,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub event: ProgressEvent,
    /// Rebuffering since the previous report (not counting seeks).
    #[serde(default)]
    pub stalls: u32,
    #[serde(default)]
    pub stall_seconds: f64,
    /// Totals since playback started (`getVideoPlaybackQuality()`).
    pub dropped_frames: Option<u32>,
    /// The player's current bandwidth estimate.
    pub bandwidth_kbps: Option<u32>,
    /// Media buffered ahead of the position.
    pub buffered_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProgressError {
    #[error("position or duration is not a finite, non-negative number")]
    InvalidTime,
    #[error("duration is longer than a week")]
    DurationTooLong,
    #[error("position is past the end")]
    PositionPastEnd,
    #[error("quality counters are out of range")]
    InvalidQuality,
}

impl ProgressError {
    pub fn code(self) -> &'static str {
        match self {
            ProgressError::InvalidTime
            | ProgressError::DurationTooLong
            | ProgressError::PositionPastEnd => "invalid_progress",
            ProgressError::InvalidQuality => "invalid_quality",
        }
    }
}

/// Longest gap between two reports that still counts as watching. A
/// player reports every [`next_report_seconds`] while playing; a longer
/// gap (laptop asleep, network down) must not count as viewing time.
pub const MAX_COUNTED_GAP_SECONDS: f64 = 35.0;

const MAX_DURATION_SECONDS: f64 = 7.0 * 24.0 * 3600.0;

impl PlaybackProgress {
    pub fn validate(&self) -> Result<(), ProgressError> {
        let finite = |v: f64| v.is_finite() && v >= 0.0;
        if !finite(self.position_seconds) || !finite(self.duration_seconds) {
            return Err(ProgressError::InvalidTime);
        }
        if self.duration_seconds > MAX_DURATION_SECONDS {
            return Err(ProgressError::DurationTooLong);
        }
        // Players report a position slightly past a stream's nominal end.
        if self.duration_seconds > 0.0 && self.position_seconds > self.duration_seconds + 1.0 {
            return Err(ProgressError::PositionPastEnd);
        }
        let stall_ok =
            finite(self.stall_seconds) && self.stall_seconds <= 3600.0 && self.stalls <= 10_000;
        let buffer_ok = self
            .buffered_seconds
            .is_none_or(|b| finite(b) && b <= 86_400.0);
        if !stall_ok || !buffer_ok || self.bandwidth_kbps.is_some_and(|b| b > 10_000_000) {
            return Err(ProgressError::InvalidQuality);
        }
        Ok(())
    }

    /// The report closes the playback session.
    pub fn ends_session(&self) -> bool {
        matches!(self.event, ProgressEvent::Ended | ProgressEvent::Stop)
    }

    /// The viewer reached the end, whatever the resume thresholds say.
    pub fn played_to_end(&self) -> bool {
        self.event == ProgressEvent::Ended
    }

    /// The position clamped into the duration (players overshoot a bit).
    pub fn clamped_position(&self) -> f64 {
        if self.duration_seconds > 0.0 {
            self.position_seconds.min(self.duration_seconds)
        } else {
            self.position_seconds
        }
    }

    /// The player is paused or gone: elapsed time since the last report is
    /// not viewing time.
    pub fn counts_as_watching(&self) -> bool {
        !self.paused && !matches!(self.event, ProgressEvent::Pause | ProgressEvent::Stop)
    }
}

/// When the client should report next: often enough that resume loses at
/// most a few seconds, rarely while nothing changes.
pub fn next_report_seconds(paused: bool) -> u32 {
    if paused { 30 } else { 15 }
}

/// What the server answers a report with.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ProgressAck {
    /// Where playback resumes next time (0 = from the start).
    pub resume_position_seconds: f64,
    /// The item now counts as watched.
    pub watched: bool,
    pub next_report_seconds: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> PlaybackProgress {
        PlaybackProgress {
            session_id: Uuid::nil(),
            position_seconds: 2700.0,
            duration_seconds: 6000.0,
            paused: false,
            event: ProgressEvent::Heartbeat,
            stalls: 0,
            stall_seconds: 0.0,
            dropped_frames: Some(3),
            bandwidth_kbps: Some(25_000),
            buffered_seconds: Some(28.5),
        }
    }

    #[test]
    fn accepts_a_normal_heartbeat_and_parses_minimal_json() {
        assert_eq!(report().validate(), Ok(()));
        let minimal: PlaybackProgress = serde_json::from_str(
            r#"{"session_id":"00000000-0000-0000-0000-000000000000","position_seconds":12.5,"duration_seconds":100}"#,
        )
        .unwrap();
        assert_eq!(minimal.event, ProgressEvent::Heartbeat);
        assert!(minimal.counts_as_watching());
        assert!(serde_json::from_str::<PlaybackProgress>(r#"{"session_id":"00000000-0000-0000-0000-000000000000","position_seconds":1,"duration_seconds":2,"extra":1}"#).is_err());
    }

    #[test]
    fn rejects_impossible_times_and_counters() {
        let bad = |change: fn(&mut PlaybackProgress)| {
            let mut r = report();
            change(&mut r);
            r.validate()
        };
        assert_eq!(
            bad(|r| r.position_seconds = f64::NAN),
            Err(ProgressError::InvalidTime)
        );
        assert_eq!(
            bad(|r| r.duration_seconds = -1.0),
            Err(ProgressError::InvalidTime)
        );
        assert_eq!(
            bad(|r| r.duration_seconds = 700_000.0),
            Err(ProgressError::DurationTooLong)
        );
        assert_eq!(
            bad(|r| r.position_seconds = 6002.0),
            Err(ProgressError::PositionPastEnd)
        );
        assert_eq!(
            bad(|r| r.stall_seconds = f64::INFINITY),
            Err(ProgressError::InvalidQuality)
        );
        assert_eq!(
            bad(|r| r.buffered_seconds = Some(-1.0)),
            Err(ProgressError::InvalidQuality)
        );
        // A live-like stream without a known duration is fine.
        assert_eq!(bad(|r| r.duration_seconds = 0.0), Ok(()));
    }

    #[test]
    fn events_decide_session_end_and_viewing_time() {
        let mut r = report();
        r.position_seconds = 6000.5;
        assert_eq!(r.clamped_position(), 6000.0);
        r.event = ProgressEvent::Ended;
        assert!(r.ends_session() && r.played_to_end());
        r.event = ProgressEvent::Stop;
        assert!(r.ends_session() && !r.played_to_end() && !r.counts_as_watching());
        r.event = ProgressEvent::Pause;
        assert!(!r.counts_as_watching());
        assert_eq!(
            (next_report_seconds(false), next_report_seconds(true)),
            (15, 30)
        );
        assert!(MAX_COUNTED_GAP_SECONDS > next_report_seconds(true) as f64);
    }
}
