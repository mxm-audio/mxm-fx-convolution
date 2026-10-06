//! Revision 6's single post-response wet path.
//!
//! Signal order is fixed by the approved plan: Wet EQ, bounded modulated delay, then Width. The EQ
//! and width matrix are project-derived linear operations. The movement stage is a linear
//! time-varying fractional delay; it does not animate or replace prepared FIR coefficients.

use crate::control::{HIGH_CUT_BYPASS_HZ, WetPostControlFrame, WetPostTailConfig};
use crate::fft::flush;

/// Pre-listening provisional rate, intended as slow room motion rather than vibrato. This value
/// has not been selected by listening and awaits owner approval.
pub const MODULATION_RATE_HZ: f32 = 0.23;
/// Pre-listening provisional excursion: at full amount the wet read moves from 1 to 7 ms. This
/// value awaits owner approval. Scaling the whole delay by amount makes zero converge continuously
/// to the exact bypass rather than retaining latency.
pub const MODULATION_MIN_DELAY_S: f32 = 0.001;
pub const MODULATION_MAX_DELAY_S: f32 = 0.007;
/// Pre-listening conservative first-order settling window. The enabled low cut is clamped to
/// 20 Hz; 200 ms is about 25 time constants and falls below the crate's quiet numeric seam. The
/// Wet EQ values have not been selected by listening and await owner approval.
pub const WET_EQ_SETTLE_S: f32 = 0.200;
const MIN_LOW_CUT_HZ: f32 = 20.0;
const MAX_LOW_CUT_HZ: f32 = 4_000.0;
const TONE_SPLIT_HZ: f32 = 1_000.0;
const MAX_TONE_DB: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WetPostPathError {
    InvalidSampleRate,
    Allocation,
}

#[derive(Debug, Clone, Copy, Default)]
struct OnePoleState {
    hp_x: f32,
    hp_y: f32,
    low_pass: f32,
    tone_low: f32,
}

impl OnePoleState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

struct WetEq {
    sample_rate: f32,
    channels: [OnePoleState; 2],
}

impl WetEq {
    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            channels: [OnePoleState::default(); 2],
        }
    }

    #[inline]
    fn process(&mut self, input: [f32; 2], controls: WetPostControlFrame) -> [f32; 2] {
        if controls.wet_eq.is_neutral() {
            self.channels[0].reset();
            self.channels[1].reset();
            return input;
        }
        let mut output = [0.0; 2];
        for channel in 0..2 {
            let mut value = input[channel];
            let state = &mut self.channels[channel];

            if controls.wet_eq.low_cut_hz > 0.0 {
                let ceiling = self.sample_rate * 0.45;
                let cutoff = controls
                    .wet_eq
                    .low_cut_hz
                    .clamp(MIN_LOW_CUT_HZ.min(ceiling), MAX_LOW_CUT_HZ.min(ceiling));
                let pole = (-core::f32::consts::TAU * cutoff / self.sample_rate).exp();
                let next = pole * (state.hp_y + value - state.hp_x);
                state.hp_x = flush(value);
                state.hp_y = flush(next);
                value = state.hp_y;
            } else {
                state.hp_x = 0.0;
                state.hp_y = 0.0;
            }

            if controls.wet_eq.high_cut_hz == HIGH_CUT_BYPASS_HZ {
                state.low_pass = 0.0;
            } else if controls.wet_eq.high_cut_hz == 0.0 {
                // Zero is a real fully closed cutoff, not Open. Clear stored energy so arriving at
                // the endpoint cannot hold the previous low-pass sample forever.
                state.low_pass = 0.0;
                value = 0.0;
            } else {
                let ceiling = self.sample_rate * 0.45;
                let cutoff = controls.wet_eq.high_cut_hz.clamp(0.0, ceiling);
                let amount = 1.0 - (-core::f32::consts::TAU * cutoff / self.sample_rate).exp();
                state.low_pass = flush(state.low_pass + amount * (value - state.low_pass));
                value = state.low_pass;
            }

            if controls.wet_eq.tone_db != 0.0 {
                let amount = 1.0
                    - (-core::f32::consts::TAU * TONE_SPLIT_HZ.min(self.sample_rate * 0.45)
                        / self.sample_rate)
                        .exp();
                state.tone_low = flush(state.tone_low + amount * (value - state.tone_low));
                let high = value - state.tone_low;
                let gain =
                    10.0f32.powf(controls.wet_eq.tone_db.clamp(-MAX_TONE_DB, MAX_TONE_DB) / 40.0);
                value = state.tone_low / gain + high * gain;
            } else {
                state.tone_low = 0.0;
            }
            output[channel] = flush(value);
        }
        output
    }

    fn reset(&mut self) {
        self.channels[0].reset();
        self.channels[1].reset();
    }
}

