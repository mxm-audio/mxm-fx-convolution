//! Allocation-free-at-process stereo pre-delay with a bounded two-tap transition.

use crate::convolver::PrepareError;
use crate::fft::flush;

pub(crate) struct Predelay {
    buffers: [Vec<f32>; 2],
    write: usize,
    /// Samples written since reset, capped at the ring length. Reads older than this generation are
    /// zero without clearing the sample-rate-sized buffers.
    valid_samples: usize,
    /// Age of the newest nonzero wet sample while any pre-delay tap could still expose it.
    newest_nonzero_age: Option<usize>,
    current_delay: usize,
    old_delay: usize,
    target_delay: usize,
    transition_position: u64,
    transition_samples: u64,
    ramp_samples: u64,
}

impl Predelay {
    pub fn new(max_delay: usize, delay: usize, ramp_samples: u64) -> Result<Self, PrepareError> {
        let len = max_delay.checked_add(1).ok_or(PrepareError::Allocation)?;
        let delay = delay.min(max_delay);
        let allocate = || {
            let mut buffer = Vec::new();
            buffer
                .try_reserve_exact(len)
                .map_err(|_| PrepareError::Allocation)?;
            buffer.resize(len, 0.0);
            Ok(buffer)
        };
        Ok(Self {
            buffers: [allocate()?, allocate()?],
            write: 0,
            valid_samples: 0,
            newest_nonzero_age: None,
            current_delay: delay,
            old_delay: delay,
            target_delay: delay,
            transition_position: 0,
            transition_samples: 0,
            ramp_samples: ramp_samples.max(1),
        })
    }

    pub fn set_delay_immediate(&mut self, delay: usize) {
        let delay = delay.min(self.buffers[0].len() - 1);
        self.current_delay = delay;
        self.old_delay = delay;
        self.target_delay = delay;
        self.transition_position = 0;
        self.transition_samples = 0;
    }

    /// Move the tap and return a conservative number of future samples for which valid stored wet
    /// history may become audible because of this edit.
    pub fn set_delay(&mut self, delay: usize) -> u64 {
        let delay = delay.min(self.buffers[0].len() - 1);
        if delay == self.target_delay {
            return 0;
        }
        // A new edit supersedes the prior target. Starting from the tap with the greater current
        // weight avoids retaining a stale third tap; the next edit remains bounded to two reads.
        if self.transition_samples != 0 && self.transition_position * 2 >= self.transition_samples {
            self.current_delay = self.target_delay;
        }
        self.old_delay = self.current_delay;
        self.target_delay = delay;
        self.transition_position = 0;
        self.transition_samples = self.ramp_samples;
        if self.newest_nonzero_age.is_some() {
            (self.buffers[0].len() as u64).saturating_add(self.ramp_samples)
        } else {
            0
        }
    }

    #[inline]
    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        self.buffers[0][self.write] = flush(input[0]);
        self.buffers[1][self.write] = flush(input[1]);
        if input[0] != 0.0 || input[1] != 0.0 {
            self.newest_nonzero_age = Some(0);
        } else if let Some(age) = self.newest_nonzero_age {
            let next = age.saturating_add(1);
            self.newest_nonzero_age = (next < self.buffers[0].len()).then_some(next);
        }
        let old_index =
            (self.write + self.buffers[0].len() - self.old_delay) % self.buffers[0].len();
        let target_index =
            (self.write + self.buffers[0].len() - self.target_delay) % self.buffers[0].len();

        let alpha = if self.transition_samples <= 1 {
            1.0
        } else {
            self.transition_position as f32 / (self.transition_samples - 1) as f32
        };
        let old = if self.old_delay <= self.valid_samples {
            [self.buffers[0][old_index], self.buffers[1][old_index]]
        } else {
            [0.0; 2]
        };
        let target = if self.target_delay <= self.valid_samples {
            [self.buffers[0][target_index], self.buffers[1][target_index]]
        } else {
            [0.0; 2]
        };
        let output = [
            flush(old[0] * (1.0 - alpha) + target[0] * alpha),
            flush(old[1] * (1.0 - alpha) + target[1] * alpha),
        ];

        if self.transition_samples != 0 {
            self.transition_position += 1;
            if self.transition_position == self.transition_samples {
                self.current_delay = self.target_delay;
                self.old_delay = self.target_delay;
                self.transition_position = 0;
                self.transition_samples = 0;
            }
        }
        self.write = (self.write + 1) % self.buffers[0].len();
        self.valid_samples = self
            .valid_samples
            .saturating_add(1)
            .min(self.buffers[0].len());
        output
    }

    pub fn reset(&mut self) {
        // Generation validity makes old cells unreadable. This is constant-time even for the
        // maximum pre-delay and prevents stale samples from returning before they are overwritten.
        self.write = 0;
        self.valid_samples = 0;
        self.newest_nonzero_age = None;
        self.current_delay = self.target_delay;
        self.old_delay = self.target_delay;
        self.transition_position = 0;
        self.transition_samples = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_does_not_clear_sample_rate_sized_storage_and_stale_samples_cannot_return() {
        for maximum in [8, 192_000] {
            let mut delay = Predelay::new(maximum, maximum, 8).unwrap();
            delay.buffers[0].fill(0.75);
            delay.buffers[1].fill(-0.5);
            let before = [delay.buffers[0][maximum / 2], delay.buffers[1][maximum / 2]];
            delay.reset();
            assert_eq!(
                [delay.buffers[0][maximum / 2], delay.buffers[1][maximum / 2]],
                before,
                "reset scaled by clearing the delay capacity"
            );
            for _ in 0..=maximum {
                assert_eq!(delay.process([0.0; 2]), [0.0; 2]);
            }
        }
    }
}
