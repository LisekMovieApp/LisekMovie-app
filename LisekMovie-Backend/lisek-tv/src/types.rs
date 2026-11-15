//! Input and output types of the engine.

use std::collections::HashMap;

use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::enumerator::EnumState;

/// One playable thing a slot can pick: a film or an episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: Uuid,
    pub duration_ms: i64,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub year: Option<i32>,
    pub title: String,
    /// "S01E02 Episode title" for an episode.
    pub subtitle: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayOrder {
    /// By year, then season and episode.
    Chronological,
    /// By season and episode.
    SeasonEpisode,
    /// Every item once in a random order, then a new order.
    Shuffle,
    /// Each pick independent.
    Random,
}

impl PlayOrder {
    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "chronological" => Self::Chronological,
            "season_episode" => Self::SeasonEpisode,
            "shuffle" => Self::Shuffle,
            "random" => Self::Random,
            _ => return None,
        })
    }

    pub fn tag(self) -> u64 {
        match self {
            Self::Chronological => 1,
            Self::SeasonEpisode => 2,
            Self::Shuffle => 3,
            Self::Random => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotMode {
    /// One item.
    One,
    /// `count` items.
    Multiple,
    /// As many items as fit in `duration_minutes`.
    Duration,
    /// Items until the next slot with a fixed start time.
    Flood,
}

impl SlotMode {
    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "one" => Self::One,
            "multiple" => Self::Multiple,
            "duration" => Self::Duration,
            "flood" => Self::Flood,
            _ => return None,
        })
    }
}

/// What happens when the next item does not fit the time that is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TailMode {
    /// Leave the rest empty (the channel shows its fallback).
    Offline,
    /// Fill the rest with the slot's tail filler.
    Filler,
    /// Play the item anyway; the slot ends when it ends.
    Overrun,
}

impl TailMode {
    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "offline" => Self::Offline,
            "filler" => Self::Filler,
            "overrun" => Self::Overrun,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartMode {
    /// On time (up to a few seconds late) or at the next occurrence.
    Strict,
    /// Also when up to half an hour late.
    Flexible,
}

impl StartMode {
    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "strict" => Self::Strict,
            "flexible" => Self::Flexible,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillerMode {
    /// `count` files.
    Count,
    /// Files adding up to `duration_ms`, the last one cut to fit.
    Duration,
    /// Files up to the next full `pad_minutes` of the local clock.
    Pad,
}

impl FillerMode {
    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "count" => Self::Count,
            "duration" => Self::Duration,
            "pad" => Self::Pad,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillerFile {
    pub id: Uuid,
    pub duration_ms: i64,
    pub title: String,
}

/// A filler preset with the files of its folders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillerPreset {
    pub id: Uuid,
    pub mode: FillerMode,
    pub count: u32,
    pub duration_ms: i64,
    pub pad_minutes: u32,
    pub files: Vec<FillerFile>,
}

/// One schedule slot, with its source already resolved to candidates.
#[derive(Debug, Clone)]
pub struct SlotSpec {
    pub id: Uuid,
    /// Local time of day in the channel's zone.
    pub start_time: Option<NaiveTime>,
    pub start_mode: StartMode,
    /// Monday = bit 0 ... Sunday = bit 6.
    pub days_of_week: u8,
    pub play_order: PlayOrder,
    pub mode: SlotMode,
    pub count: u32,
    pub duration_minutes: u32,
    pub tail_mode: TailMode,
    pub pre_filler: Option<Uuid>,
    pub post_filler: Option<Uuid>,
    pub tail_filler: Option<Uuid>,
    pub title_override: Option<String>,
    pub in_guide: bool,
    pub candidates: Vec<Candidate>,
}

/// Where the engine stopped; stored with the channel and passed back in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineState {
    /// End of the last planned entry (or of a wait).
    pub next_start: Option<DateTime<Utc>>,
    /// The slot to run next.
    pub cursor: usize,
    /// The slot at `cursor` has begun (a flood the horizon cut, or a slot
    /// a rebuild resumed in the middle): its start time is not awaited again.
    pub entered: bool,
    pub guide_group: i32,
    pub slots: HashMap<Uuid, EnumState>,
    pub fillers: HashMap<Uuid, EnumState>,
}

pub struct BuildInput<'a> {
    pub channel_seed: i64,
    pub tz: Tz,
    pub slots: &'a [SlotSpec],
    pub fillers: &'a HashMap<Uuid, FillerPreset>,
    pub state: EngineState,
    /// Nothing is planned before this.
    pub from: DateTime<Utc>,
    /// Planning stops once this is reached (the last entry may end later).
    pub until: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Content,
    Pre,
    Mid,
    Post,
    Tail,
    Pad,
}

impl ItemKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Pre => "pre",
            Self::Mid => "mid",
            Self::Post => "post",
            Self::Tail => "tail",
            Self::Pad => "pad",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "content" => Self::Content,
            "pre" => Self::Pre,
            "mid" => Self::Mid,
            "post" => Self::Post,
            "tail" => Self::Tail,
            "pad" => Self::Pad,
            _ => return None,
        })
    }
}

/// One entry of the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedItem {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub kind: ItemKind,
    pub media_item_id: Option<Uuid>,
    pub filler_file_id: Option<Uuid>,
    pub in_ms: i64,
    pub out_ms: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub slot_id: Uuid,
    pub guide_group: i32,
    pub in_guide: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BuildOutput {
    pub items: Vec<PlannedItem>,
    pub state: EngineState,
    pub warnings: Vec<String>,
}
