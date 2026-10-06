//! Conservative finite-tail accounting, independent of plugin-framework status types.

/// Framework-free form of the collection's process activity contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Live input, exact silence, or parked Off state.
    Normal,
    /// A finite number of samples may remain audible after the end of the current block.
    Tail(u64),
    /// Nonzero recursive state has no certified finite horizon.
    KeepAlive,
}

/// Recursive contribution to activity, supplied by the Feedback audio core after inspecting its
/// actual loop state. A nonzero Feedback target alone never creates activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecursiveActivity {
    Empty,
    CertifiedTail(u64),
    Uncertified,
}

/// The latest possible response span for input accepted now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TailBounds {
    /// Longest active prepared response, including both transition legs, in samples.
    pub response_samples: u64,
    /// Current pre-delay in samples.
    pub pre_delay_samples: u64,
    /// Conservative extra allowance while response or Mix transitions are audible.
    pub transition_samples: u64,
}

impl TailBounds {
    /// Samples which may remain after the sample receiving a nonzero input.
    pub const fn samples_after_input(self) -> u64 {
        self.response_samples
            .saturating_sub(1)
            .saturating_add(self.pre_delay_samples)
            .saturating_add(self.transition_samples)
    }
}

/// Tracks only audibility bounds; the later convolution engine owns actual histories.
///
/// Loading a response does not call this tracker and therefore creates no tail. Only a nonzero input
/// observation can extend one. [`park`](Self::park) and [`reset`](Self::reset) discard the bound so
/// stale history cannot reappear after Mix-zero Off.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActivityTracker {
    feed_forward_remaining: u64,
    recursive_remaining: u64,
    recursive_uncertified: bool,
    parked: bool,
}

impl ActivityTracker {
    pub const fn new_parked() -> Self {
        Self {
            feed_forward_remaining: 0,
            recursive_remaining: 0,
            recursive_uncertified: false,
            parked: true,
        }
    }

    pub const fn new_active() -> Self {
        Self {
            feed_forward_remaining: 0,
            recursive_remaining: 0,
            recursive_uncertified: false,
            parked: false,
        }
    }

    pub const fn remaining_samples(&self) -> u64 {
        if self.feed_forward_remaining > self.recursive_remaining {
            self.feed_forward_remaining
        } else {
            self.recursive_remaining
        }
    }

    pub const fn is_parked(&self) -> bool {
        self.parked
    }

    pub fn wake_empty(&mut self) {
        self.feed_forward_remaining = 0;
        self.recursive_remaining = 0;
        self.recursive_uncertified = false;
        self.parked = false;
    }

    /// Extend activity for a control edit that can expose already-valid wet history. The count is
    /// measured from the current block boundary and may be conservative, but never shorter than the
    /// latest possible newly selected read.
    pub fn extend(&mut self, samples: u64) {
        if !self.parked {
            self.feed_forward_remaining = self.feed_forward_remaining.max(samples);
        }
    }

    pub fn park(&mut self) {
        self.feed_forward_remaining = 0;
        self.recursive_remaining = 0;
        self.recursive_uncertified = false;
        self.parked = true;
    }

    pub fn reset(&mut self) {
        self.park();
    }

    /// Finish one host block and return the activity at its boundary.
    ///
    /// `last_nonzero_input` is the zero-based index of the last nonzero input frame in the block.
    /// The caller may conservatively classify a very small finite frame as nonzero. If input is
    /// present, activity is `Normal`; otherwise any previous finite tail is counted down by the
    /// complete block. A zero-length block cannot carry an input index.
    pub fn finish_block(
        &mut self,
        block_len: usize,
        last_nonzero_input: Option<usize>,
        bounds: TailBounds,
    ) -> Activity {
        self.finish_block_with_recursive(
            block_len,
            last_nonzero_input,
            bounds,
            RecursiveActivity::Empty,
        )
    }

