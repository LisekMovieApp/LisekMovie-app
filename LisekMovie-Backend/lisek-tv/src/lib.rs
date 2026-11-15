//! Faza 26: the TV schedule engine.
//!
//! A channel has an ordered list of schedule slots (what to play, in which
//! order, how much, at which local time). [`build`] turns them into a
//! timeline of [`PlannedItem`]s up to a horizon and returns the
//! [`EngineState`] to continue from. It is a pure function: no database,
//! no clock, no randomness except a seeded generator, so the same input
//! always gives the same timeline (a rebuild after a restart or an edit
//! never reshuffles what was already decided) and every rule is testable
//! without a server.
//!
//! The model is the "classic" ErsatzTV schedule reduced to what Lisek
//! needs: slot modes One / Multiple / Duration / Flood, an optional fixed
//! start time with weekdays, four orders, and ad/bumper fillers
//! (pre, post, tail, pad). See `docs` Etap 0 for the comparison.

mod engine;
mod enumerator;
mod rng;
mod time;
mod types;

pub use engine::{build, restart_state};
pub use enumerator::{EnumState, Enumerator, candidates_fingerprint};
pub use time::{day_allowed, next_boundary, next_fixed, next_midnight, next_occurrence, parse_zone, prev_occurrence};
pub use chrono_tz::Tz;
pub use types::*;

#[cfg(test)]
mod tests;
