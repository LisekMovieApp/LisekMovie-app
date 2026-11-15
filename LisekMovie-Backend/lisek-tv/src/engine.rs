//! The schedule engine: slots in, a timeline out.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use uuid::Uuid;

use crate::enumerator::{EnumState, Enumerator, enumerator_seed};
use crate::time::{day_allowed, next_boundary, next_fixed, next_midnight, next_occurrence, prev_occurrence};
use crate::types::*;

/// Safety valves: a schedule can never make one build run away.
const MAX_ITEMS: usize = 20_000;
const MAX_ITERATIONS: usize = 100_000;
/// A filler gap shorter than this is not worth an entry.
const MIN_TRIM_MS: i64 = 1_000;

struct Builder<'a> {
    input: &'a BuildInput<'a>,
    st: EngineState,
    items: Vec<PlannedItem>,
    warnings: Vec<String>,
    slot_enums: HashMap<Uuid, Enumerator>,
    filler_enums: HashMap<Uuid, Enumerator>,
}

/// What a rolled-back entry bundle restores.
struct Checkpoint {
    items: usize,
    guide_group: i32,
    fillers: Vec<(Uuid, EnumState)>,
}

fn ms(duration_ms: i64) -> Duration {
    Duration::milliseconds(duration_ms)
}

impl<'a> Builder<'a> {
    fn tz(&self) -> Tz {
        self.input.tz
    }

