//! Zero-delay partitioned finite impulse response engine.
//!
//! Gardner, *Efficient Convolution without Input-Output Delay* (JAES 43(3), 1995), establishes the
//! architecture: a direct head emits the response onset immediately, while delayed response
//! partitions are transformed and scheduled before their samples are due. This implementation uses
//! a 64-sample early tier and a distributed 2,048-sample late tier. FFT padding is twice each
//! partition width, derived from the `B + P - 1` linear-convolution length.

use crate::fft::{Complex, flush, transform};
use crate::routing::ResponseInterpretation;

/// Sixty-four current samples are evaluated directly and also form one tail scheduling tick. It is
/// public so the plugin and measurement harness can share the proven processing quantum.
pub const PARTITION_SAMPLES: usize = 64;
const FFT_SAMPLES: usize = PARTITION_SAMPLES * 2;
/// The late response begins two 2,048-sample blocks after the input. One block is algorithmic
/// lookahead used to distribute its frequency-domain accumulation over 32 early-tier ticks.
pub const LATE_OFFSET_SAMPLES: usize = 4_096;
pub const LATE_PARTITION_SAMPLES: usize = 2_048;
const LATE_FFT_SAMPLES: usize = LATE_PARTITION_SAMPLES * 2;
const LATE_SCHEDULE_TICKS: usize = LATE_PARTITION_SAMPLES / PARTITION_SAMPLES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareError {
    EmptyResponse,
    ChannelCount {
        expected: usize,
        actual: usize,
    },
    UnequalChannelLengths,
    NonFiniteCoefficient {
        channel: usize,
        sample: usize,
    },
    /// The checked absolute-sum bound cannot be represented by the runtime coefficient type.
    ResponseBoundOverflow,
    /// A checked heap reservation failed. Preparation rejects the response instead of aborting the
    /// host; the previously active engine remains untouched.
    Allocation,
}

type Spectra<const FFT: usize> = Vec<[Complex; FFT]>;

#[derive(Debug)]
struct PreparedPath {
    head: [f32; PARTITION_SAMPLES],
    head_len: usize,
    early_spectra: Vec<[Complex; FFT_SAMPLES]>,
    early_history: Vec<[Complex; FFT_SAMPLES]>,
    late_spectra: Vec<[Complex; LATE_FFT_SAMPLES]>,
    /// Runtime spectrum rings and fixed workspaces are reserved beside immutable spectra so engine
    /// construction is allocation-free and cannot fail after a candidate has been accepted.
    late_history: Vec<[Complex; LATE_FFT_SAMPLES]>,
    late_input: Vec<f32>,
    late_job: Vec<Complex>,
    late_current_output: Vec<f32>,
    late_next_output: Vec<f32>,
    late_overlap: Vec<f32>,
}

/// Prepared response measurements used by the runtime. The wet and Feedback peak bounds are
/// conservative induced bounds. The Feedback edge is the first positive-feedback Nyquist crossing
/// of the complete small-signal loop, including damping and the explicit sample delay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponseBounds {
    /// Maximum absolute row sum from response excitation to raw wet output.
    pub wet: f32,
    /// Peak magnitude of the damped composed Feedback operator. This is the small-gain bound used
    /// only to certify contraction and truthful finite activity.
    pub feedback_peak: f32,
    /// Positive-real Nyquist crossing which puts a displayed Feedback of 1.0 at the linear
    /// self-oscillation edge. This is the runtime gain divisor.
    pub feedback_edge: f32,
}

/// Immutable, completely prepared response data. Construction performs all allocation and FFT work.
#[derive(Debug)]
pub struct PreparedResponse {
    interpretation: ResponseInterpretation,
    response_samples: usize,
    bounds: ResponseBounds,
    paths: [PreparedPath; 2],
}

