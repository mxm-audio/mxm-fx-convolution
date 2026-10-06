//! Host/input routing before the response engines.
//!
//! A two-channel WAV never chooses its own meaning. [`ResponseInterpretation`] is explicit durable
//! state, and every supported host-layout/response row follows the matrix in
//! `plans/plan-mxm-fx-convolution.md` §1.

/// One host input frame. The effect always produces stereo output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputFrame {
    Mono(f32),
    Stereo([f32; 2]),
}

/// The declared meaning of the canonical response channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseInterpretation {
    /// One response. A stereo host applies it independently to left and right.
    Mono,
    /// Left and right are the outputs of one source position.
    MonoToStereo,
    /// Left and right are independent diagonal input/output paths.
    DiagonalStereo,
}

impl ResponseInterpretation {
    pub const fn source_channels(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::MonoToStereo | Self::DiagonalStereo => 2,
        }
    }

    /// Validate the declaration against decoded canonical audio.
    ///
    /// There is intentionally no width-only constructor: two channels are ambiguous and four-path
    /// matrices are outside v1 until an ordering convention is approved.
    pub const fn validate_channel_count(
        self,
        actual_channels: usize,
    ) -> Result<(), ResponseLayoutError> {
        let expected = self.source_channels();
        if actual_channels == expected {
            Ok(())
        } else {
            Err(ResponseLayoutError {
                expected,
                actual: actual_channels,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseLayoutError {
    pub expected: usize,
    pub actual: usize,
}

/// The two samples which excite the left and right prepared response engines.
///
/// For a mono response those engines share coefficients. For either declared two-channel response,
/// they use the corresponding left/right coefficient stream.
pub type StereoFrame = [f32; 2];

/// Route input into the two wet response paths.
///
/// Non-finite host input is recovered to zero at this public numeric seam. For stereo input and a
/// mono-to-stereo response, `(L + R) / sqrt(2)` is the approved equal-power fold for uncorrelated
/// material. Correlated material may gain and anti-correlated material may cancel; that behavior is
/// not hidden by content-aware processing.
#[inline]
pub fn wet_excitation(input: InputFrame, response: ResponseInterpretation) -> StereoFrame {
    match (finite_input(input), response) {
        (InputFrame::Mono(x), _) => [x, x],
        (InputFrame::Stereo([left, right]), ResponseInterpretation::Mono) => [left, right],
        (InputFrame::Stereo([left, right]), ResponseInterpretation::MonoToStereo) => {
            let folded = (left + right) * core::f32::consts::FRAC_1_SQRT_2;
            [finite_or_zero(folded), finite_or_zero(folded)]
        }
        (InputFrame::Stereo([left, right]), ResponseInterpretation::DiagonalStereo) => {
            [left, right]
        }
    }
}

/// Route the dry leg. Mono is duplicated and stereo remains channel-for-channel.
#[inline]
pub fn dry_output(input: InputFrame) -> StereoFrame {
    match finite_input(input) {
        InputFrame::Mono(x) => [x, x],
        InputFrame::Stereo(frame) => frame,
    }
}

/// Apply the declared Feedback return matrix `R` to the complementary raw response output.
///
/// `f64` keeps the mono-to-stereo sum finite for every finite `f32` raw sample. The return-only
/// saturator consumes this wider value before narrowing it again.
#[inline]
pub(crate) fn feedback_return(wet: StereoFrame, response: ResponseInterpretation) -> [f64; 2] {
    match response {
        ResponseInterpretation::Mono | ResponseInterpretation::DiagonalStereo => {
            [f64::from(wet[0]), f64::from(wet[1])]
        }
        ResponseInterpretation::MonoToStereo => {
            let folded = (f64::from(wet[0]) + f64::from(wet[1])) * core::f64::consts::FRAC_1_SQRT_2;
            [folded, folded]
        }
    }
}

#[inline]
fn finite_input(input: InputFrame) -> InputFrame {
    match input {
        InputFrame::Mono(x) => InputFrame::Mono(finite_or_zero(x)),
        InputFrame::Stereo([left, right]) => {
            InputFrame::Stereo([finite_or_zero(left), finite_or_zero(right)])
        }
    }
}

#[inline]
fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_approved_host_response_row_has_the_declared_excitation() {
        let mono = InputFrame::Mono(0.25);
        for response in [
            ResponseInterpretation::Mono,
            ResponseInterpretation::MonoToStereo,
            ResponseInterpretation::DiagonalStereo,
        ] {
            assert_eq!(wet_excitation(mono, response), [0.25, 0.25]);
        }

        let stereo = InputFrame::Stereo([0.25, -0.5]);
        assert_eq!(
            wet_excitation(stereo, ResponseInterpretation::Mono),
            [0.25, -0.5]
        );
        assert_eq!(
            wet_excitation(stereo, ResponseInterpretation::DiagonalStereo),
            [0.25, -0.5]
        );
        let folded = -0.25 * core::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(
            wet_excitation(stereo, ResponseInterpretation::MonoToStereo),
            [folded, folded]
        );
    }

    #[test]
    fn mono_to_stereo_fold_exposes_correlation_behavior() {
        let correlated = wet_excitation(
            InputFrame::Stereo([1.0, 1.0]),
            ResponseInterpretation::MonoToStereo,
        );
        assert!((correlated[0] - core::f32::consts::SQRT_2).abs() < 1.0e-6);

        let cancelled = wet_excitation(
            InputFrame::Stereo([1.0, -1.0]),
            ResponseInterpretation::MonoToStereo,
        );
        assert_eq!(cancelled, [0.0, 0.0]);
    }

    #[test]
    fn feedback_return_routing_matches_the_declared_composed_operator() {
        assert_eq!(
            feedback_return([0.25, -0.5], ResponseInterpretation::Mono),
            [0.25, -0.5]
        );
        assert_eq!(
            feedback_return([0.25, -0.5], ResponseInterpretation::DiagonalStereo),
            [0.25, -0.5]
        );
        let folded = -0.25f64 * core::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(
            feedback_return([0.25, -0.5], ResponseInterpretation::MonoToStereo),
            [folded, folded]
        );
    }

    #[test]
    fn channel_interpretation_is_explicit_and_width_checked() {
        assert_eq!(
            ResponseInterpretation::Mono.validate_channel_count(1),
            Ok(())
        );
        assert_eq!(
            ResponseInterpretation::MonoToStereo.validate_channel_count(2),
            Ok(())
        );
        assert_eq!(
            ResponseInterpretation::DiagonalStereo.validate_channel_count(4),
            Err(ResponseLayoutError {
                expected: 2,
                actual: 4
            })
        );
    }

    #[test]
    fn public_input_seam_recovers_non_finite_values_without_touching_finite_channels() {
        assert_eq!(dry_output(InputFrame::Mono(f32::NAN)), [0.0, 0.0]);
        assert_eq!(
            wet_excitation(
                InputFrame::Stereo([f32::INFINITY, 0.5]),
                ResponseInterpretation::DiagonalStereo,
            ),
            [0.0, 0.5]
        );
    }
}
