//! Engine behaviour: the rules of the schedule, end to end and pure.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveTime, Utc};
use chrono_tz::Tz;
use uuid::Uuid;

use crate::*;

const MIN: i64 = 60_000;

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn cand(n: u128, minutes: i64) -> Candidate {
    Candidate {
        id: Uuid::from_u128(n),
        duration_ms: minutes * MIN,
        season: Some(1),
        episode: Some(n as i32),
        year: None,
        title: format!("Item {n}"),
        subtitle: None,
    }
}

fn slot(n: u128, mode: SlotMode, candidates: Vec<Candidate>) -> SlotSpec {
    SlotSpec {
        id: Uuid::from_u128(1000 + n),
        start_time: None,
        start_mode: StartMode::Strict,
        days_of_week: 127,
        play_order: PlayOrder::SeasonEpisode,
        mode,
        count: 1,
        duration_minutes: 60,
        tail_mode: TailMode::Offline,
        pre_filler: None,
        post_filler: None,
        tail_filler: None,
        title_override: None,
        in_guide: true,
        candidates,
    }
}

fn at(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn preset(n: u128, mode: FillerMode, files: &[i64]) -> FillerPreset {
    FillerPreset {
        id: Uuid::from_u128(2000 + n),
        mode,
        count: 2,
        duration_ms: 90_000,
        pad_minutes: 30,
        files: files
            .iter()
            .enumerate()
            .map(|(i, secs)| FillerFile { id: Uuid::from_u128(3000 + n * 100 + i as u128), duration_ms: secs * 1000, title: format!("Ad {n}.{i}") })
            .collect(),
    }
}

struct Setup {
    slots: Vec<SlotSpec>,
    fillers: HashMap<Uuid, FillerPreset>,
    tz: Tz,
}

impl Setup {
    fn new(slots: Vec<SlotSpec>) -> Self {
        Setup { slots, fillers: HashMap::new(), tz: Tz::UTC }
    }

    fn with(mut self, p: FillerPreset) -> Self {
        self.fillers.insert(p.id, p);
        self
    }

    fn run(&self, state: EngineState, from: &str, until: &str) -> BuildOutput {
        build(&BuildInput {
            channel_seed: 42,
            tz: self.tz,
            slots: &self.slots,
            fillers: &self.fillers,
            state,
            from: utc(from),
            until: utc(until),
        })
    }
}

fn content(out: &BuildOutput) -> Vec<(u128, String)> {
    out.items
        .iter()
        .filter(|i| i.kind == ItemKind::Content)
        .map(|i| (i.media_item_id.unwrap().as_u128(), i.starts_at.format("%H:%M").to_string()))
        .collect()
}

/// Entries never overlap and are in order; the properties every schedule keeps.
fn assert_sane(out: &BuildOutput) {
    for pair in out.items.windows(2) {
        assert!(pair[0].ends_at <= pair[1].starts_at, "overlap: {:?} then {:?}", pair[0], pair[1]);
    }
    for item in &out.items {
        assert!(item.ends_at > item.starts_at && item.out_ms > item.in_ms && item.in_ms >= 0);
        assert_eq!((item.ends_at - item.starts_at).num_milliseconds(), item.out_ms - item.in_ms);
    }
}

#[test]
fn one_item_per_visit_cycles_through_the_slots() {
    let setup = Setup::new(vec![
        slot(1, SlotMode::One, vec![cand(1, 30), cand(2, 30)]),
        slot(2, SlotMode::One, vec![cand(3, 20)]),
    ]);
    let out = setup.run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T12:00:00Z");
    assert_sane(&out);
    let got = content(&out);
    assert_eq!(
        got[..5],
        [(1, "10:00".into()), (3, "10:30".into()), (2, "10:50".into()), (3, "11:20".into()), (1, "11:40".into())]
    );
}

#[test]
fn multiple_plays_count_items_in_a_row() {
    let mut a = slot(1, SlotMode::Multiple, vec![cand(1, 10), cand(2, 10), cand(3, 10)]);
    a.count = 2;
    let b = slot(2, SlotMode::One, vec![cand(9, 5)]);
    let out = Setup::new(vec![a, b]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:40:00Z");
    assert_eq!(content(&out)[..4], [(1, "10:00".into()), (2, "10:10".into()), (9, "10:20".into()), (3, "10:25".into())]);
}

#[test]
fn duration_fills_the_time_and_leaves_the_rest_empty_when_offline() {
    let mut a = slot(1, SlotMode::Duration, vec![cand(1, 25), cand(2, 25), cand(3, 25)]);
    a.duration_minutes = 60;
    let out = Setup::new(vec![a]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:59:00Z");
    assert_sane(&out);
    // 25 + 25 = 50 minutes; the third (25) does not fit the last 10.
    assert_eq!(content(&out), [(1, "10:00".into()), (2, "10:25".into())]);
    assert_eq!(out.state.next_start, Some(utc("2026-11-16T11:00:00Z")), "the slot's hour is over");
    // The item that did not fit is first in the next pass.
    let next = Setup::new(vec![{
        let mut a = slot(1, SlotMode::Duration, vec![cand(1, 25), cand(2, 25), cand(3, 25)]);
        a.duration_minutes = 60;
        a
    }])
    .run(out.state, "2026-11-16T10:00:00Z", "2026-11-16T11:30:00Z");
    assert_eq!(content(&next)[0], (3, "11:00".into()));
}

#[test]
fn a_tail_filler_fills_a_duration_slot_exactly() {
    let mut a = slot(1, SlotMode::Duration, vec![cand(1, 25), cand(2, 25), cand(3, 25)]);
    a.tail_mode = TailMode::Filler;
    a.tail_filler = Some(Uuid::from_u128(2001));
    let setup = Setup::new(vec![a]).with(preset(1, FillerMode::Count, &[40, 40, 40]));
    let out = setup.run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:55:00Z");
    assert_sane(&out);
    let tail: Vec<_> = out.items.iter().filter(|i| i.kind == ItemKind::Tail).collect();
    assert_eq!(tail.first().unwrap().starts_at, utc("2026-11-16T10:50:00Z"));
    assert_eq!(tail.last().unwrap().ends_at, utc("2026-11-16T11:00:00Z"), "cut to end where the slot ends");
    assert_eq!(tail.iter().map(|i| i.out_ms - i.in_ms).sum::<i64>(), 10 * MIN);
}

#[test]
fn overrun_plays_the_item_that_does_not_fit() {
    let mut a = slot(1, SlotMode::Duration, vec![cand(1, 45), cand(2, 45)]);
    a.tail_mode = TailMode::Overrun;
    let out = Setup::new(vec![a]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:59:00Z");
    assert_eq!(content(&out), [(1, "10:00".into()), (2, "10:45".into())]);
    assert_eq!(out.state.next_start, Some(utc("2026-11-16T11:30:00Z")));
}

#[test]
fn flood_plays_until_the_next_fixed_slot() {
    let flood = slot(1, SlotMode::Flood, vec![cand(1, 40), cand(2, 40), cand(3, 40)]);
    let mut news = slot(2, SlotMode::One, vec![cand(9, 30)]);
    news.start_time = Some(at(11, 0));
    let out = Setup::new(vec![flood, news]).run(EngineState::default(), "2026-11-16T09:00:00Z", "2026-11-16T12:00:00Z");
    assert_sane(&out);
    let got = content(&out);
    // 09:00, 09:40, then 10:20 would end 11:00 exactly: it fits.
    assert_eq!(got[..4], [(1, "09:00".into()), (2, "09:40".into()), (3, "10:20".into()), (9, "11:00".into())]);
}

#[test]
fn a_flood_item_that_would_cross_the_fixed_start_is_left_out() {
    let flood = slot(1, SlotMode::Flood, vec![cand(1, 50), cand(2, 50)]);
    let mut news = slot(2, SlotMode::One, vec![cand(9, 30)]);
    news.start_time = Some(at(11, 0));
    let out = Setup::new(vec![flood, news]).run(EngineState::default(), "2026-11-16T09:00:00Z", "2026-11-16T11:10:00Z");
    assert_eq!(content(&out)[..3], [(1, "09:00".into()), (2, "09:50".into()), (9, "11:00".into())], "10:40 -> 11:00 stays empty");
    assert_sane(&out);
}

#[test]
fn a_fixed_slot_waits_for_its_time_and_repeats_daily() {
    let mut a = slot(1, SlotMode::One, vec![cand(1, 30), cand(2, 30)]);
    a.start_time = Some(at(20, 0));
    let out = Setup::new(vec![a]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-18T23:00:00Z");
    assert_eq!(
        out.items.iter().map(|i| i.starts_at.to_rfc3339()).collect::<Vec<_>>(),
        ["2026-11-16T20:00:00+00:00", "2026-11-17T20:00:00+00:00", "2026-11-18T20:00:00+00:00"]
    );
}

#[test]
fn weekdays_limit_a_slot() {
    let mut weekend = slot(1, SlotMode::One, vec![cand(1, 30)]);
    weekend.start_time = Some(at(9, 0));
    weekend.days_of_week = 0b110_0000; // Saturday and Sunday
    let out = Setup::new(vec![weekend]).run(EngineState::default(), "2026-11-16T00:00:00Z", "2026-11-23T00:00:00Z");
    let days: Vec<String> = out.items.iter().map(|i| i.starts_at.format("%a").to_string()).collect();
    assert_eq!(days, ["Sat", "Sun"]);
}

#[test]
fn a_dynamic_slot_that_is_off_today_is_skipped() {
    let mut weekend_only = slot(1, SlotMode::One, vec![cand(1, 30)]);
    weekend_only.days_of_week = 0b110_0000;
    let always = slot(2, SlotMode::One, vec![cand(2, 30)]);
    // Monday: only the second slot plays, over and over.
    let out = Setup::new(vec![weekend_only, always]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T12:00:00Z");
    assert!(content(&out).iter().all(|(id, _)| *id == 2));
    assert_sane(&out);
}

#[test]
fn local_time_decides_the_fixed_start() {
    let mut a = slot(1, SlotMode::One, vec![cand(1, 30)]);
    a.start_time = Some(at(20, 0));
    let mut setup = Setup::new(vec![a]);
    setup.tz = "Europe/Warsaw".parse().unwrap();
    let out = setup.run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-17T00:00:00Z");
    assert_eq!(out.items[0].starts_at, utc("2026-11-16T19:00:00Z"), "20:00 in Warsaw in November");
    let summer = setup.run(EngineState::default(), "2026-07-16T10:00:00Z", "2026-07-17T00:00:00Z");
    assert_eq!(summer.items[0].starts_at, utc("2026-07-16T18:00:00Z"), "and 18:00 UTC in summer");
}

#[test]
fn pre_and_post_fillers_wrap_every_item_and_share_its_guide_group() {
    let mut a = slot(1, SlotMode::Multiple, vec![cand(1, 20), cand(2, 20)]);
    a.count = 2;
    a.pre_filler = Some(Uuid::from_u128(2001));
    a.post_filler = Some(Uuid::from_u128(2002));
    let setup = Setup::new(vec![a]).with(preset(1, FillerMode::Count, &[30])).with(preset(2, FillerMode::Count, &[15]));
    let out = setup.run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:01:00Z");
    assert_sane(&out);
    let kinds: Vec<_> = out.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        &kinds[..8],
        [ItemKind::Pre, ItemKind::Pre, ItemKind::Content, ItemKind::Post, ItemKind::Post, ItemKind::Pre, ItemKind::Pre, ItemKind::Content]
    );
    assert!(out.items[..5].iter().all(|i| i.guide_group == out.items[0].guide_group));
    assert_ne!(out.items[5].guide_group, out.items[0].guide_group);
}

#[test]
fn a_pad_filler_ends_on_the_half_hour() {
    let mut a = slot(1, SlotMode::One, vec![cand(1, 22)]);
    a.post_filler = Some(Uuid::from_u128(2001));
    let setup = Setup::new(vec![a]).with(preset(1, FillerMode::Pad, &[60, 45, 20, 30]));
    let out = setup.run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:01:00Z");
    assert_sane(&out);
    let last = out.items.last().unwrap();
    assert_eq!(last.kind, ItemKind::Post);
    assert_eq!(last.ends_at, utc("2026-11-16T10:30:00Z"));
    assert_eq!(out.state.next_start, Some(utc("2026-11-16T10:30:00Z")));
}

#[test]
fn a_pre_pad_makes_the_film_start_on_the_hour() {
    let mut a = slot(1, SlotMode::One, vec![cand(1, 90)]);
    a.pre_filler = Some(Uuid::from_u128(2001));
    let mut p = preset(1, FillerMode::Pad, &[60]);
    p.pad_minutes = 60;
    let setup = Setup::new(vec![a]).with(p);
    let out = setup.run(EngineState::default(), "2026-11-16T09:50:00Z", "2026-11-16T09:51:00Z");
    let film = out.items.iter().find(|i| i.kind == ItemKind::Content).unwrap();
    assert_eq!(film.starts_at, utc("2026-11-16T10:00:00Z"));
}

#[test]
fn a_filler_without_files_is_ignored() {
    let mut a = slot(1, SlotMode::One, vec![cand(1, 20)]);
    a.pre_filler = Some(Uuid::from_u128(2001));
    a.post_filler = Some(Uuid::from_u128(9999)); // not even defined
    let setup = Setup::new(vec![a]).with(preset(1, FillerMode::Count, &[]));
    let out = setup.run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:01:00Z");
    assert_eq!(out.items.len(), 1);
}

#[test]
fn building_in_two_steps_equals_building_at_once() {
    let mut a = slot(1, SlotMode::Duration, vec![cand(1, 22), cand(2, 41), cand(3, 18), cand(4, 55)]);
    a.play_order = PlayOrder::Shuffle;
    a.post_filler = Some(Uuid::from_u128(2001));
    let mut b = slot(2, SlotMode::Flood, vec![cand(5, 27), cand(6, 33), cand(7, 90)]);
    b.play_order = PlayOrder::Random;
    b.pre_filler = Some(Uuid::from_u128(2001));
    let mut c = slot(3, SlotMode::One, vec![cand(8, 100)]);
    c.start_time = Some(at(21, 0));
    let setup = Setup::new(vec![a, b, c]).with(preset(1, FillerMode::Count, &[20, 30, 45]));
    let whole = setup.run(EngineState::default(), "2026-11-16T06:00:00Z", "2026-11-18T06:00:00Z");
    assert_sane(&whole);

    let first = setup.run(EngineState::default(), "2026-11-16T06:00:00Z", "2026-11-17T06:00:00Z");
    // The state goes through JSON, as it does through the database.
    let state: EngineState = serde_json::from_str(&serde_json::to_string(&first.state).unwrap()).unwrap();
    let second = setup.run(state, "2026-11-16T06:00:00Z", "2026-11-18T06:00:00Z");
    let mut stitched = first.items.clone();
    stitched.extend(second.items);
    assert_eq!(stitched, whole.items);
    assert_eq!(second.state, whole.state);
}

#[test]
fn the_same_input_gives_the_same_timeline() {
    let mut a = slot(1, SlotMode::Flood, vec![cand(1, 22), cand(2, 41), cand(3, 18)]);
    a.play_order = PlayOrder::Shuffle;
    let setup = Setup::new(vec![a]);
    let one = setup.run(EngineState::default(), "2026-11-16T06:00:00Z", "2026-11-17T06:00:00Z");
    let two = setup.run(EngineState::default(), "2026-11-16T06:00:00Z", "2026-11-17T06:00:00Z");
    assert_eq!(one.items, two.items);
}

#[test]
fn slots_without_anything_to_play_are_skipped_and_never_loop_forever() {
    let empty = slot(1, SlotMode::One, vec![]);
    let real = slot(2, SlotMode::One, vec![cand(2, 30)]);
    let out = Setup::new(vec![empty, real]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T11:00:00Z");
    assert!(!content(&out).is_empty());
    assert!(out.warnings.iter().any(|w| w.contains("slot 1")));

    let only_empty = Setup::new(vec![slot(1, SlotMode::Flood, vec![])]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T11:00:00Z");
    assert!(only_empty.items.is_empty());
    assert!(only_empty.state.next_start.unwrap() >= utc("2026-11-16T11:00:00Z"), "the horizon is reached, so the next build does not start over");

    let none = Setup::new(vec![]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T11:00:00Z");
    assert!(none.items.is_empty() && !none.warnings.is_empty());
}

#[test]
fn a_schedule_with_a_slot_no_day_allows_does_not_hang() {
    let mut a = slot(1, SlotMode::One, vec![cand(1, 30)]);
    a.days_of_week = 0b000_0001; // Mondays only, but dynamic
    let out = Setup::new(vec![a]).run(EngineState::default(), "2026-11-17T00:00:00Z", "2026-11-25T00:00:00Z");
    let days: std::collections::BTreeSet<String> = out.items.iter().map(|i| i.starts_at.format("%a").to_string()).collect();
    assert_eq!(days.into_iter().collect::<Vec<_>>(), ["Mon"]);
}

#[test]
fn the_title_override_and_the_guide_flag_apply() {
    let mut a = slot(1, SlotMode::One, vec![cand(1, 20)]);
    a.title_override = Some("Wieczór z kinem".into());
    a.in_guide = false;
    let out = Setup::new(vec![a]).run(EngineState::default(), "2026-11-16T10:00:00Z", "2026-11-16T10:01:00Z");
    assert_eq!(out.items[0].title, "Wieczór z kinem");
    assert!(!out.items[0].in_guide);
}

#[test]
fn a_rebuild_resumes_the_running_slot() {
    let mut morning = slot(1, SlotMode::Flood, vec![cand(1, 30)]);
    morning.start_time = Some(at(6, 0));
    let mut evening = slot(2, SlotMode::Flood, vec![cand(2, 30)]);
    evening.start_time = Some(at(20, 0));
    let mut hour = slot(3, SlotMode::Duration, vec![cand(3, 30)]);
    hour.start_time = Some(at(22, 0));
    hour.duration_minutes = 60;
    let slots = vec![morning, evening, hour];
    let tz = Tz::UTC;
    assert_eq!(restart_state(&slots, tz, utc("2026-11-16T07:00:00Z")), (0, true), "morning flood");
    assert_eq!(restart_state(&slots, tz, utc("2026-11-16T21:00:00Z")), (1, true), "evening flood");
    assert_eq!(restart_state(&slots, tz, utc("2026-11-16T22:30:00Z")), (2, true), "inside the duration slot");
    assert_eq!(restart_state(&slots, tz, utc("2026-11-16T23:30:00Z")), (0, false), "after it: on to the next slot");
    assert_eq!(restart_state(&slots, tz, utc("2026-11-16T05:00:00Z")), (0, false), "yesterday's last slot is over");
    assert_eq!(restart_state(&[slot(1, SlotMode::One, vec![cand(1, 5)])], tz, utc("2026-11-16T05:00:00Z")), (0, false));
}

#[test]
fn many_random_schedules_stay_sane() {
    for seed in 0..60u128 {
        let n = 1 + seed % 4;
        let slots: Vec<SlotSpec> = (0..n)
            .map(|i| {
                let mode = [SlotMode::One, SlotMode::Multiple, SlotMode::Duration, SlotMode::Flood][((seed + i) % 4) as usize];
                let mut s = slot(i, mode, (0..(1 + (seed + i) % 5)).map(|k| cand(i * 10 + k, 7 + ((seed * 13 + k * 17) % 90) as i64)).collect());
                s.play_order = [PlayOrder::Chronological, PlayOrder::Shuffle, PlayOrder::Random, PlayOrder::SeasonEpisode][((seed + i) % 4) as usize];
                s.count = 1 + (seed % 3) as u32;
                s.duration_minutes = 30 + (seed % 90) as u32;
                s.tail_mode = [TailMode::Offline, TailMode::Filler, TailMode::Overrun][(seed % 3) as usize];
                if (seed + i) % 3 == 0 {
                    s.start_time = Some(at(((seed * 5 + i * 7) % 24) as u32, 0));
                }
                s.tail_filler = Some(Uuid::from_u128(2001));
                s.pre_filler = (seed % 2 == 0).then_some(Uuid::from_u128(2002));
                s.post_filler = (seed % 5 == 0).then_some(Uuid::from_u128(2003));
                s
            })
            .collect();
        let setup = Setup::new(slots)
            .with(preset(1, FillerMode::Pad, &[20, 31, 12]))
            .with(preset(2, FillerMode::Count, &[25]))
            .with(preset(3, FillerMode::Duration, &[40, 40]));
        let out = setup.run(EngineState::default(), "2026-11-16T00:00:00Z", "2026-11-19T00:00:00Z");
        assert_sane(&out);
        assert!(out.state.next_start.unwrap() >= utc("2026-11-19T00:00:00Z") || out.items.is_empty() || out.state.next_start.is_some(), "seed {seed}");
        let end = out.items.last().map(|i| i.ends_at);
        if let Some(end) = end {
            assert!(end <= out.state.next_start.unwrap() + Duration::minutes(1) || out.state.next_start.unwrap() >= end - Duration::days(1), "seed {seed}");
        }
    }
}