impl PreparedResponse {
    /// Prepare one mono or two explicitly interpreted response channels.
    pub fn from_channels(
        interpretation: ResponseInterpretation,
        channels: &[&[f32]],
        sample_rate: f32,
    ) -> Result<Self, PrepareError> {
        let expected = interpretation.source_channels();
        if channels.len() != expected {
            return Err(PrepareError::ChannelCount {
                expected,
                actual: channels.len(),
            });
        }
        let response_samples = channels.first().map_or(0, |channel| channel.len());
        if response_samples == 0 {
            return Err(PrepareError::EmptyResponse);
        }
        if channels
            .iter()
            .any(|channel| channel.len() != response_samples)
        {
            return Err(PrepareError::UnequalChannelLengths);
        }
        for (channel_index, channel) in channels.iter().enumerate() {
            if let Some((sample, _)) = channel
                .iter()
                .enumerate()
                .find(|(_, coefficient)| !coefficient.is_finite())
            {
                return Err(PrepareError::NonFiniteCoefficient {
                    channel: channel_index,
                    sample,
                });
            }
        }

        let left_bound = absolute_sum_bound(channels[0])?;
        let right_bound = match interpretation {
            ResponseInterpretation::Mono => left_bound,
            ResponseInterpretation::MonoToStereo | ResponseInterpretation::DiagonalStereo => {
                absolute_sum_bound(channels[1])?
            }
        };
        let wet_bound = left_bound.max(right_bound);
        // This full-response transform is prepared once per distinct source channel, beside the
        // partition transforms and never on the audio thread. Both channel spectra stay live long
        // enough to compose the mono-to-stereo return with its actual phase; summing separate peak
        // magnitudes would recreate the inaudible L1-style overestimate this measurement replaces.
        let (feedback_peak, feedback_edge) =
            feedback_bounds(interpretation, channels, sample_rate)?;
        let left = prepare_path(channels[0])?;
        let right = match interpretation {
            ResponseInterpretation::Mono => prepare_path(channels[0])?,
            ResponseInterpretation::MonoToStereo | ResponseInterpretation::DiagonalStereo => {
                prepare_path(channels[1])?
            }
        };
        Ok(Self {
            interpretation,
            response_samples,
            bounds: ResponseBounds {
                wet: wet_bound,
                feedback_peak,
                feedback_edge,
            },
            paths: [left, right],
        })
    }

    pub const fn interpretation(&self) -> ResponseInterpretation {
        self.interpretation
    }

    pub const fn response_samples(&self) -> usize {
        self.response_samples
    }

    pub const fn bounds(&self) -> ResponseBounds {
        self.bounds
    }

    pub fn into_convolver(self) -> StereoConvolver {
        let response_samples = self.response_samples;
        StereoConvolver {
            interpretation: self.interpretation,
            response_samples,
            bounds: self.bounds,
            paths: self
                .paths
                .map(|path| PathConvolver::new(path, response_samples)),
        }
    }
}

fn feedback_bounds(
    interpretation: ResponseInterpretation,
    channels: &[&[f32]],
    sample_rate: f32,
) -> Result<(f32, f32), PrepareError> {
    // Oversample the finite response's DTFT by at least four. The oscillation edge is a phase
    // crossing, not merely a magnitude maximum, so two bins per response tap can place almost a
    // whole half-cycle between adjacent samples of a late reflection. Four is the smallest grid
    // used here; short responses still receive enough bins to resolve the fixed 180 Hz/8 kHz loop
    // damping. This allocation and transform happen only during response preparation.
    const MIN_BINS: usize = 4_096;
    let fft_samples = channels[0]
        .len()
        .checked_mul(4)
        .and_then(usize::checked_next_power_of_two)
        .map(|padded| padded.max(MIN_BINS))
        .ok_or(PrepareError::Allocation)?;
    let mut spectra = Vec::new();
    spectra
        .try_reserve_exact(channels.len())
        .map_err(|_| PrepareError::Allocation)?;
    for channel in channels {
        let mut spectrum = Vec::new();
        spectrum
            .try_reserve_exact(fft_samples)
            .map_err(|_| PrepareError::Allocation)?;
        spectrum.resize(fft_samples, Complex::ZERO);
        for (bin, coefficient) in spectrum.iter_mut().zip(channel.iter().copied()) {
            bin.re = flush(coefficient);
        }
        transform(&mut spectrum, false);
        spectra.push(spectrum);
    }

    // Two different measurements are required. `peak` is the H-infinity/small-gain bound: dividing
    // by it proves contraction, but it does not locate instability because magnitude discards phase.
    // `edge` is the largest positive-real crossing of the complete open loop
    // `z^-1 * H_damping(z) * R * H(z)`. Positive scalar feedback first loses stability at the
    // reciprocal of that crossing. The previous implementation confused these quantities, then
    // fitted a global 1.2 correction which could not make the edge response-independent.
    let low_pass_a = 1.0
        - (-core::f64::consts::TAU * f64::from(crate::feedback::LOOP_HIGH_CUT_HZ)
            / f64::from(sample_rate))
        .exp();
    let high_pass_pole = (-core::f64::consts::TAU * f64::from(crate::feedback::LOOP_LOW_CUT_HZ)
        / f64::from(sample_rate))
    .exp();
    let multiply = |left: (f64, f64), right: (f64, f64)| {
        (
            left.0 * right.0 - left.1 * right.1,
            left.0 * right.1 + left.1 * right.0,
        )
    };
    let divide = |numerator: (f64, f64), denominator: (f64, f64)| {
        let square = denominator.0 * denominator.0 + denominator.1 * denominator.1;
        (
            (numerator.0 * denominator.0 + numerator.1 * denominator.1) / square,
            (numerator.1 * denominator.0 - numerator.0 * denominator.1) / square,
        )
    };

    let mode_count = if interpretation == ResponseInterpretation::DiagonalStereo {
        2
    } else {
        1
    };
    let mut peak = 0.0f64;
    let mut edge = 0.0f64;
    for mode in 0..mode_count {
        let mut previous = (0.0f64, 0.0f64);
        for (index, _) in spectra[0].iter().enumerate().take(fft_samples / 2 + 1) {
            let response = match interpretation {
                ResponseInterpretation::Mono => spectra[0][index],
                ResponseInterpretation::DiagonalStereo => spectra[mode][index],
                ResponseInterpretation::MonoToStereo => Complex {
                    re: flush(
                        (spectra[0][index].re + spectra[1][index].re)
                            * core::f32::consts::FRAC_1_SQRT_2,
                    ),
                    im: flush(
                        (spectra[0][index].im + spectra[1][index].im)
                            * core::f32::consts::FRAC_1_SQRT_2,
                    ),
                },
            };
            let omega = core::f64::consts::TAU * index as f64 / fft_samples as f64;
            let (sin, cos) = omega.sin_cos();
            let z_inverse = (cos, -sin);
            let low_pass = divide(
                (low_pass_a, 0.0),
                (1.0 - (1.0 - low_pass_a) * cos, (1.0 - low_pass_a) * sin),
            );
            let high_pass = divide(
                (high_pass_pole * (1.0 - cos), high_pass_pole * sin),
                (1.0 - high_pass_pole * cos, high_pass_pole * sin),
            );
            let damping = multiply(low_pass, high_pass);
            let damped = multiply((f64::from(response.re), f64::from(response.im)), damping);
            peak = peak.max(damped.0.hypot(damped.1));

            // The explicit previous-sample return is part of the loop phase. Omitting this factor
            // moves every phase crossing while leaving the magnitude unchanged, which is exactly
            // why a magnitude-only implementation can look plausible and still put the edge wrong.
            let open_loop = multiply(damped, z_inverse);
            if index != 0 {
                if previous.1 == 0.0 {
                    edge = edge.max(previous.0);
                }
                if (previous.1 < 0.0 && open_loop.1 > 0.0)
                    || (previous.1 > 0.0 && open_loop.1 < 0.0)
                {
                    let fraction = previous.1 / (previous.1 - open_loop.1);
                    let crossing_real = previous.0 + fraction * (open_loop.0 - previous.0);
                    edge = edge.max(crossing_real);
                }
            }
            previous = open_loop;
        }
        // A real response is real at Nyquist. Floating trigonometry can leave a tiny imaginary
        // residue there, so include that endpoint explicitly rather than asking a sign test to see
        // through roundoff.
        edge = edge.max(previous.0);
    }

    let peak = upward_f32(peak)?;
    // Some pathological signed responses have no positive-real crossing on the sampled grid. They
    // are allowed not to self-oscillate; the H-infinity fallback still gives useful bounded
    // regeneration instead of silently disabling Feedback with a zero divisor.
    let edge = upward_f32(if edge > 0.0 { edge } else { f64::from(peak) })?;
    Ok((peak, edge))
}

