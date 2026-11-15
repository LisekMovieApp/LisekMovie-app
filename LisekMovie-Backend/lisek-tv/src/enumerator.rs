//! Walks a list of candidates in a play order, remembering where it was.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::rng::{mix, mix2, permutation};
use crate::types::{Candidate, PlayOrder};

/// Where an enumerator stopped. Stored in the channel's engine state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumState {
    /// Which pass over the list (a shuffle has a new order per pass).
    pub pass: u32,
    /// Position inside the pass.
    pub index: u32,
    /// Picks made so far (the `random` order).
    pub counter: u64,
    /// The item that played last.
    pub last_id: Option<Uuid>,
    /// Of the list and order the position belongs to.
    pub fingerprint: u64,
}

/// A fingerprint of a list of candidates: changes when items come or go.
pub fn candidates_fingerprint(items: &[Candidate]) -> u64 {
    let mut ids: Vec<Uuid> = items.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.iter().fold(0x51ED_270B_u64, |acc, id| {
        let (hi, lo) = id.as_u64_pair();
        mix2(mix2(acc, hi), lo)
    })
}

pub struct Enumerator {
    /// The candidates in their pass-0 order (`shuffle` and `random` start
    /// from the id order, so the database order never matters).
    base: Vec<Candidate>,
    order: PlayOrder,
    seed: u64,
    state: EnumState,
}

fn sort_base(items: &[Candidate], order: PlayOrder) -> Vec<Candidate> {
    let mut base = items.to_vec();
    match order {
        PlayOrder::Chronological => base.sort_by(|a, b| {
            (a.year.unwrap_or(0), a.season.unwrap_or(0), a.episode.unwrap_or(0), a.title.to_lowercase(), a.id)
                .cmp(&(b.year.unwrap_or(0), b.season.unwrap_or(0), b.episode.unwrap_or(0), b.title.to_lowercase(), b.id))
        }),
        PlayOrder::SeasonEpisode => base.sort_by(|a, b| {
            (a.season.unwrap_or(0), a.episode.unwrap_or(0), a.year.unwrap_or(0), a.title.to_lowercase(), a.id)
                .cmp(&(b.season.unwrap_or(0), b.episode.unwrap_or(0), b.year.unwrap_or(0), b.title.to_lowercase(), b.id))
        }),
        PlayOrder::Shuffle | PlayOrder::Random => base.sort_by_key(|c| c.id),
    }
    base
}

impl Enumerator {
    /// An enumerator over `items`, continuing from `state` when it belongs
    /// to the same list and order. When the list changed, an ordered walk
    /// goes on after the item that played last; a shuffle starts a new pass.
    pub fn new(items: &[Candidate], order: PlayOrder, seed: u64, mut state: EnumState) -> Self {
        let base = sort_base(items, order);
        let fingerprint = mix2(candidates_fingerprint(items), order.tag());
        if state.fingerprint != fingerprint {
            let had_position = state.fingerprint != 0;
            state.fingerprint = fingerprint;
            match order {
                PlayOrder::Chronological | PlayOrder::SeasonEpisode => {
                    let after_last = state
                        .last_id
                        .and_then(|id| base.iter().position(|c| c.id == id))
                        .map(|p| p + 1);
                    // The item that played last is gone: what followed it moved
                    // one place up.
                    state.index = after_last.unwrap_or(if had_position { state.index.saturating_sub(1) as usize } else { 0 }) as u32;
                    state.pass = 0;
                }
                PlayOrder::Shuffle => {
                    if had_position {
                        state.pass += 1;
                    }
                    state.index = 0;
                }
                PlayOrder::Random => {}
            }
        }
        let mut enumerator = Enumerator { base, order, seed, state };
        enumerator.wrap();
        enumerator
    }

