//! Revision 7's bounded return-only nonlinearity and explicit one-sample loop delay.

use crate::fft::flush;

/// Ceiling for the in-loop `A * tanh(v / A)` saturator.
///
/// `A` is both the multiplier and the divisor, so it cancels exactly at small signal and acts only
/// once the loop is driven into saturation. That makes it a pure *ceiling* control: it sets how loud
/// self-oscillation settles without moving the oscillation threshold, so the owner's "a displayed
/// 1.00 is the edge" contract survives whatever is chosen here.
///
/// Measured on the 2 s room-scale response at mix 0.38 against a 0.25-peak sine, wet peak at the
/// 1.25 maximum: `1.0 -> 31.57`, `0.5 -> 24.92`, `0.25 -> 15.79`, `0.125 -> 7.76`. Across that whole
/// 8x range the q=1.00 threshold reading moved 0.5% (6.437 -> 6.407), confirming that this ceiling
/// can be tuned independently of the small-signal Nyquist edge.
///
/// `0.25` is the chosen trade: 6 dB below the unbounded ceiling, but still a 2.8x rise over the 5.60
/// feedback-off level, so oscillation stays an unmistakable event rather than a tame swell. `0.5`
/// was rejected as inaudible against `1.0` (2 dB); `0.125` as too tame to be worth a dial.
///
/// This bounds the *return inside the loop only*. It is not an output limiter and no other stage may
/// reuse it as one, nor as an automatic gain law. It does not address the separate fact that an
/// un-normalized response already puts the wet leg well above the dry at feedback zero.
pub const FEEDBACK_SATURATION_SCALE: f32 = 0.25;

// There is deliberately no global threshold calibration. Preparation measures the positive-real
// Nyquist crossing of each complete response loop, including damping and the explicit sample delay.
// Dividing by that response-specific value puts the linear edge at a displayed 1.0 directly. The old
// magnitude normalization plus a fitted 1.2 multiplier confused a contraction bound with a gain
// margin and could classify an already-unstable loop as a finite tail.

#[derive(Debug, Clone, Copy)]
/// One pole down, one pole up, in the loop only.
///
/// Ported from `mxm-shimmer-dsp`'s `LoopFilter`, which is the shipped, working pattern for a
/// regenerating loop in this project. Without it a convolution loop is dominated by whichever two
/// bins the impulse peaks at: an un-normalised room response can peak near 270 while its average is
/// far lower, so those bins cross unity and scream while the body of the reverb never reaches it.
/// The owner heard exactly that - "immediately infinite and gets limited, just a low and a high
/// pitch". Filtering each pass is what stops a resonance running away with the whole loop.
#[derive(Default)]
struct LoopFilter {
    lp: f32,
    hp: f32,
    previous: f32,
}

