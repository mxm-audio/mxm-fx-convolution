//! Allocation-free-at-process stereo pre-delay with a bounded two-tap transition.

use crate::convolver::PrepareError;
use crate::fft::flush;
use mxm_dsp::delay::AgeLine;

pub(crate) struct Predelay {
    /// One `mxm_dsp::delay::AgeLine` a channel, `max_delay + 1` long: reads older than what was
    /// written since reset are zero without clearing the sample-rate-sized buffers. The input is
    /// pushed before the taps are read, so a delay of `d` samples (0 = the current input) is the
    /// line's age `d + 1`.
    lines: [AgeLine; 2],
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
        let allocate = || AgeLine::try_new(len).map_err(|_| PrepareError::Allocation);
        Ok(Self {
            lines: [allocate()?, allocate()?],
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
        let delay = delay.min(self.lines[0].capacity() - 1);
        self.current_delay = delay;
        self.old_delay = delay;
        self.target_delay = delay;
        self.transition_position = 0;
        self.transition_samples = 0;
    }

    /// Move the tap and return a conservative number of future samples for which valid stored wet
    /// history may become audible because of this edit.
    pub fn set_delay(&mut self, delay: usize) -> u64 {
        let delay = delay.min(self.lines[0].capacity() - 1);
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
            (self.lines[0].capacity() as u64).saturating_add(self.ramp_samples)
        } else {
            0
        }
    }

    #[inline]
    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        self.lines[0].push(flush(input[0]));
        self.lines[1].push(flush(input[1]));
        if input[0] != 0.0 || input[1] != 0.0 {
            self.newest_nonzero_age = Some(0);
        } else if let Some(age) = self.newest_nonzero_age {
            let next = age.saturating_add(1);
            self.newest_nonzero_age = (next < self.lines[0].capacity()).then_some(next);
        }
        let alpha = if self.transition_samples <= 1 {
            1.0
        } else {
            self.transition_position as f32 / (self.transition_samples - 1) as f32
        };
        let tap = |delay: usize| [self.lines[0].read(delay + 1), self.lines[1].read(delay + 1)];
        let old = tap(self.old_delay);
        let target = tap(self.target_delay);
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
        output
    }

    pub fn reset(&mut self) {
        // Generation validity makes old cells unreadable. This is constant-time even for the
        // maximum pre-delay and prevents stale samples from returning before they are overwritten.
        self.lines[0].clear();
        self.lines[1].clear();
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
        // Reset invalidates the lines in constant time (`AgeLine::clear` touches no sample); what
        // this crate must guarantee is that nothing written before it ever returns.
        for maximum in [8, 192_000] {
            let mut delay = Predelay::new(maximum, maximum, 8).unwrap();
            for _ in 0..=maximum {
                delay.process([0.75, -0.5]);
            }
            delay.reset();
            for _ in 0..=maximum {
                assert_eq!(delay.process([0.0; 2]), [0.0; 2]);
            }
        }
    }
}