    pub fn state(&self) -> &EnumState {
        &self.state
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    /// Moves `index` into range, rolling over into the next pass.
    fn wrap(&mut self) {
        let n = self.base.len() as u32;
        if n == 0 {
            return;
        }
        while self.state.index >= n {
            self.state.index -= n;
            self.state.pass += 1;
        }
    }

    fn pass_order(&self, pass: u32) -> Vec<usize> {
        let n = self.base.len();
        let mut order = permutation(n, mix2(self.seed, u64::from(pass)));
        if pass > 0 && n > 1 {
            // The first item of a pass is never the last one of the pass before.
            let previous = permutation(n, mix2(self.seed, u64::from(pass - 1)));
            if previous.last() == Some(&order[0]) {
                order.swap(0, 1);
            }
        }
        order
    }

    /// The `ahead`-th item from the current position, without moving.
    pub fn peek(&self, ahead: usize) -> Option<&Candidate> {
        let n = self.base.len();
        if n == 0 {
            return None;
        }
        match self.order {
            PlayOrder::Chronological | PlayOrder::SeasonEpisode => {
                Some(&self.base[(self.state.index as usize + ahead) % n])
            }
            PlayOrder::Shuffle => {
                let position = self.state.index as usize + ahead;
                let pass = self.state.pass + (position / n) as u32;
                Some(&self.base[self.pass_order(pass)[position % n]])
            }
            PlayOrder::Random => {
                // Each pick is a seeded draw that never repeats the pick
                // before it (the one that played last, to begin with).
                let mut previous = self.state.last_id.and_then(|id| self.base.iter().position(|c| c.id == id));
                let mut index = 0;
                for step in 0..=ahead as u64 {
                    index = (mix2(self.seed, self.state.counter + step) % n as u64) as usize;
                    if n > 1 && previous == Some(index) {
                        index = (index + 1) % n;
                    }
                    previous = Some(index);
                }
                Some(&self.base[index])
            }
        }
    }

    /// Takes the current item and moves on.
    pub fn advance(&mut self) -> Option<Candidate> {
        let current = self.peek(0)?.clone();
        self.state.last_id = Some(current.id);
        self.state.counter += 1;
        self.state.index += 1;
        self.wrap();
        Some(current)
    }
}

/// A seed for an enumerator from the channel seed and an id.
pub fn enumerator_seed(channel_seed: i64, id: Uuid) -> u64 {
    mix(channel_seed as u64) ^ crate::rng::uuid_seed(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<Candidate> {
        (0..n)
            .map(|i| Candidate {
                id: Uuid::from_u128(i as u128 + 1),
                duration_ms: 60_000,
                season: Some(1 + (i / 10) as i32),
                episode: Some(1 + (i % 10) as i32),
                year: None,
                title: format!("Show {i}"),
                subtitle: None,
            })
            .collect()
    }

    fn take(enumerator: &mut Enumerator, n: usize) -> Vec<Uuid> {
        (0..n).map(|_| enumerator.advance().unwrap().id).collect()
    }

    #[test]
    fn chronological_cycles_in_order() {
        let list = items(3);
        let mut e = Enumerator::new(&list, PlayOrder::Chronological, 1, EnumState::default());
        let got = take(&mut e, 7);
        let ids: Vec<u128> = got.iter().map(|id| id.as_u128()).collect();
        assert_eq!(ids, vec![1, 2, 3, 1, 2, 3, 1]);
    }

    #[test]
    fn shuffle_plays_everything_once_per_pass_and_never_repeats_at_the_seam() {
        let list = items(6);
        for seed in 0..40 {
            let mut e = Enumerator::new(&list, PlayOrder::Shuffle, seed, EnumState::default());
            let got = take(&mut e, 24);
            for pass in got.chunks(6) {
                let mut sorted = pass.to_vec();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted.len(), 6, "seed {seed}: a pass has every item once");
            }
            for pair in got.windows(2) {
                assert_ne!(pair[0], pair[1], "seed {seed}: no immediate repeat");
            }
        }
    }

    #[test]
    fn the_database_order_does_not_change_a_shuffle() {
        let list = items(8);
        let mut reversed = list.clone();
        reversed.reverse();
        let a = take(&mut Enumerator::new(&list, PlayOrder::Shuffle, 5, EnumState::default()), 8);
        let b = take(&mut Enumerator::new(&reversed, PlayOrder::Shuffle, 5, EnumState::default()), 8);
        assert_eq!(a, b);
    }

    #[test]
    fn peeking_matches_taking_and_state_resumes() {
        let list = items(5);
        for order in [PlayOrder::Chronological, PlayOrder::Shuffle, PlayOrder::Random, PlayOrder::SeasonEpisode] {
            let fresh = Enumerator::new(&list, order, 9, EnumState::default());
            let peeked: Vec<Uuid> = (0..12).map(|k| fresh.peek(k).unwrap().id).collect();
            let mut e = Enumerator::new(&list, order, 9, EnumState::default());
            let mut got = take(&mut e, 5);
            // Resume from the stored state, as after a restart.
            let json = serde_json::to_string(e.state()).unwrap();
            let mut resumed = Enumerator::new(&list, order, 9, serde_json::from_str(&json).unwrap());
            got.extend(take(&mut resumed, 7));
            assert_eq!(got, peeked, "{order:?}");
        }
    }

    #[test]
    fn random_does_not_repeat_back_to_back() {
        let list = items(3);
        let mut e = Enumerator::new(&list, PlayOrder::Random, 3, EnumState::default());
        let got = take(&mut e, 200);
        assert!(got.windows(2).all(|p| p[0] != p[1]));
        assert!(got.iter().collect::<std::collections::HashSet<_>>().len() == 3);
    }

    #[test]
    fn an_ordered_walk_continues_after_the_last_item_when_the_list_changes() {
        let list = items(5);
        let mut e = Enumerator::new(&list, PlayOrder::SeasonEpisode, 1, EnumState::default());
        take(&mut e, 2); // items 1 and 2 played
        let state = e.state().clone();
        // Item 2 is removed and a new first item appears.
        let mut changed = items(6);
        changed.remove(1);
        let mut e = Enumerator::new(&changed, PlayOrder::SeasonEpisode, 1, state);
        assert_eq!(e.advance().unwrap().id.as_u128(), 3, "after the last played item (2) in the new list");
    }

    #[test]
    fn an_empty_list_yields_nothing() {
        let mut e = Enumerator::new(&[], PlayOrder::Shuffle, 1, EnumState::default());
        assert!(e.is_empty() && e.peek(0).is_none() && e.advance().is_none());
    }
}