    fn warn_once(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn slot_enum(&mut self, slot: &SlotSpec) -> &mut Enumerator {
        let seed = self.input.channel_seed;
        let state = &self.st.slots;
        self.slot_enums.entry(slot.id).or_insert_with(|| {
            Enumerator::new(
                &slot.candidates,
                slot.play_order,
                enumerator_seed(seed, slot.id),
                state.get(&slot.id).cloned().unwrap_or_default(),
            )
        })
    }

    fn filler_enum(&mut self, preset: &FillerPreset) -> &mut Enumerator {
        let seed = self.input.channel_seed;
        let state = &self.st.fillers;
        self.filler_enums.entry(preset.id).or_insert_with(|| {
            let files: Vec<Candidate> = preset
                .files
                .iter()
                .map(|f| Candidate {
                    id: f.id,
                    duration_ms: f.duration_ms,
                    season: None,
                    episode: None,
                    year: None,
                    title: f.title.clone(),
                    subtitle: None,
                })
                .collect();
            Enumerator::new(&files, PlayOrder::Shuffle, enumerator_seed(seed, preset.id), state.get(&preset.id).cloned().unwrap_or_default())
        })
    }

    fn checkpoint(&mut self, slot: &SlotSpec) -> Checkpoint {
        let mut fillers = Vec::new();
        for id in [slot.pre_filler, slot.post_filler].into_iter().flatten() {
            if let Some(e) = self.filler_enums.get(&id) {
                fillers.push((id, e.state().clone()));
            } else if let Some(state) = self.st.fillers.get(&id) {
                fillers.push((id, state.clone()));
            }
        }
        Checkpoint { items: self.items.len(), guide_group: self.st.guide_group, fillers }
    }

    fn rollback(&mut self, checkpoint: Checkpoint) {
        self.items.truncate(checkpoint.items);
        self.st.guide_group = checkpoint.guide_group;
        for (id, state) in checkpoint.fillers {
            self.filler_enums.remove(&id);
            self.st.fillers.insert(id, state);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: ItemKind,
        slot: &SlotSpec,
        start: DateTime<Utc>,
        in_ms: i64,
        out_ms: i64,
        media: Option<Uuid>,
        filler: Option<Uuid>,
        title: String,
        subtitle: Option<String>,
        group: i32,
    ) -> DateTime<Utc> {
        let end = start + ms(out_ms - in_ms);
        self.items.push(PlannedItem {
            starts_at: start,
            ends_at: end,
            kind,
            media_item_id: media,
            filler_file_id: filler,
            in_ms,
            out_ms,
            title,
            subtitle,
            slot_id: slot.id,
            guide_group: group,
            in_guide: slot.in_guide,
        });
        end
    }

    /// Fills time with a filler preset, from `start`. With a `target`, up to
    /// that time (the last file is cut to end exactly there); without one,
    /// as the preset's mode says. Returns where the filler ends.
    fn fill(&mut self, preset_id: Uuid, kind: ItemKind, slot: &SlotSpec, start: DateTime<Utc>, target: Option<DateTime<Utc>>, group: i32) -> DateTime<Utc> {
        let input = self.input;
        let Some(preset) = input.fillers.get(&preset_id) else { return start };
        if preset.files.is_empty() {
            return start;
        }
        let tz = self.tz();
        let span_ms = match (target, preset.mode) {
            (Some(end), _) => Some((end - start).num_milliseconds()),
            (None, FillerMode::Count) => None,
            (None, FillerMode::Duration) => Some(preset.duration_ms),
            (None, FillerMode::Pad) => Some((next_boundary(start, preset.pad_minutes, tz) - start).num_milliseconds()),
        };
        let mut time = start;
        let Some(mut remaining) = span_ms else {
            for _ in 0..preset.count {
                let Some(file) = self.filler_enum(preset).advance() else { break };
                time = self.push(kind, slot, time, 0, file.duration_ms, None, Some(file.id), file.title, None, group);
            }
            return time;
        };
        let mut guard = 0;
        while remaining >= MIN_TRIM_MS && guard < 5_000 && self.items.len() < MAX_ITEMS {
            guard += 1;
            let Some(file) = self.filler_enum(preset).advance() else { break };
            let play = file.duration_ms.min(remaining);
            time = self.push(kind, slot, time, 0, play, None, Some(file.id), file.title, None, group);
            remaining -= play;
        }
        time
    }

    /// One programme: its pre filler, the item, its post filler.
    fn bundle(&mut self, slot: &SlotSpec, candidate: &Candidate, start: DateTime<Utc>) -> DateTime<Utc> {
        let group = self.st.guide_group;
        let mut time = start;
        if let Some(pre) = slot.pre_filler {
            time = self.fill(pre, ItemKind::Pre, slot, time, None, group);
        }
        let title = slot.title_override.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| candidate.title.clone());
        time = self.push(ItemKind::Content, slot, time, 0, candidate.duration_ms, Some(candidate.id), None, title, candidate.subtitle.clone(), group);
        if let Some(post) = slot.post_filler {
            time = self.fill(post, ItemKind::Post, slot, time, None, group);
        }
        self.st.guide_group += 1;
        time
    }

    /// The start of the next slot when it has a fixed time (what a flood
    /// plays up to). A slot followed by a dynamic slot has none.
    fn deadline(&self, idx: usize, start: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let slots = self.input.slots;
        let next = (idx + 1) % slots.len();
        if next == idx || slots[next].candidates.is_empty() {
            return None;
        }
        let fixed = slots[next].start_time?;
        Some(next_occurrence(start + Duration::milliseconds(1), fixed, slots[next].days_of_week, self.tz()))
    }

    fn advance_cursor(&mut self) {
        let n = self.input.slots.len();
        self.st.cursor = (self.st.cursor + 1) % n;
        self.st.entered = false;
    }

    /// Plays one slot starting at `time`. Returns the time it ended at and
    /// whether the horizon cut it short (a flood to be resumed).
    fn run_slot(&mut self, idx: usize, start: DateTime<Utc>) -> (DateTime<Utc>, bool) {
        let input = self.input;
        let slot = &input.slots[idx];
        let until = input.until;
        let deadline = self.deadline(idx, start);
        let mut time = start;
        match slot.mode {
            SlotMode::One | SlotMode::Multiple => {
                let times = if slot.mode == SlotMode::One { 1 } else { slot.count.max(1) };
                for _ in 0..times {
                    let Some(candidate) = self.slot_enum(slot).peek(0).cloned() else { break };
                    time = self.bundle(slot, &candidate, time);
                    self.slot_enum(slot).advance();
                    if self.items.len() >= MAX_ITEMS {
                        break;
                    }
                }
                if let Some(tail) = slot.tail_filler {
                    let group = self.st.guide_group;
                    time = self.fill(tail, ItemKind::Tail, slot, time, None, group);
                    self.st.guide_group += 1;
                }
                (time, false)
            }
            SlotMode::Duration => {
                let mut end = start + Duration::minutes(i64::from(slot.duration_minutes.max(1)));
                if let Some(d) = deadline {
                    end = end.min(d);
                }
                while time < end && self.items.len() < MAX_ITEMS {
                    let Some(candidate) = self.slot_enum(slot).peek(0).cloned() else { break };
                    let checkpoint = self.checkpoint(slot);
                    let bundle_end = self.bundle(slot, &candidate, time);
                    if bundle_end > end && slot.tail_mode != TailMode::Overrun {
                        self.rollback(checkpoint);
                        break;
                    }
                    self.slot_enum(slot).advance();
                    time = bundle_end;
                }
                (self.finish_gap(slot, time, end), false)
            }
            SlotMode::Flood => {
                let stop = deadline.unwrap_or(until);
                let mut unfit = false;
                while time < stop && time < until && self.items.len() < MAX_ITEMS {
                    let Some(candidate) = self.slot_enum(slot).peek(0).cloned() else { break };
                    let checkpoint = self.checkpoint(slot);
                    let bundle_end = self.bundle(slot, &candidate, time);
                    if deadline.is_some_and(|d| bundle_end > d) && slot.tail_mode != TailMode::Overrun {
                        self.rollback(checkpoint);
                        unfit = true;
                        break;
                    }
                    self.slot_enum(slot).advance();
                    time = bundle_end;
                }
                match deadline {
                    Some(d) if unfit || time >= d => (self.finish_gap(slot, time, d), false),
                    // The horizon ended the loop before the next fixed slot.
                    _ => (time, time >= until || deadline.is_none()),
                }
            }
        }
    }

    /// The rest of a slot's time that no item filled: a tail filler, or a
    /// gap. Returns the slot's end (not earlier than `time`).
    fn finish_gap(&mut self, slot: &SlotSpec, time: DateTime<Utc>, end: DateTime<Utc>) -> DateTime<Utc> {
        if time >= end {
            return time;
        }
        if slot.tail_mode == TailMode::Filler
            && let Some(tail) = slot.tail_filler
        {
            let group = self.st.guide_group;
            let filled = self.fill(tail, ItemKind::Tail, slot, time, Some(end), group);
            if filled > time {
                self.st.guide_group += 1;
            }
        }
        end
    }
}

/// Plans the timeline from `input.state` up to `input.until`.
pub fn build(input: &BuildInput) -> BuildOutput {
    let mut b = Builder {
        input,
        st: input.state.clone(),
        items: Vec::new(),
        warnings: Vec::new(),
        slot_enums: HashMap::new(),
        filler_enums: HashMap::new(),
    };
    let n = input.slots.len();
    let mut time = b.st.next_start.map_or(input.from, |t| t.max(input.from));
    if n == 0 {
        b.st.next_start = Some(time.max(input.until));
        b.warn_once("the schedule has no slots".into());
        return BuildOutput { items: b.items, state: b.st, warnings: b.warnings };
    }
    b.st.cursor %= n;
    let mut idle = 0usize;
    let mut iterations = 0usize;
    'plan: while time < input.until {
        iterations += 1;
        if iterations > MAX_ITERATIONS || b.items.len() >= MAX_ITEMS {
            b.warn_once("the timeline hit the size limit of one build".into());
            time = input.until;
            break;
        }
        let idx = b.st.cursor;
        let slot = &input.slots[idx];
        let before = (time, b.items.len());
        if slot.candidates.is_empty() {
            b.warn_once(format!("slot {} has nothing to play and was skipped", idx + 1));
            b.advance_cursor();
            idle += 1;
            if idle >= n {
                time = input.until;
                b.warn_once("no slot has anything to play".into());
            }
            continue;
        }
        let mut start = time;
        if !b.st.entered {
            if let Some(fixed) = slot.start_time {
                start = next_fixed(time, fixed, slot.days_of_week, b.tz(), slot.start_mode == StartMode::Flexible);
                if start >= input.until {
                    // Its time lies beyond the horizon: wait there, plan nothing.
                    break 'plan;
                }
            } else if !day_allowed(slot.days_of_week, time, b.tz()) {
                b.advance_cursor();
                idle += 1;
                if idle >= n {
                    time = next_midnight(time, b.tz());
                    idle = 0;
                }
                continue;
            }
        }
        b.st.entered = true;
        let (end, cut) = b.run_slot(idx, start);
        time = end.max(start);
        if cut {
            // The horizon stopped a flood: the next build resumes it.
            break;
        }
        b.advance_cursor();
        if (time, b.items.len()) == before {
            idle += 1;
            if idle >= n {
                b.warn_once("the schedule makes no progress and was stopped".into());
                time = input.until;
            }
        } else {
            idle = 0;
        }
    }
    // Write the enumerators back.
    for (id, e) in &b.slot_enums {
        b.st.slots.insert(*id, e.state().clone());
    }
    for (id, e) in &b.filler_enums {
        b.st.fillers.insert(*id, e.state().clone());
    }
    let keep: Vec<Uuid> = input.slots.iter().map(|s| s.id).collect();
    b.st.slots.retain(|id, _| keep.contains(id));
    // When the next slot starts beyond the horizon, `time` is where the wait
    // began and the next build computes that start again.
    b.st.next_start = Some(time);
    BuildOutput { items: b.items, state: b.st, warnings: b.warnings }
}

/// Where a schedule resumes when a timeline is rebuilt from `at` (the end
/// of what is playing now): the slot that was running at that time. Returns
/// the cursor and whether the slot counts as already started.
///
/// A flood slot is still running until the next fixed slot starts. A
/// duration slot is while its time lasts. The length of a one/multiple slot
/// is unknown here, so it counts as finished and the schedule goes on with
/// the slot after it.
pub fn restart_state(slots: &[SlotSpec], tz: Tz, at: DateTime<Utc>) -> (usize, bool) {
    let latest = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.candidates.is_empty())
        .filter_map(|(i, s)| Some((i, prev_occurrence(at, s.start_time?, s.days_of_week, tz)?)))
        .max_by_key(|(_, occurrence)| *occurrence);
    let Some((idx, occurrence)) = latest else { return (0, false) };
    let slot = &slots[idx];
    match slot.mode {
        SlotMode::Flood => (idx, true),
        SlotMode::Duration if at < occurrence + Duration::minutes(i64::from(slot.duration_minutes)) => (idx, true),
        _ => ((idx + 1) % slots.len(), false),
    }
}