impl LoopFilter {
    #[inline]
    fn process(&mut self, input: f32, low_cut: f32, high_cut: f32, sample_rate: f32) -> f32 {
        let lp_a = 1.0 - (-core::f32::consts::TAU * high_cut / sample_rate).exp();
        self.lp += lp_a * (input - self.lp);
        let hp_a = (-core::f32::consts::TAU * low_cut / sample_rate).exp();
        self.hp = hp_a * (self.hp + self.lp - self.previous);
        self.previous = self.lp;
        // An exact-zero seam, without which a contractive loop never reports idle.
        //
        // The highpass recursion decays geometrically but crosses the subnormal seam far too slowly
        // for `flush` to catch: fed exact zeros at 44.1 kHz it still holds 1.6e-23 after two
        // thousand samples, against an `f32::MIN_POSITIVE` of 1.2e-38. Because `is_active` is true
        // for any nonzero return, the engine's recursive state would never clear, the declared tail
        // would never be honoured, and a host trusting it would keep the voice alive forever. Below
        // this threshold the state is inaudible at any Mix, so it is snapped to exact zero.
        // Keyed on magnitude alone, deliberately. Requiring `input == 0.0` never fired: the filter's
        // input is the convolver's raw return, which decays through ever-smaller nonzero values
        // rather than snapping to zero, so the seam was unreachable in the real signal path.
        const IDLE_SEAM: f32 = 1.0e-20;
        if self.lp.abs() < IDLE_SEAM && self.hp.abs() < IDLE_SEAM && input.abs() < IDLE_SEAM {
            *self = Self::default();
            return 0.0;
        }
        flush(self.hp)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Loop damping, fixed rather than exposed until the owner has heard it.
///
/// These are `mxm-shimmer-dsp`'s shipped loop-filter defaults, adopted deliberately so this loop
/// sits in the same territory as an effect the owner has already approved rather than in one this
/// crate invented. Shimmer's own values were chosen by measurement against a stated criterion - its
/// `OUTER_REGEN_SCALE` comment records the sweep - and the same method applies here before either
/// number is treated as tuned.
/// Shared with `convolver`, which must weight the return spectrum by this same damping to measure
/// the loop's actual gain. A bound taken from the undamped spectrum measures a resonance the loop
/// never sees, because every pass goes through these two poles first.
pub(crate) const LOOP_HIGH_CUT_HZ: f32 = 8_000.0;
/// See [`LOOP_HIGH_CUT_HZ`].
pub(crate) const LOOP_LOW_CUT_HZ: f32 = 180.0;

/// The loop damping corners, for proofs that must model what the return actually does.
#[cfg(test)]
pub fn loop_high_cut_hz() -> f32 {
    LOOP_HIGH_CUT_HZ
}

/// See [`loop_high_cut_hz`].
#[cfg(test)]
pub fn loop_low_cut_hz() -> f32 {
    LOOP_LOW_CUT_HZ
}

pub(crate) struct FeedbackLoop {
    delayed_return: [f32; 2],
    current_q: f32,
    current_contraction_bound: f32,
    filters: [LoopFilter; 2],
    sample_rate: f32,
}

impl FeedbackLoop {
    pub const fn new() -> Self {
        Self {
            delayed_return: [0.0; 2],
            current_q: 0.0,
            current_contraction_bound: 0.0,
            filters: [LoopFilter {
                lp: 0.0,
                hp: 0.0,
                previous: 0.0,
            }; 2],
            // Replaced by `set_sample_rate` at activation; 48 kHz keeps the coefficient finite for
            // any engine constructed before a rate is known.
            sample_rate: 48_000.0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate > 0.0 {
            self.sample_rate = sample_rate;
        }
    }

    /// Return the excitation computed from the previous raw response sample. Calling this method is
    /// intentionally avoided by the engine's exact-zero branch.
    #[inline]
    pub const fn delayed_return(&self) -> [f32; 2] {
        self.delayed_return
    }

    /// Compute `sat_A((q / H_edge) R y[n])` for use at sample `n + 1`.
    ///
    /// `H_edge` is the prepared response's positive-real Nyquist crossing, so `q = 1` is the
    /// small-signal oscillation boundary. `contraction_peak` is the separate H-infinity magnitude
    /// bound; it never sets the sound, only whether the current recursive tail can be certified.
    /// The complete multiply/divide is evaluated in `f64`, so every finite `f32` control, bound and
    /// raw response remains finite before `tanh`; narrowing happens only after the bounded curve.
    #[inline]
    pub fn update(
        &mut self,
        routed_raw: [f64; 2],
        q: f32,
        gain_divisor: f32,
        contraction_peak: f32,
    ) {
        self.current_q = q;
        if q == 0.0 || gain_divisor == 0.0 {
            self.current_contraction_bound = 0.0;
            self.delayed_return = [0.0; 2];
            return;
        }
        self.current_contraction_bound =
            (f64::from(q) * f64::from(contraction_peak) / f64::from(gain_divisor)) as f32;

        let scale = f64::from(FEEDBACK_SATURATION_SCALE);
        let gain_over_scale = f64::from(q) / (f64::from(gain_divisor) * scale);
        let sample_rate = self.sample_rate;
        // Damp first, then saturate. Filtering the return is what keeps a single resonance from
        // taking the whole loop while the body of the response stays below unity.
        for ((out, filter), raw) in self
            .delayed_return
            .iter_mut()
            .zip(self.filters.iter_mut())
            .zip(routed_raw.iter().copied())
        {
            let damped = filter.process(raw as f32, LOOP_LOW_CUT_HZ, LOOP_HIGH_CUT_HZ, sample_rate);
            let bounded = scale * (gain_over_scale * f64::from(damped)).tanh();
            *out = flush(bounded as f32);
        }
    }

    pub fn reset(&mut self) {
        self.delayed_return = [0.0; 2];
        self.current_q = 0.0;
        self.current_contraction_bound = 0.0;
        self.filters[0].reset();
        self.filters[1].reset();
    }

    pub fn is_active(&self) -> bool {
        self.delayed_return[0] != 0.0 || self.delayed_return[1] != 0.0
    }

    /// Samples the loop damping itself needs to fall to the exact-zero seam.
    ///
    /// The declared tail is otherwise derived from `q` alone as a pure geometric decay, which was
    /// true before the loop carried a filter. It no longer is: measured on a one-tap at `q = 0.5`
    /// the horizon said 254 samples while the audio was still nonzero at 1421. Under-declaring is
    /// the dangerous direction - a host that believes it cuts the tail off - so the filter's own
    /// settling has to be part of the promise.
    pub fn settling_samples(&self) -> u64 {
        let pole = (-core::f32::consts::TAU * LOOP_LOW_CUT_HZ / self.sample_rate).exp();
        if !(0.0..1.0).contains(&pole) || pole == 0.0 {
            return 0;
        }
        const IDLE_SEAM: f64 = 1.0e-20;
        let samples = IDLE_SEAM.ln() / f64::from(pole).ln();
        if samples.is_finite() && samples > 0.0 {
            samples.ceil() as u64
        } else {
            0
        }
    }

    pub const fn current_q(&self) -> f32 {
        self.current_q
    }

    pub const fn current_contraction_bound(&self) -> f32 {
        self.current_contraction_bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_is_odd_bounded_and_uses_an_explicit_sample_delay() {
        let mut feedback = FeedbackLoop::new();
        assert_eq!(feedback.delayed_return(), [0.0; 2]);

        feedback.update([0.5, -0.5], 1.0, 0.5, 0.5);
        let delayed = feedback.delayed_return();
        assert!(delayed[0] > 0.0 && delayed[0] < FEEDBACK_SATURATION_SCALE);
        assert_eq!(delayed[0], -delayed[1]);
    }

    #[test]
    fn zero_and_zero_bound_are_exact_and_reset_is_deterministic() {
        let mut feedback = FeedbackLoop::new();
        feedback.update([1.0, 1.0], 1.0, 1.0, 1.0);
        feedback.update([1.0, 1.0], 0.0, 1.0, 1.0);
        assert_eq!(feedback.delayed_return(), [0.0; 2]);

        feedback.update([1.0, 1.0], 1.0, 0.0, 1.0);
        assert_eq!(feedback.delayed_return(), [0.0; 2]);
        feedback.update([1.0, -1.0], 1.0, 1.0, 1.0);
        feedback.reset();
        assert_eq!(feedback.delayed_return(), [0.0; 2]);
        assert_eq!(feedback.current_q(), 0.0);
    }

    #[test]
    fn hostile_finite_values_saturate_without_an_overflowing_f32_product() {
        let mut feedback = FeedbackLoop::new();
        feedback.update(
            [f64::from(f32::MAX), -f64::from(f32::MAX)],
            f32::MAX,
            f32::MIN_POSITIVE,
            f32::MAX,
        );
        assert_eq!(
            feedback.delayed_return(),
            [FEEDBACK_SATURATION_SCALE, -FEEDBACK_SATURATION_SCALE]
        );
    }
}