fn absolute_sum_bound(coefficients: &[f32]) -> Result<f32, PrepareError> {
    let mut sum = 0.0f64;
    for coefficient in coefficients {
        let magnitude = f64::from(flush(*coefficient).abs());
        if magnitude != 0.0 {
            sum += magnitude;
            // Directed rounding after every addition makes this a true upper bound even when a
            // much smaller binary32 coefficient falls below the binary64 accumulator's ULP.
            sum = f64::from_bits(sum.to_bits() + 1);
        }
    }
    upward_f32(sum)
}

fn upward_f32(value: f64) -> Result<f32, PrepareError> {
    let mut rounded = value as f32;
    if !rounded.is_finite() {
        return Err(PrepareError::ResponseBoundOverflow);
    }
    if f64::from(rounded) < value {
        rounded = f32::from_bits(rounded.to_bits() + 1);
        if !rounded.is_finite() {
            return Err(PrepareError::ResponseBoundOverflow);
        }
    }
    Ok(rounded)
}

fn prepare_path(coefficients: &[f32]) -> Result<PreparedPath, PrepareError> {
    let mut head = [0.0; PARTITION_SAMPLES];
    let head_len = coefficients.len().min(PARTITION_SAMPLES);
    for (target, source) in head.iter_mut().zip(coefficients).take(head_len) {
        *target = flush(*source);
    }

    let early_end = coefficients.len().min(LATE_OFFSET_SAMPLES);
    let (early_spectra, early_history) =
        prepare_tier::<PARTITION_SAMPLES, FFT_SAMPLES>(&coefficients[head_len..early_end])?;
    let (late_spectra, late_history) =
        prepare_tier::<LATE_PARTITION_SAMPLES, LATE_FFT_SAMPLES>(&coefficients[early_end..])?;
    let late_input = reserved_zeros(LATE_PARTITION_SAMPLES, 0.0f32)?;
    let late_job = reserved_zeros(LATE_FFT_SAMPLES, Complex::ZERO)?;
    let late_current_output = reserved_zeros(LATE_PARTITION_SAMPLES, 0.0f32)?;
    let late_next_output = reserved_zeros(LATE_PARTITION_SAMPLES, 0.0f32)?;
    let late_overlap = reserved_zeros(LATE_PARTITION_SAMPLES, 0.0f32)?;
    Ok(PreparedPath {
        head,
        head_len,
        early_spectra,
        early_history,
        late_spectra,
        late_history,
        late_input,
        late_job,
        late_current_output,
        late_next_output,
        late_overlap,
    })
}