    /// Finish a block and compose the loop state observed at that block's end with feed-forward
    /// history. The recursive horizon is therefore not decremented for the block which measured it.
    pub fn finish_block_with_recursive(
        &mut self,
        block_len: usize,
        last_nonzero_input: Option<usize>,
        bounds: TailBounds,
        recursive: RecursiveActivity,
    ) -> Activity {
        if self.parked {
            return Activity::Normal;
        }

        self.feed_forward_remaining = self.feed_forward_remaining.saturating_sub(block_len as u64);
        self.recursive_remaining = self.recursive_remaining.saturating_sub(block_len as u64);

        if let Some(index) = last_nonzero_input {
            assert!(
                index < block_len,
                "input index must be inside its host block"
            );
            let samples_after_in_block = (block_len - index - 1) as u64;
            let new_remaining = bounds
                .samples_after_input()
                .saturating_sub(samples_after_in_block);
            self.feed_forward_remaining = self.feed_forward_remaining.max(new_remaining);
        }

        match recursive {
            RecursiveActivity::Empty => {
                self.recursive_remaining = 0;
                self.recursive_uncertified = false;
            }
            RecursiveActivity::CertifiedTail(samples) => {
                self.recursive_remaining = samples;
                self.recursive_uncertified = false;
            }
            RecursiveActivity::Uncertified => self.recursive_uncertified = true,
        }

        if self.recursive_uncertified {
            Activity::KeepAlive
        } else if last_nonzero_input.is_some() {
            Activity::Normal
        } else {
            match self.remaining_samples() {
                0 => Activity::Normal,
                remaining => Activity::Tail(remaining),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: TailBounds = TailBounds {
        response_samples: 9,
        pre_delay_samples: 3,
        transition_samples: 2,
    };

    #[test]
    fn response_loading_alone_never_creates_a_tail() {
        let mut tracker = ActivityTracker::new_active();
        assert_eq!(tracker.finish_block(64, None, BOUNDS), Activity::Normal);
        assert_eq!(tracker.remaining_samples(), 0);
    }

    #[test]
    fn last_input_position_sets_exact_conservative_remaining_span() {
        let mut at_end = ActivityTracker::new_active();
        assert_eq!(at_end.finish_block(8, Some(7), BOUNDS), Activity::Normal);
        assert_eq!(at_end.remaining_samples(), 13);

        let mut at_start = ActivityTracker::new_active();
        assert_eq!(at_start.finish_block(8, Some(0), BOUNDS), Activity::Normal);
        assert_eq!(at_start.remaining_samples(), 6);
        assert_eq!(at_start.finish_block(4, None, BOUNDS), Activity::Tail(2));
        assert_eq!(at_start.finish_block(2, None, BOUNDS), Activity::Normal);
    }

    #[test]
    fn tail_accounting_is_host_block_partition_invariant() {
        let mut whole = ActivityTracker::new_active();
        whole.finish_block(8, Some(2), BOUNDS);

        let mut split = ActivityTracker::new_active();
        split.finish_block(3, Some(2), BOUNDS);
        split.finish_block(5, None, BOUNDS);

        assert_eq!(whole.remaining_samples(), split.remaining_samples());
        assert_eq!(whole.remaining_samples(), 8);
    }

    #[test]
    fn newer_input_can_only_extend_the_bound() {
        let mut tracker = ActivityTracker::new_active();
        tracker.finish_block(1, Some(0), BOUNDS);
        assert_eq!(tracker.remaining_samples(), 13);

        let shorter = TailBounds {
            response_samples: 2,
            pre_delay_samples: 0,
            transition_samples: 0,
        };
        tracker.finish_block(1, Some(0), shorter);
        assert_eq!(tracker.remaining_samples(), 12);
    }

    #[test]
    fn off_reset_and_wake_discard_stale_tail() {
        let mut tracker = ActivityTracker::new_active();
        tracker.finish_block(1, Some(0), BOUNDS);
        tracker.park();
        assert!(tracker.is_parked());
        assert_eq!(tracker.remaining_samples(), 0);
        assert_eq!(tracker.finish_block(1, Some(0), BOUNDS), Activity::Normal);
        assert_eq!(tracker.remaining_samples(), 0);

        tracker.wake_empty();
        assert!(!tracker.is_parked());
        assert_eq!(tracker.finish_block(1, None, BOUNDS), Activity::Normal);
    }

    #[test]
    fn recursive_activity_distinguishes_certified_and_uncertified_state() {
        let mut tracker = ActivityTracker::new_active();
        assert_eq!(
            tracker
                .finish_block_with_recursive(4, Some(3), BOUNDS, RecursiveActivity::Uncertified,),
            Activity::KeepAlive
        );

        assert_eq!(
            tracker.finish_block_with_recursive(
                4,
                None,
                BOUNDS,
                RecursiveActivity::CertifiedTail(20),
            ),
            Activity::Tail(20)
        );
        assert_eq!(
            tracker.finish_block_with_recursive(16, None, BOUNDS, RecursiveActivity::Empty),
            Activity::Normal
        );
    }

    #[test]
    fn park_and_wake_clear_uncertified_recursive_state() {
        let mut tracker = ActivityTracker::new_active();
        tracker.finish_block_with_recursive(1, None, BOUNDS, RecursiveActivity::Uncertified);
        tracker.park();
        assert_eq!(tracker.finish_block(1, None, BOUNDS), Activity::Normal);
        tracker.wake_empty();
        assert_eq!(tracker.finish_block(1, None, BOUNDS), Activity::Normal);
    }

    #[test]
    fn reset_is_deterministic() {
        let mut first = ActivityTracker::new_active();
        first.finish_block(4, Some(3), BOUNDS);
        first.reset();
        first.wake_empty();

        let mut second = ActivityTracker::new_parked();
        second.wake_empty();
        assert_eq!(first.remaining_samples(), second.remaining_samples());
        assert_eq!(
            first.finish_block(4, Some(1), BOUNDS),
            second.finish_block(4, Some(1), BOUNDS)
        );
        assert_eq!(first.remaining_samples(), second.remaining_samples());
    }
}