struct ModulatedDelay {
    buffers: [Vec<f32>; 2],
    write: usize,
    valid_samples: usize,
    phase: f32,
    phase_step: f32,
    sample_rate: f32,
    max_delay_samples: usize,
}

impl ModulatedDelay {
    fn try_new(sample_rate: f32) -> Result<Self, WetPostPathError> {
        let phase_step = MODULATION_RATE_HZ / sample_rate;
        if !phase_step.is_finite() || phase_step >= 1.0 {
            return Err(WetPostPathError::InvalidSampleRate);
        }
        let required_wide = f64::from(MODULATION_MAX_DELAY_S) * f64::from(sample_rate);
        if required_wide.ceil() > (usize::MAX - 3) as f64 {
            return Err(WetPostPathError::Allocation);
        }
        // Match the audio/control f32 time-to-sample rounding law after the wide overflow guard.
        let max_delay_samples = (MODULATION_MAX_DELAY_S * sample_rate).ceil() as usize + 2;
        let len = max_delay_samples + 1;
        let allocate = || {
            let mut values = Vec::new();
            values
                .try_reserve_exact(len)
                .map_err(|_| WetPostPathError::Allocation)?;
            values.resize(len, 0.0);
            Ok(values)
        };
        Ok(Self {
            buffers: [allocate()?, allocate()?],
            write: 0,
            valid_samples: 0,
            phase: 0.0,
            phase_step,
            sample_rate,
            max_delay_samples,
        })
    }

    #[inline]
    fn process(&mut self, input: [f32; 2], amount: f32) -> [f32; 2] {
        self.buffers[0][self.write] = flush(input[0]);
        self.buffers[1][self.write] = flush(input[1]);
        if amount == 0.0 {
            self.phase = 0.0;
            self.advance_write();
            return input;
        }

        let mut output = [0.0; 2];
        for (channel, value) in output.iter_mut().enumerate() {
            let phase = (self.phase + channel as f32 * 0.25).fract();
            let lfo = 0.5 + 0.5 * (core::f32::consts::TAU * phase).sin();
            let delay_s = amount
                * (MODULATION_MIN_DELAY_S
                    + (MODULATION_MAX_DELAY_S - MODULATION_MIN_DELAY_S) * lfo);
            *value = self.read(channel, delay_s * self.sample_rate);
        }
        self.phase += self.phase_step;
        self.phase -= self.phase.floor();
        self.advance_write();
        output.map(flush)
    }

    #[inline]
    fn read(&self, channel: usize, delay_samples: f32) -> f32 {
        let delay = delay_samples.clamp(0.0, self.max_delay_samples as f32);
        let younger = delay.floor() as usize;
        let older = (younger + 1).min(self.max_delay_samples);
        let fraction = delay - younger as f32;
        let sample = |age: usize| {
            if age <= self.valid_samples {
                let index =
                    (self.write + self.buffers[channel].len() - age) % self.buffers[channel].len();
                self.buffers[channel][index]
            } else {
                0.0
            }
        };
        sample(younger) * (1.0 - fraction) + sample(older) * fraction
    }

    #[inline]
    fn advance_write(&mut self) {
        self.write = (self.write + 1) % self.buffers[0].len();
        self.valid_samples = self
            .valid_samples
            .saturating_add(1)
            .min(self.buffers[0].len());
    }

    fn reset(&mut self) {
        self.write = 0;
        self.valid_samples = 0;
        self.phase = 0.0;
    }
}

/// Single-instance Revision 6 stages downstream of the old/new response crossfade.
pub(crate) struct WetPostPath {
    eq: WetEq,
    modulation: ModulatedDelay,
    remaining: u64,
    eq_settle_samples: u64,
}