fn reserved_zeros<T: Copy>(length: usize, zero: T) -> Result<Vec<T>, PrepareError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| PrepareError::Allocation)?;
    values.resize(length, zero);
    Ok(values)
}

fn prepare_tier<const PARTITION: usize, const FFT: usize>(
    coefficients: &[f32],
) -> Result<(Spectra<FFT>, Spectra<FFT>), PrepareError> {
    let partition_count = coefficients.len().div_ceil(PARTITION);
    let mut spectra = Vec::new();
    spectra
        .try_reserve_exact(partition_count)
        .map_err(|_| PrepareError::Allocation)?;
    for partition in 0..partition_count {
        let mut spectrum = [Complex::ZERO; FFT];
        let start = partition * PARTITION;
        let end = (start + PARTITION).min(coefficients.len());
        for (target, source) in spectrum.iter_mut().zip(&coefficients[start..end]) {
            target.re = flush(*source);
        }
        transform(&mut spectrum, false);
        spectra.push(spectrum);
    }
    let mut history = Vec::new();
    history
        .try_reserve_exact(partition_count.max(1))
        .map_err(|_| PrepareError::Allocation)?;
    history.resize(partition_count.max(1), [Complex::ZERO; FFT]);
    Ok((spectra, history))
}

struct LateConvolver {
    spectra: Vec<[Complex; LATE_FFT_SAMPLES]>,
    history: Vec<[Complex; LATE_FFT_SAMPLES]>,
    input: Vec<f32>,
    position: usize,
    spectrum_write: usize,
    completed_blocks: usize,
    job: Vec<Complex>,
    job_write: usize,
    job_available: usize,
    job_partition: usize,
    job_ticks: usize,
    current_output: Vec<f32>,
    next_output: Vec<f32>,
    next_ready: bool,
    overlap: Vec<f32>,
}

