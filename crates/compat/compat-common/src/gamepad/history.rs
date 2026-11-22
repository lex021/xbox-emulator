use std::collections::VecDeque;

pub const MAX_READINGS: usize = 512;
pub const HORIZON_US: u64 = 500_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub seq: u64,
    pub ts: u64,
    pub state: [u8; 28],
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Found(Entry),
    NotFound,
    TooOld,
}

pub struct History {
    v: VecDeque<Entry>,
    dropped: bool,
}

impl History {
    pub const fn new() -> Self {
        History { v: VecDeque::new(), dropped: false }
    }

    pub fn push(&mut self, e: Entry) {
        self.v.push_back(e);
        while self.v.len() > MAX_READINGS {
            self.v.pop_front();
            self.dropped = true;
        }
        self.prune(e.ts);
    }

    pub fn prune(&mut self, now: u64) {
        while self.v.len() >= 2 && now.saturating_sub(self.v[1].ts) > HORIZON_US {
            self.v.pop_front();
            self.dropped = true;
        }
    }

    fn gone(&self, seq: u64) -> bool {
        self.dropped && self.v.front().map_or(true, |f| seq < f.seq)
    }

    pub fn next(&mut self, seq: u64, now: u64) -> Step {
        self.prune(now);
        if self.gone(seq) {
            return Step::TooOld;
        }
        match self.v.iter().find(|e| e.seq > seq) {
            Some(e) => Step::Found(*e),
            None => Step::NotFound,
        }
    }

    pub fn prev(&mut self, seq: u64, now: u64) -> Step {
        self.prune(now);
        if self.gone(seq) {
            return Step::TooOld;
        }
        match self.v.iter().rev().find(|e| e.seq < seq) {
            Some(e) => Step::Found(*e),
            None if self.dropped => Step::TooOld,
            None => Step::NotFound,
        }
    }

    pub fn len(&self) -> usize {
        self.v.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(seq: u64, ts: u64, b: u8) -> Entry {
        let mut state = [0u8; 28];
        state[0] = b;
        Entry { seq, ts, state }
    }

    #[test]
    fn press_and_release_between_polls_are_both_walkable() {
        let mut h = History::new();
        h.push(e(1, 0, 0));
        h.push(e(2, 1_000, 4));
        h.push(e(3, 2_000, 0));
        assert_eq!(h.next(1, 3_000), Step::Found(e(2, 1_000, 4)));
        assert_eq!(h.next(2, 3_000), Step::Found(e(3, 2_000, 0)));
        assert_eq!(h.next(3, 3_000), Step::NotFound);
        assert_eq!(h.prev(3, 3_000), Step::Found(e(2, 1_000, 4)));
        assert_eq!(h.prev(1, 3_000), Step::NotFound);
    }

    #[test]
    fn readings_older_than_the_horizon_are_dropped() {
        let mut h = History::new();
        h.push(e(1, 0, 0));
        h.push(e(2, 100, 4));
        h.push(e(3, 200, 0));
        assert_eq!(h.next(1, 200 + HORIZON_US + 1), Step::TooOld);
        assert_eq!(h.len(), 1);
        assert_eq!(h.next(3, 200 + HORIZON_US + 1), Step::NotFound);
        assert_eq!(h.prev(3, 200 + HORIZON_US + 1), Step::TooOld);
    }

    #[test]
    fn latest_reading_is_kept_however_old() {
        let mut h = History::new();
        h.push(e(1, 0, 0));
        h.prune(10 * HORIZON_US);
        assert_eq!(h.len(), 1);
        assert_eq!(h.next(1, 10 * HORIZON_US), Step::NotFound);
    }

    #[test]
    fn capacity_is_bounded() {
        let mut h = History::new();
        for i in 0..(MAX_READINGS as u64 + 10) {
            h.push(e(i + 1, i, 0));
        }
        assert_eq!(h.len(), MAX_READINGS);
        assert_eq!(h.next(1, MAX_READINGS as u64 + 10), Step::TooOld);
    }
}