impl WetPostPath {
    pub fn try_new(sample_rate: f32) -> Result<Self, WetPostPathError> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(WetPostPathError::InvalidSampleRate);
        }
        Ok(Self {
            eq: WetEq::new(sample_rate),
            modulation: ModulatedDelay::try_new(sample_rate)?,
            remaining: 0,
            eq_settle_samples: (WET_EQ_SETTLE_S * sample_rate).ceil() as u64,
        })
    }

    pub fn tail_config(&self) -> WetPostTailConfig {
        WetPostTailConfig {
            wet_eq_settle_samples: self.eq_settle_samples,
            modulation_history_samples: self.modulation.max_delay_samples as u64,
        }
    }

    #[inline]
    pub fn process(&mut self, input: [f32; 2], controls: WetPostControlFrame) -> [f32; 2] {
        let eq_active = !controls.wet_eq.is_neutral();
        let modulation_active = controls.modulation != 0.0;
        if input[0] != 0.0 || input[1] != 0.0 {
            self.remaining = (eq_active as u64 * self.eq_settle_samples).saturating_add(
                modulation_active as u64 * self.modulation.max_delay_samples as u64,
            );
        } else if self.remaining == 0 {
            self.reset_history();
            return [0.0; 2];
        } else {
            self.remaining -= 1;
        }

        let equalised = self.eq.process(input, controls);
        let moved = self.modulation.process(equalised, controls.modulation);
        width(moved, controls.width)
    }

    pub fn reset_history(&mut self) {
        self.eq.reset();
        self.modulation.reset();
        self.remaining = 0;
    }
}