impl LateConvolver {
    #[allow(clippy::too_many_arguments)]
    fn new(
        spectra: Vec<[Complex; LATE_FFT_SAMPLES]>,
        history: Vec<[Complex; LATE_FFT_SAMPLES]>,
        input: Vec<f32>,
        job: Vec<Complex>,
        current_output: Vec<f32>,
        next_output: Vec<f32>,
        overlap: Vec<f32>,
    ) -> Self {
        Self {
            spectra,
            history,
            input,
            position: 0,
            spectrum_write: 0,
            completed_blocks: 0,
            job,
            job_write: 0,
            job_available: 0,
            job_partition: 0,
            job_ticks: 0,
            current_output,
            next_output,
            next_ready: false,
            overlap,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.current_output[self.position];
        self.input[self.position] = input;
        self.position += 1;
        let early_tick = self.position.is_multiple_of(PARTITION_SAMPLES);
        if self.position == LATE_PARTITION_SAMPLES {
            if self.next_ready {
                self.next_ready = false;
                core::mem::swap(&mut self.current_output, &mut self.next_output);
                self.next_output.fill(0.0);
            } else {
                self.current_output.fill(0.0);
            }
            self.start_job();
            self.input.fill(0.0);
            self.position = 0;
        }
        if early_tick {
            self.advance_job();
        }
        output
    }

    fn start_job(&mut self) {
        if self.spectra.is_empty() {
            return;
        }
        debug_assert_eq!(self.job_ticks, 0, "late convolution missed its deadline");
        self.job.fill(Complex::ZERO);
        for (value, input) in self.job.iter_mut().zip(self.input.iter().copied()) {
            value.re = input;
        }
        transform(&mut self.job, false);
        self.history[self.spectrum_write].copy_from_slice(&self.job);
        self.job.fill(Complex::ZERO);
        self.job_write = self.spectrum_write;
        self.job_available = (self.completed_blocks + 1).min(self.spectra.len());
        self.job_partition = 0;
        self.job_ticks = LATE_SCHEDULE_TICKS;
        self.completed_blocks = self.completed_blocks.saturating_add(1);
        self.spectrum_write = (self.spectrum_write + 1) % self.history.len();
    }

    fn advance_job(&mut self) {
        if self.job_ticks == 0 {
            return;
        }
        let remaining = self.job_available - self.job_partition;
        let count = remaining.div_ceil(self.job_ticks);
        let end = self.job_partition + count;
        for partition in self.job_partition..end {
            let history_index =
                (self.job_write + self.history.len() - partition) % self.history.len();
            // Inputs and responses are real, so the upper FFT bins are the conjugates of the lower
            // bins. Accumulate the independent half only, then mirror once before inversion.
            for bin in 0..=LATE_FFT_SAMPLES / 2 {
                let input = self.history[history_index][bin];
                let response = self.spectra[partition][bin];
                self.job[bin].re += input.re * response.re - input.im * response.im;
                self.job[bin].im += input.re * response.im + input.im * response.re;
            }
        }
        // Keep the distributed accumulator raw between ticks: sanitizing every product or every
        // partial sum makes the 96 kHz ten-second corner miss its deadline. Prepared/history spectra
        // are finite; the complete accumulator is flushed once before the inverse transform.
        self.job_partition = end;
        self.job_ticks -= 1;
        if self.job_ticks == 0 {
            debug_assert_eq!(self.job_partition, self.job_available);
            for bin in 0..=LATE_FFT_SAMPLES / 2 {
                self.job[bin].re = flush(self.job[bin].re);
                self.job[bin].im = flush(self.job[bin].im);
            }
            for bin in 1..LATE_FFT_SAMPLES / 2 {
                self.job[LATE_FFT_SAMPLES - bin] = Complex {
                    re: self.job[bin].re,
                    im: -self.job[bin].im,
                };
            }
            transform(&mut self.job, true);
            for sample in 0..LATE_PARTITION_SAMPLES {
                self.next_output[sample] = flush(self.job[sample].re + self.overlap[sample]);
                self.overlap[sample] = flush(self.job[sample + LATE_PARTITION_SAMPLES].re);
            }
            self.next_ready = true;
        }
    }

    fn reset(&mut self) {
        self.input.fill(0.0);
        self.position = 0;
        self.spectrum_write = 0;
        self.completed_blocks = 0;
        self.job.fill(Complex::ZERO);
        self.job_write = 0;
        self.job_available = 0;
        self.job_partition = 0;
        self.job_ticks = 0;
        self.current_output.fill(0.0);
        self.next_output.fill(0.0);
        self.next_ready = false;
        self.overlap.fill(0.0);
    }
}

struct PathConvolver {
    prepared: PreparedPath,
    direct_history: [f32; PARTITION_SAMPLES],
    direct_write: usize,
    input_block: [f32; PARTITION_SAMPLES],
    block_position: usize,
    spectra: Vec<[Complex; FFT_SAMPLES]>,
    late: LateConvolver,
    spectrum_write: usize,
    completed_blocks: usize,
    tail_output: [f32; PARTITION_SAMPLES],
    overlap: [f32; PARTITION_SAMPLES],
    scratch: [Complex; FFT_SAMPLES],
    response_samples: usize,
    remaining_after_current: usize,
}

impl PathConvolver {
    fn new(mut prepared: PreparedPath, response_samples: usize) -> Self {
        let spectra = core::mem::take(&mut prepared.early_history);
        let late = LateConvolver::new(
            core::mem::take(&mut prepared.late_spectra),
            core::mem::take(&mut prepared.late_history),
            core::mem::take(&mut prepared.late_input),
            core::mem::take(&mut prepared.late_job),
            core::mem::take(&mut prepared.late_current_output),
            core::mem::take(&mut prepared.late_next_output),
            core::mem::take(&mut prepared.late_overlap),
        );
        Self {
            prepared,
            direct_history: [0.0; PARTITION_SAMPLES],
            direct_write: 0,
            input_block: [0.0; PARTITION_SAMPLES],
            block_position: 0,
            spectra,
            late,
            spectrum_write: 0,
            completed_blocks: 0,
            tail_output: [0.0; PARTITION_SAMPLES],
            overlap: [0.0; PARTITION_SAMPLES],
            scratch: [Complex::ZERO; FFT_SAMPLES],
            response_samples,
            remaining_after_current: 0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let input = flush(input);
        if input != 0.0 {
            self.remaining_after_current = self.response_samples.saturating_sub(1);
        }
        self.direct_history[self.direct_write] = input;
        let mut direct = 0.0;
        for tap in 0..self.prepared.head_len {
            let index = (self.direct_write + PARTITION_SAMPLES - tap) % PARTITION_SAMPLES;
            direct = flush(direct + self.prepared.head[tap] * self.direct_history[index]);
        }
        self.direct_write = (self.direct_write + 1) % PARTITION_SAMPLES;

        let tail = flush(self.tail_output[self.block_position] + self.late.process(input));
        self.input_block[self.block_position] = input;
        self.block_position += 1;
        if self.block_position == PARTITION_SAMPLES {
            self.complete_input_block();
            self.block_position = 0;
        }
        let output = if input != 0.0 || self.remaining_after_current != 0 {
            flush(direct + tail)
        } else {
            0.0
        };
        if input == 0.0 {
            self.remaining_after_current = self.remaining_after_current.saturating_sub(1);
            if self.remaining_after_current == 0 {
                // FFT roundoff outside the finite response support is not an audible tail. Clear
                // it exactly so idle is exact and no denormal state survives between events.
                self.clear_history();
            }
        }
        output
    }

    fn complete_input_block(&mut self) {
        if self.prepared.early_spectra.is_empty() {
            self.tail_output.fill(0.0);
            self.overlap.fill(0.0);
            self.input_block.fill(0.0);
            return;
        }

        self.scratch.fill(Complex::ZERO);
        for (value, input) in self.scratch.iter_mut().zip(self.input_block) {
            value.re = input;
        }
        transform(&mut self.scratch, false);
        self.spectra[self.spectrum_write] = self.scratch;

        self.scratch.fill(Complex::ZERO);
        let available = (self.completed_blocks + 1).min(self.prepared.early_spectra.len());
        for partition in 0..available {
            let history_index =
                (self.spectrum_write + self.spectra.len() - partition) % self.spectra.len();
            for bin in 0..=FFT_SAMPLES / 2 {
                let input = self.spectra[history_index][bin];
                let response = self.prepared.early_spectra[partition][bin];
                self.scratch[bin].re += input.re * response.re - input.im * response.im;
                self.scratch[bin].im += input.re * response.im + input.im * response.re;
            }
        }
        for bin in 0..=FFT_SAMPLES / 2 {
            self.scratch[bin].re = flush(self.scratch[bin].re);
            self.scratch[bin].im = flush(self.scratch[bin].im);
        }
        for bin in 1..FFT_SAMPLES / 2 {
            self.scratch[FFT_SAMPLES - bin] = Complex {
                re: self.scratch[bin].re,
                im: -self.scratch[bin].im,
            };
        }
        transform(&mut self.scratch, true);
        for sample in 0..PARTITION_SAMPLES {
            self.tail_output[sample] = flush(self.scratch[sample].re + self.overlap[sample]);
            self.overlap[sample] = flush(self.scratch[sample + PARTITION_SAMPLES].re);
        }

        self.input_block.fill(0.0);
        self.completed_blocks = self.completed_blocks.saturating_add(1);
        self.spectrum_write = (self.spectrum_write + 1) % self.spectra.len();
    }

    fn clear_history(&mut self) {
        self.direct_history.fill(0.0);
        self.direct_write = 0;
        self.input_block.fill(0.0);
        self.block_position = 0;
        // Do not clear the response-sized spectrum ring here. Resetting `completed_blocks` means
        // `complete_input_block` can read only slots written in this generation: block one reads
        // slot 0, block two reads slots 1 and 0, and so on. Old spectra are therefore invalid by
        // construction without an O(response length) callback spike.
        self.spectrum_write = 0;
        self.completed_blocks = 0;
        self.tail_output.fill(0.0);
        self.overlap.fill(0.0);
        self.scratch.fill(Complex::ZERO);
        self.late.reset();
    }

    fn reset(&mut self) {
        self.clear_history();
        self.remaining_after_current = 0;
    }
}

/// Two response paths sharing one declared channel interpretation.
pub struct StereoConvolver {
    interpretation: ResponseInterpretation,
    response_samples: usize,
    bounds: ResponseBounds,
    paths: [PathConvolver; 2],
}

impl StereoConvolver {
    pub const fn interpretation(&self) -> ResponseInterpretation {
        self.interpretation
    }

