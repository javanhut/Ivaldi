//! Binary search through commit history for bug-finding.
//!
//! Bisect narrows down which commit introduced a bug by binary search:
//! 1. Mark a "good" commit (before the bug)
//! 2. Mark a "bad" commit (after the bug)
//! 3. Ivaldi checks out the midpoint
//! 4. User tests and marks good/bad
//! 5. Repeat until the offending commit is found.


/// Bisect session state.
#[derive(Debug, Clone)]
pub struct BisectState {
    /// Known good commit index.
    pub good: u64,
    /// Known bad commit index.
    pub bad: u64,
    /// All commit indices between good and bad (inclusive), oldest first.
    pub range: Vec<u64>,
    /// Current position being tested.
    pub current: usize,
    /// Indices confirmed good by the user.
    pub confirmed_good: Vec<u64>,
    /// Indices confirmed bad by the user.
    pub confirmed_bad: Vec<u64>,
    /// Whether bisect is complete.
    pub done: bool,
    /// The found offending commit (when done).
    pub result: Option<u64>,
}

impl BisectState {
    /// Start a new bisect between good and bad commits.
    /// `range` should be the commit indices from good to bad in order.
    pub fn new(good: u64, bad: u64, range: Vec<u64>) -> Result<Self, BisectError> {
        if range.len() < 2 {
            return Err(BisectError::TooFewCommits);
        }
        let mid = range.len() / 2;
        Ok(Self {
            good,
            bad,
            range,
            current: mid,
            confirmed_good: vec![good],
            confirmed_bad: vec![bad],
            done: false,
            result: None,
        })
    }

    /// Get the current commit index to test.
    pub fn current_commit(&self) -> u64 {
        self.range[self.current]
    }

    /// Remaining steps (worst case).
    pub fn remaining_steps(&self) -> u32 {
        let n = self.search_range_size();
        if n <= 1 { 0 } else { (n as f64).log2().ceil() as u32 }
    }

    /// Mark the current commit as good.
    pub fn mark_good(&mut self) {
        let idx = self.range[self.current];
        self.confirmed_good.push(idx);
        self.narrow_range();
    }

    /// Mark the current commit as bad.
    pub fn mark_bad(&mut self) {
        let idx = self.range[self.current];
        self.confirmed_bad.push(idx);
        self.narrow_range();
    }

    fn search_range_size(&self) -> usize {
        let latest_good = self.confirmed_good.iter().max().copied().unwrap_or(0);
        let earliest_bad = self.confirmed_bad.iter().min().copied().unwrap_or(u64::MAX);
        self.range.iter().filter(|&&i| i > latest_good && i < earliest_bad).count()
    }

    fn narrow_range(&mut self) {
        let latest_good = self.confirmed_good.iter().max().copied().unwrap_or(0);
        let earliest_bad = self.confirmed_bad.iter().min().copied().unwrap_or(u64::MAX);

        // Find remaining candidates
        let candidates: Vec<u64> = self.range.iter()
            .filter(|&&i| i > latest_good && i < earliest_bad)
            .copied()
            .collect();

        if candidates.is_empty() {
            // The first bad commit is the answer
            self.done = true;
            self.result = Some(earliest_bad);
        } else if candidates.len() == 1 {
            // Only one candidate left
            self.done = true;
            self.result = Some(candidates[0]);
        } else {
            // Pick midpoint of remaining candidates
            let mid_idx = candidates.len() / 2;
            let target = candidates[mid_idx];
            self.current = self.range.iter().position(|&i| i == target).unwrap_or(0);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BisectError {
    #[error("need at least 2 commits to bisect")]
    TooFewCommits,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_bisect() {
        let range: Vec<u64> = (0..10).collect();
        let mut state = BisectState::new(0, 9, range).unwrap();

        assert!(!state.done);
        assert_eq!(state.current_commit(), 5); // midpoint

        state.mark_good(); // 5 is good
        assert!(!state.done);
        // Should now test between 5 and 9

        state.mark_bad(); // next midpoint is bad
        // Narrows further
    }

    #[test]
    fn bisect_small_range() {
        let range = vec![0, 1, 2];
        let mut state = BisectState::new(0, 2, range).unwrap();
        assert_eq!(state.current_commit(), 1);

        state.mark_bad(); // 1 is bad
        assert!(state.done);
        assert_eq!(state.result, Some(1));
    }

    #[test]
    fn bisect_finds_first_bad() {
        let range: Vec<u64> = (0..8).collect();
        let mut state = BisectState::new(0, 7, range).unwrap();

        // Simulate: bug introduced at commit 3
        while !state.done {
            if state.current_commit() < 3 {
                state.mark_good();
            } else {
                state.mark_bad();
            }
        }
        assert_eq!(state.result, Some(3));
    }

    #[test]
    fn remaining_steps() {
        let range: Vec<u64> = (0..16).collect();
        let state = BisectState::new(0, 15, range).unwrap();
        assert!(state.remaining_steps() <= 4); // log2(14) ≈ 3.8
    }

    #[test]
    fn too_few_commits() {
        let result = BisectState::new(0, 0, vec![0]);
        assert!(result.is_err());
    }
}