/// Project-derived mid/side matrix. The exact neutral branch avoids a round-trip rounding change.
#[inline]
pub(crate) fn width(input: [f32; 2], side_gain: f32) -> [f32; 2] {
    if side_gain == 1.0 {
        return input;
    }
    let mid = (input[0] + input[1]) * 0.5;
    let side = (input[0] - input[1]) * 0.5 * side_gain;
    [flush(mid + side), flush(mid - side)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::WetEqControls;

    fn frame() -> WetPostControlFrame {
        WetPostControlFrame {
            wet_eq: WetEqControls::default(),
            modulation: 0.0,
            width: 1.0,
        }
    }

    #[test]
    fn neutral_path_is_bit_identical() {
        let mut path = WetPostPath::try_new(48_000.0).unwrap();
        for input in [[0.25, -0.75], [f32::MIN_POSITIVE, -0.0], [0.0, 0.0]] {
            assert_eq!(path.process(input, frame()), input);
        }
    }

    #[test]
    fn width_uses_the_output_pair() {
        assert_eq!(width([0.75, -0.25], 0.0), [0.25, 0.25]);
        assert_eq!(width([0.75, -0.25], 2.0), [1.25, -0.75]);
        assert_eq!(width([0.4, 0.4], 8.0), [0.4, 0.4]);
    }

    #[test]
    fn fractional_delay_matches_a_scalar_linear_interpolation_oracle() {
        let rate = 1_000.0;
        let amount = 0.8;
        let mut delay = ModulatedDelay::try_new(rate).unwrap();
        let mut history = Vec::<[f32; 2]>::new();
        let mut phase = 0.0f32;
        for index in 0usize..100 {
            let input = [index as f32 * 0.01, -(index as f32) * 0.02];
            history.push(input);
            let actual = delay.process(input, amount);
            for channel in 0..2 {
                let channel_phase = (phase + channel as f32 * 0.25).fract();
                let lfo = 0.5 + 0.5 * (core::f32::consts::TAU * channel_phase).sin();
                let delay_samples = amount
                    * (MODULATION_MIN_DELAY_S
                        + (MODULATION_MAX_DELAY_S - MODULATION_MIN_DELAY_S) * lfo)
                    * rate;
                let younger = delay_samples.floor() as usize;
                let older = younger + 1;
                let fraction = delay_samples - younger as f32;
                let sample = |age: usize| {
                    index
                        .checked_sub(age)
                        .map_or(0.0, |at| history[at][channel])
                };
                let expected = sample(younger) * (1.0 - fraction) + sample(older) * fraction;
                assert!((actual[channel] - expected).abs() < 1.0e-6);
            }
            phase += MODULATION_RATE_HZ / rate;
            phase -= phase.floor();
        }
    }

    #[test]
    fn zero_high_cut_is_fully_closed_while_the_private_sentinel_is_bypass() {
        let mut closed = WetPostPath::try_new(48_000.0).unwrap();
        let mut controls = frame();
        controls.wet_eq.high_cut_hz = 0.0;
        assert_eq!(closed.process([0.75, -0.25], controls), [0.0, 0.0]);
        assert_eq!(
            WetPostPath::try_new(48_000.0)
                .unwrap()
                .process([0.75, -0.25], frame()),
            [0.75, -0.25]
        );
    }

    #[test]
    fn wet_eq_removes_dc_and_attenuates_nyquist_without_non_finite_state() {
        let mut low_cut = WetPostPath::try_new(48_000.0).unwrap();
        let mut high_cut = WetPostPath::try_new(48_000.0).unwrap();
        let mut low_controls = frame();
        low_controls.wet_eq.low_cut_hz = 100.0;
        let mut high_controls = frame();
        high_controls.wet_eq.high_cut_hz = 1_000.0;
        let mut last_dc = 1.0;
        let mut high_energy = 0.0;
        for index in 0..4_000 {
            last_dc = low_cut.process([1.0; 2], low_controls)[0];
            let alternating = if index & 1 == 0 { 1.0 } else { -1.0 };
            let filtered = high_cut.process([alternating; 2], high_controls)[0];
            if index >= 3_500 {
                high_energy += filtered * filtered;
            }
            assert!(last_dc.is_finite() && filtered.is_finite());
        }
        assert!(last_dc.abs() < 1.0e-5);
        assert!((high_energy / 500.0).sqrt() < 0.1);
    }

    #[test]
    fn modulation_is_bounded_finite_and_reset_deterministic() {
        let mut first = WetPostPath::try_new(48_000.0).unwrap();
        let mut second = WetPostPath::try_new(48_000.0).unwrap();
        let mut controls = frame();
        controls.modulation = 1.0;
        for index in 0..2_000 {
            let input = if index == 0 { [1.0, -1.0] } else { [0.0; 2] };
            let a = first.process(input, controls);
            let b = second.process(input, controls);
            assert_eq!(a, b);
            assert!(a.into_iter().all(f32::is_finite));
        }
        first.reset_history();
        for _ in 0..first.modulation.max_delay_samples + 2 {
            assert_eq!(first.process([0.0; 2], controls), [0.0; 2]);
        }
    }

    #[test]
    fn hostile_subnormal_input_leaves_no_subnormal_post_state() {
        let mut path = WetPostPath::try_new(48_000.0).unwrap();
        let mut controls = frame();
        controls.wet_eq = WetEqControls {
            low_cut_hz: 20.0,
            high_cut_hz: 200.0,
            tone_db: 6.0,
        };
        controls.modulation = 1.0;
        controls.width = 2.0;
        for _ in 0..1_000 {
            assert_eq!(
                path.process([f32::from_bits(1), -f32::from_bits(1)], controls),
                [0.0, 0.0]
            );
        }
        for state in path.eq.channels {
            assert!(normal_or_zero(state.hp_x));
            assert!(normal_or_zero(state.hp_y));
            assert!(normal_or_zero(state.low_pass));
            assert!(normal_or_zero(state.tone_low));
        }
        assert!(
            path.modulation
                .buffers
                .iter()
                .flatten()
                .all(|value| normal_or_zero(*value))
        );
    }

    fn normal_or_zero(value: f32) -> bool {
        value == 0.0 || value.is_normal()
    }

    #[test]
    fn invalid_rate_is_rejected_without_allocation() {
        assert!(matches!(
            WetPostPath::try_new(f32::NAN),
            Err(WetPostPathError::InvalidSampleRate)
        ));
        assert!(matches!(
            WetPostPath::try_new(0.0),
            Err(WetPostPathError::InvalidSampleRate)
        ));
        assert!(matches!(
            WetPostPath::try_new(f32::MIN_POSITIVE),
            Err(WetPostPathError::InvalidSampleRate)
        ));
        assert!(matches!(
            WetPostPath::try_new(f32::MAX),
            Err(WetPostPathError::Allocation)
        ));
    }
}