    pub const fn response_samples(&self) -> usize {
        self.response_samples
    }

    pub const fn bounds(&self) -> ResponseBounds {
        self.bounds
    }

    pub fn is_active(&self) -> bool {
        self.paths
            .iter()
            .any(|path| path.remaining_after_current != 0)
    }

    #[inline]
    pub fn process_excitation(&mut self, excitation: [f32; 2]) -> [f32; 2] {
        [
            self.paths[0].process(excitation[0]),
            self.paths[1].process(excitation[1]),
        ]
    }

    pub fn reset(&mut self) {
        self.paths[0].reset();
        self.paths[1].reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(input: &[f32], response: &[f32]) -> Vec<f32> {
        let mut output = vec![0.0; input.len() + response.len() - 1];
        for (input_index, input_sample) in input.iter().enumerate() {
            for (tap, coefficient) in response.iter().enumerate() {
                output[input_index + tap] += input_sample * coefficient;
            }
        }
        output
    }

    #[test]
    fn checked_response_bounds_include_the_declared_return_matrix() {
        let mono = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[&[1.0, -2.0]],
            48_000.0,
        )
        .unwrap();
        let mono_bounds = mono.bounds();
        let one_ulp_above_three = f32::from_bits(3.0f32.to_bits() + 1);
        assert!((3.0..=one_ulp_above_three).contains(&mono_bounds.wet));
        // The Feedback divisor is the peak of the return spectrum *after* the loop damping, measured
        // on a linear (zero-padded) grid. That is the loop's actual round-trip gain, so `q = 1` is
        // unity gain on any response rather than a coordinate that drifts with response shape.
        //
        // For [1, -2] the damped magnitudes peak at 1.4244, and that value is invariant across every
        // grid from 2 to 65,536 bins - unlike the one-tap case, whose peak only appears once the
        // grid resolves the damping. Dividing by the undamped peak (3.0 here) was inaudible at every
        // setting, because that peak sits far above the body of a real response and often in bins
        // the damping removes. Dividing by the RMS (sqrt 5 here) was audible but left the edge
        // wherever each impulse's shape put it - across the owner's 200-impulse catalogue it
        // scattered from a displayed 0.51 to 2.32, with 36% never oscillating at all.
        let mono_damped_peak = 1.424_437_8f32;
        assert!((mono_bounds.feedback_peak - mono_damped_peak).abs() < 2.0e-6);
        // The positive-real crossing is lower than the magnitude peak because phase decides when
        // positive feedback actually loses stability. Keeping these as separate assertions prevents
        // a later cleanup from recreating the two-day magnitude/threshold confusion.
        assert!((mono_bounds.feedback_edge - 1.284_58).abs() < 2.0e-4);
        assert!(mono_bounds.feedback_edge < mono_bounds.feedback_peak);
        assert!(mono_bounds.feedback_peak < mono_bounds.wet);

        let left = [1.0, -2.0];
        let right = [-0.5, 0.0];
        let stereo = PreparedResponse::from_channels(
            ResponseInterpretation::MonoToStereo,
            &[&left, &right],
            48_000.0,
        )
        .unwrap();
        assert!((3.0..=one_ulp_above_three).contains(&stereo.bounds().wet));
        // Composed with the channel phase retained, then damped and taken at its peak on the same
        // grid. Summing per-channel magnitudes instead would recreate the L1-style overestimate this
        // measurement exists to replace.
        // Read from the engine, not modelled: composing `(L + R) / sqrt(2)` in `f64` and comparing
        // predicts 1.033_191_6, which misses this by 3.4e-6 - more than the tolerance - because the
        // engine flushes each component in `f32` before taking the magnitude. The engine is the
        // authority for its own bound.
        let stereo_damped_peak = 1.033_188_2f32;
        assert!((stereo.bounds().feedback_peak - stereo_damped_peak).abs() < 2.0e-6);
        assert!((stereo.bounds().feedback_edge - 0.906_25).abs() < 2.0e-4);
        assert!(stereo.bounds().feedback_edge < stereo.bounds().feedback_peak);
        assert!(stereo.bounds().feedback_peak < 3.5 * core::f32::consts::FRAC_1_SQRT_2);
    }

    #[test]
    fn unrepresentable_response_bound_is_rejected_without_truncation() {
        assert_eq!(
            PreparedResponse::from_channels(
                ResponseInterpretation::Mono,
                &[&[f32::MAX, f32::MAX]],
                48_000.0,
            )
            .unwrap_err(),
            PrepareError::ResponseBoundOverflow
        );
    }

    #[test]
    fn impulse_reproduces_response_across_partition_seams_without_latency() {
        let response: Vec<_> = (0..151)
            .map(|index| ((index as f32 + 1.0) * 0.17).sin() * 0.1)
            .collect();
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        let mut actual = Vec::new();
        for sample in core::iter::once(1.0).chain(core::iter::repeat_n(0.0, response.len() - 1)) {
            actual.push(convolver.process_excitation([sample, sample])[0]);
        }
        for (index, (actual, expected)) in actual.iter().zip(&response).enumerate() {
            assert!(
                (actual - expected).abs() < 2.0e-5,
                "sample {index}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn distributed_late_tier_reproduces_coefficients_at_and_after_its_boundary() {
        let response: Vec<_> = (0..LATE_OFFSET_SAMPLES + LATE_PARTITION_SAMPLES + 73)
            .map(|index| ((index as f32 + 0.25) * 0.071).sin() * 0.03)
            .collect();
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        for (index, expected) in response.iter().enumerate() {
            let input = if index == 0 { 1.0 } else { 0.0 };
            let actual = convolver.process_excitation([input, input])[0];
            assert!(
                (actual - expected).abs() < 3.0e-5,
                "sample {index}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn seeded_signal_matches_complete_direct_convolution() {
        let input: Vec<_> = (0..173)
            .map(|index| ((index as f32 + 0.3) * 0.71).cos() * 0.2)
            .collect();
        let response: Vec<_> = (0..LATE_OFFSET_SAMPLES + 137)
            .map(|index| (-0.001 * index as f32).exp() * (index as f32 * 0.31).sin())
            .collect();
        let expected = direct(&input, &response);
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        let mut actual = Vec::with_capacity(expected.len());
        for sample in input
            .iter()
            .copied()
            .chain(core::iter::repeat_n(0.0, response.len() - 1))
        {
            actual.push(convolver.process_excitation([sample, sample])[0]);
        }
        let worst = actual
            .iter()
            .zip(&expected)
            .map(|(actual, expected)| (actual - expected).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 5.0e-4, "worst null residual {worst}");
    }

    #[test]
    fn preparation_rejects_invalid_audio_and_flushes_subnormal_coefficients() {
        assert!(matches!(
            PreparedResponse::from_channels(ResponseInterpretation::Mono, &[&[]], 48_000.0),
            Err(PrepareError::EmptyResponse)
        ));
        assert!(matches!(
            PreparedResponse::from_channels(ResponseInterpretation::Mono, &[&[f32::NAN]], 48_000.0),
            Err(PrepareError::NonFiniteCoefficient { .. })
        ));

        let response = [f32::from_bits(1)];
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        assert_eq!(convolver.process_excitation([1.0, 1.0]), [0.0, 0.0]);
    }

    #[test]
    fn finite_response_reaches_exact_idle_after_its_declared_support() {
        let response: Vec<_> = (0..PARTITION_SAMPLES + 19)
            .map(|index| 0.5f32.powi(index as i32 / 8 + 1))
            .collect();
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        for sample in core::iter::once(1.0).chain(core::iter::repeat_n(0.0, response.len() - 1)) {
            convolver.process_excitation([sample, sample]);
        }
        for _ in 0..PARTITION_SAMPLES * 3 {
            let output = convolver.process_excitation([0.0, 0.0]);
            assert_eq!(output, [0.0, 0.0]);
        }
    }

    #[test]
    fn subnormal_stress_leaves_no_subnormal_history_or_output() {
        let response = vec![f32::from_bits(1); PARTITION_SAMPLES * 2 + 3];
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        for _ in 0..PARTITION_SAMPLES * 4 {
            let output = convolver.process_excitation([f32::from_bits(1); 2]);
            assert!(output.into_iter().all(|value| value == 0.0));
        }
        for path in &convolver.paths {
            assert!(
                path.direct_history
                    .iter()
                    .all(|value| normal_or_zero(*value))
            );
            assert!(path.input_block.iter().all(|value| normal_or_zero(*value)));
            assert!(path.tail_output.iter().all(|value| normal_or_zero(*value)));
            assert!(path.overlap.iter().all(|value| normal_or_zero(*value)));
            assert!(
                path.spectra
                    .iter()
                    .flatten()
                    .all(|value| { normal_or_zero(value.re) && normal_or_zero(value.im) })
            );
            assert!(
                path.scratch
                    .iter()
                    .all(|value| normal_or_zero(value.re) && normal_or_zero(value.im))
            );
        }
    }

    fn normal_or_zero(value: f32) -> bool {
        value == 0.0 || value.is_normal()
    }

    #[test]
    fn reset_invalidates_response_sized_spectra_without_clearing_them() {
        let response = vec![0.5; LATE_OFFSET_SAMPLES + LATE_PARTITION_SAMPLES * 3];
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        for _ in 0..LATE_PARTITION_SAMPLES * 2 {
            convolver.process_excitation([1.0, 1.0]);
        }
        let early_before: Vec<_> = convolver.paths[0]
            .spectra
            .iter()
            .map(|spectrum| spectrum[0].re.to_bits())
            .collect();
        let late_before: Vec<_> = convolver.paths[0]
            .late
            .history
            .iter()
            .map(|spectrum| spectrum[0].re.to_bits())
            .collect();
        assert!(early_before.iter().any(|value| *value != 0));
        assert!(late_before.iter().any(|value| *value != 0));
        convolver.reset();
        let early_after: Vec<_> = convolver.paths[0]
            .spectra
            .iter()
            .map(|spectrum| spectrum[0].re.to_bits())
            .collect();
        let late_after: Vec<_> = convolver.paths[0]
            .late
            .history
            .iter()
            .map(|spectrum| spectrum[0].re.to_bits())
            .collect();
        assert_eq!(early_after, early_before, "reset cleared early spectra");
        assert_eq!(late_after, late_before, "reset cleared late spectra");
        for _ in 0..response.len() + PARTITION_SAMPLES {
            assert_eq!(convolver.process_excitation([0.0, 0.0]), [0.0, 0.0]);
        }
    }

    #[test]
    fn reset_discards_partial_blocks_and_histories() {
        let response = vec![0.5; PARTITION_SAMPLES + 7];
        let prepared = PreparedResponse::from_channels(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut convolver = prepared.into_convolver();
        for _ in 0..PARTITION_SAMPLES + 3 {
            convolver.process_excitation([1.0, 1.0]);
        }
        convolver.reset();
        for _ in 0..response.len() + PARTITION_SAMPLES {
            assert_eq!(convolver.process_excitation([0.0, 0.0]), [0.0, 0.0]);
        }
    }
}
