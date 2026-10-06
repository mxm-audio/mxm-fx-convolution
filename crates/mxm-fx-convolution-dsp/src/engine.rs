//! Complete linear run-time path around the prepared FIR.

use crate::activity::{Activity, ActivityTracker, RecursiveActivity, TailBounds};
use crate::control::{
    ControlBlock, ControlChange, FeedbackControlState, GateAction, LiveControls, MixGate,
    WetPostControlState, WetPostControls, WetPostTailConfig,
};
use crate::convolver::{PrepareError, PreparedResponse, StereoConvolver};
use crate::delay::Predelay;
use crate::feedback::{FEEDBACK_SATURATION_SCALE, FeedbackLoop};
use crate::fft::flush;
use crate::post::{WetPostPath, WetPostPathError};
use crate::routing::{InputFrame, dry_output, feedback_return, wet_excitation};

#[derive(Debug, Clone, Copy)]
struct WetSample {
    raw: [f32; 2],
    delayed: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedbackTransitionStage {
    Idle,
    PreRamp,
    Crossfade,
    PostRamp,
}

struct WetEngine {
    convolver: StereoConvolver,
    predelay: Predelay,
}

impl WetEngine {
    // The failure path returns the owned prepared response without another allocation so a bounded
    // replacement can retain/retry it. Boxing this control-thread error would undermine fallibility.
    #[allow(clippy::result_large_err)]
    fn new(
        response: PreparedResponse,
        max_pre_delay_samples: usize,
        pre_delay_samples: usize,
        control_ramp_samples: u64,
    ) -> Result<Self, (PrepareError, PreparedResponse)> {
        let predelay = match Predelay::new(
            max_pre_delay_samples,
            pre_delay_samples,
            control_ramp_samples,
        ) {
            Ok(predelay) => predelay,
            Err(error) => return Err((error, response)),
        };
        Ok(Self {
            convolver: response.into_convolver(),
            predelay,
        })
    }

    #[inline]
    fn process(&mut self, input: InputFrame, feedback: Option<[f32; 2]>) -> WetSample {
        let external = wet_excitation(input, self.convolver.interpretation());
        // `None` preserves Revision 6's exact operation sequence. The nonzero branch adds the same
        // one-sample-delayed return excitation to both transition engines.
        let excitation = match feedback {
            None => external,
            Some(returned) => [
                flush(external[0] + returned[0]),
                flush(external[1] + returned[1]),
            ],
        };
        let raw = self.convolver.process_excitation(excitation);
        WetSample {
            raw,
            delayed: self.predelay.process(raw),
        }
    }

    fn set_pre_delay_immediate(&mut self, samples: usize) {
        self.predelay.set_delay_immediate(samples);
    }

    fn set_pre_delay(&mut self, samples: usize) -> u64 {
        self.predelay.set_delay(samples)
    }

    fn response_samples(&self) -> usize {
        self.convolver.response_samples()
    }

    fn feedback_edge(&self) -> f32 {
        self.convolver.bounds().feedback_edge
    }

    fn feedback_peak(&self) -> f32 {
        self.convolver.bounds().feedback_peak
    }

    fn interpretation(&self) -> crate::routing::ResponseInterpretation {
        self.convolver.interpretation()
    }

    fn is_active(&self) -> bool {
        self.convolver.is_active()
    }

    fn reset(&mut self) {
        self.convolver.reset();
        self.predelay.reset();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    LengthMismatch,
    EventBlockLengthMismatch,
}

/// Opaque response storage removed from the audio engine for destruction on a non-realtime thread.
pub struct RetiredResponse {
    _engine: WetEngine,
}

/// The zero-delay response engine plus only linear surrounding operations.
///
/// Construction, [`begin_response_transition`](Self::begin_response_transition), and destruction of
/// storage released by [`take_retired`](Self::take_retired) are control-thread operations. Processing
/// performs no allocation, locking, I/O, planning, logging, or destruction of response storage.
pub struct AudioEngine {
    controls: LiveControls,
    mix: MixGate,
    primary: WetEngine,
    old: Option<WetEngine>,
    retired: Option<WetEngine>,
    transition_position: u64,
    transition_samples: u64,
    feedback_transition_stage: FeedbackTransitionStage,
    feedback_transition_position: u64,
    feedback_bridge_bound: f32,
    max_pre_delay_samples: usize,
    control_ramp_samples: u64,
    activity: ActivityTracker,
    clipped: bool,
    numeric_fault: bool,
    feedback: FeedbackControlState,
    feedback_loop: FeedbackLoop,
    feedback_history_active: bool,
    wet_post: WetPostControlState,
    wet_post_tail: WetPostTailConfig,
    wet_post_path: Option<WetPostPath>,
}

impl AudioEngine {
    /// Build a completely allocated engine off the audio thread, rejecting allocation failure.
    pub fn try_new(
        response: PreparedResponse,
        max_pre_delay_samples: usize,
        supplied_controls: LiveControls,
        control_ramp_samples: u64,
    ) -> Result<Self, PrepareError> {
        // Recover malformed restored controls against known-finite defaults instead of trying to
        // sanitize a value in place (which would leave NaN unchanged when the edit is rejected).
        let mut controls = LiveControls::default();
        controls.apply(ControlChange::Mix(supplied_controls.mix));
        controls.pre_delay_samples = supplied_controls
            .pre_delay_samples
            .min(max_pre_delay_samples as u64);
        let primary = WetEngine::new(
            response,
            max_pre_delay_samples,
            controls.pre_delay_samples as usize,
            control_ramp_samples,
        )
        .map_err(|(error, _)| error)?;
        let active = controls.mix != 0.0;
        Ok(Self {
            controls,
            mix: if active {
                MixGate::active(controls.mix, control_ramp_samples)
            } else {
                MixGate::parked(control_ramp_samples)
            },
            primary,
            old: None,
            retired: None,
            transition_position: 0,
            transition_samples: 0,
            feedback_transition_stage: FeedbackTransitionStage::Idle,
            feedback_transition_position: 0,
            feedback_bridge_bound: 0.0,
            max_pre_delay_samples,
            control_ramp_samples: control_ramp_samples.max(1),
            activity: if active {
                ActivityTracker::new_active()
            } else {
                ActivityTracker::new_parked()
            },
            clipped: false,
            numeric_fault: false,
            feedback: FeedbackControlState::new(control_ramp_samples),
            feedback_loop: FeedbackLoop::new(),
            feedback_history_active: false,
            wet_post: WetPostControlState::new(control_ramp_samples),
            wet_post_tail: WetPostTailConfig::default(),
            wet_post_path: None,
        })
    }

    /// Convenience for fixed, program-owned test/demo responses. User-controlled preparation must
    /// call [`Self::try_new`] and preserve the working engine on failure.
    pub fn new(
        response: PreparedResponse,
        max_pre_delay_samples: usize,
        supplied_controls: LiveControls,
        control_ramp_samples: u64,
    ) -> Self {
        Self::try_new(
            response,
            max_pre_delay_samples,
            supplied_controls,
            control_ramp_samples,
        )
        .expect("fixed response engine allocation failed")
    }

    pub const fn controls(&self) -> LiveControls {
        self.controls
    }

    pub const fn wet_post_controls(&self) -> WetPostControls {
        self.wet_post.targets()
    }

    pub const fn feedback(&self) -> f32 {
        self.feedback.target()
    }

    /// Retarget Feedback without carrying an in-flight smoothing ramp into activation or restore.
    pub fn set_feedback_immediate(&mut self, feedback: f32) {
        self.feedback.apply(ControlChange::Feedback(feedback));
        self.feedback.reset();
        self.feedback_loop.reset();
    }

    /// Allocate and enable the single Revision 6 post-crossfade path off the audio thread.
    /// Failure leaves the current path and Revision 5 sound untouched.
    /// Also the one place the engine learns its rate: the plugin calls this unconditionally at
    /// activation, so the loop filter's coefficients are set for every engine that ever processes.
    pub fn try_enable_wet_post(&mut self, sample_rate: f32) -> Result<(), WetPostPathError> {
        self.feedback_loop.set_sample_rate(sample_rate);
        let path = WetPostPath::try_new(sample_rate)?;
        self.wet_post_tail = path.tail_config();
        self.wet_post_path = Some(path);
        Ok(())
    }

    pub const fn wet_post_enabled(&self) -> bool {
        self.wet_post_path.is_some()
    }

    /// Override conservative post-stage bounds for a measured host configuration.
    pub fn set_wet_post_tail_config(&mut self, config: WetPostTailConfig) {
        self.wet_post_tail = config;
    }

    /// Retarget Revision 6 controls without carrying an in-flight smoothing ramp into activation.
    pub fn set_wet_post_controls_immediate(&mut self, controls: WetPostControls) {
        for change in [
            ControlChange::LowCutHz(controls.wet_eq.low_cut_hz),
            ControlChange::HighCutHz(controls.wet_eq.high_cut_hz),
            ControlChange::ToneDb(controls.wet_eq.tone_db),
            ControlChange::Modulation(controls.modulation),
            ControlChange::Width(controls.width),
        ] {
            self.wet_post.apply(change);
        }
        self.wet_post.reset();
    }

    /// Retarget a completely prepared, still-silent engine immediately before publication. This is
    /// a control-thread operation and performs no allocation.
    pub fn set_controls_immediate(&mut self, supplied: LiveControls) {
        let mut controls = LiveControls::default();
        controls.apply(ControlChange::Mix(supplied.mix));
        controls.pre_delay_samples = supplied
            .pre_delay_samples
            .min(self.max_pre_delay_samples as u64);
        self.controls = controls;
        self.mix = if controls.mix == 0.0 {
            MixGate::parked(self.control_ramp_samples)
        } else {
            MixGate::active(controls.mix, self.control_ramp_samples)
        };
        self.primary
            .set_pre_delay_immediate(controls.pre_delay_samples as usize);
        self.activity = if controls.mix == 0.0 {
            ActivityTracker::new_parked()
        } else {
            ActivityTracker::new_active()
        };
    }

    /// Start a complete-output convex response crossfade using an engine whose response spectra,
    /// runtime history and pre-delay storage were all allocated off audio.
    // The control-thread caller must recover the completely prepared candidate when bounded
    // transition storage is occupied; boxing only this rare error would add another allocation.
    #[allow(clippy::result_large_err)]
    pub fn begin_engine_transition(
        &mut self,
        mut candidate: AudioEngine,
        controls: LiveControls,
        transition_samples: u64,
    ) -> Result<(), AudioEngine> {
        if self.old.is_some()
            || self.retired.is_some()
            || self.feedback_transition_stage != FeedbackTransitionStage::Idle
        {
            return Err(candidate);
        }
        candidate.set_controls_immediate(controls);
        debug_assert!(candidate.old.is_none() && candidate.retired.is_none());
        let next = candidate.primary;
        self.apply_control(ControlChange::Mix(controls.mix));
        self.apply_control(ControlChange::PreDelaySamples(controls.pre_delay_samples));
        let previous = core::mem::replace(&mut self.primary, next);
        if self.mix.is_parked() {
            self.retired = Some(previous);
            self.clear_response_transition();
        } else {
            // Crossfading two responses can create a phase crossing neither endpoint has. Bridge
            // with the larger H-infinity peak: conservative for the short transition, while each
            // steady response uses its measured Nyquist edge.
            self.feedback_bridge_bound = previous.feedback_peak().max(self.primary.feedback_peak());
            self.feedback_transition_stage = if self.feedback.is_exact_bypass() {
                FeedbackTransitionStage::Idle
            } else {
                FeedbackTransitionStage::PreRamp
            };
            self.feedback_transition_position = 0;
            self.old = Some(previous);
            self.transition_position = 0;
            self.transition_samples = transition_samples.max(1);
        }
        Ok(())
    }

    /// Start a complete-output convex response crossfade.
    ///
    /// A transition already in flight or response storage waiting for off-audio destruction returns
    /// the candidate unchanged. This is the bounded two-engine supersession policy: publication
    /// waits rather than dropping either audible leg or constructing a third engine.
    // Returning the owned candidate lets the off-audio caller retry after collecting retired
    // storage. Boxing it solely to shrink a rare control-thread error would add an allocation.
    #[allow(clippy::result_large_err)]
    pub fn begin_response_transition(
        &mut self,
        response: PreparedResponse,
        transition_samples: u64,
    ) -> Result<(), PreparedResponse> {
        if self.old.is_some()
            || self.retired.is_some()
            || self.feedback_transition_stage != FeedbackTransitionStage::Idle
        {
            return Err(response);
        }
        let next = match WetEngine::new(
            response,
            self.max_pre_delay_samples,
            self.controls.pre_delay_samples as usize,
            self.control_ramp_samples,
        ) {
            Ok(next) => next,
            Err((_, response)) => return Err(response),
        };
        let previous = core::mem::replace(&mut self.primary, next);
        if self.mix.is_parked() {
            // Off has no audible history to preserve and no callback work with which to advance a
            // fade. Make the new empty response primary immediately and retire the old allocation.
            self.retired = Some(previous);
            self.clear_response_transition();
        } else {
            // Crossfading two responses can create a phase crossing neither endpoint has. Bridge
            // with the larger H-infinity peak: conservative for the short transition, while each
            // steady response uses its measured Nyquist edge.
            self.feedback_bridge_bound = previous.feedback_peak().max(self.primary.feedback_peak());
            self.feedback_transition_stage = if self.feedback.is_exact_bypass() {
                FeedbackTransitionStage::Idle
            } else {
                FeedbackTransitionStage::PreRamp
            };
            self.feedback_transition_position = 0;
            self.old = Some(previous);
            self.transition_position = 0;
            self.transition_samples = transition_samples.max(1);
        }
        Ok(())
    }

    /// Move retired response storage out for destruction on a control/background thread.
    pub fn take_retired_response(&mut self) -> Option<RetiredResponse> {
        self.retired
            .take()
            .map(|engine| RetiredResponse { _engine: engine })
    }

    /// Collect retired storage immediately. Call only from a non-realtime context.
    pub fn take_retired(&mut self) -> bool {
        self.take_retired_response().is_some()
    }

    pub const fn transition_active(&self) -> bool {
        self.old.is_some()
            || matches!(
                self.feedback_transition_stage,
                FeedbackTransitionStage::PostRamp
            )
    }

    pub const fn clipped(&self) -> bool {
        self.clipped
    }

    pub const fn numeric_fault(&self) -> bool {
        self.numeric_fault
    }

    pub fn clear_telemetry(&mut self) {
        self.clipped = false;
        self.numeric_fault = false;
    }

    pub fn apply_control(&mut self, change: ControlChange) {
        let old = self.controls;
        let old_post_tail = self.wet_post.tail_allowance(self.wet_post_tail);
        self.controls.apply(change);
        self.wet_post.apply(change);
        self.feedback.apply(change);
        let new_post_tail = self.wet_post.tail_allowance(self.wet_post_tail);
        if self.activity.remaining_samples() != 0 && new_post_tail > old_post_tail {
            self.activity.extend(
                self.activity
                    .remaining_samples()
                    .saturating_add(new_post_tail - old_post_tail),
            );
        }
        self.controls.pre_delay_samples = self
            .controls
            .pre_delay_samples
            .min(self.max_pre_delay_samples as u64);

        if self.controls.pre_delay_samples != old.pre_delay_samples {
            let delay = self.controls.pre_delay_samples as usize;
            let mut exposed = self.primary.set_pre_delay(delay);
            if let Some(old) = &mut self.old {
                exposed = exposed.max(old.set_pre_delay(delay));
            }
            // A longer tap can expose wet samples after the tail calculated at their arrival has
            // expired. Keep the host awake through every valid ring position and the tap fade.
            self.activity.extend(exposed);
        }
        if self.controls.mix != old.mix {
            match self.mix.set_target(self.controls.mix) {
                GateAction::ResetAndWake => {
                    self.reset_wet_histories();
                    self.wet_post.reset();
                    self.feedback.reset();
                    self.activity.wake_empty();
                }
                GateAction::None | GateAction::Park => {}
            }
        }
    }

    /// Apply sample-offset controls and process one host block without allocation.
    pub fn process_block<const N: usize>(
        &mut self,
        input: &[InputFrame],
        output: &mut [[f32; 2]],
        mut events: ControlBlock<N>,
    ) -> Result<Activity, ProcessError> {
        if input.len() != output.len() {
            return Err(ProcessError::LengthMismatch);
        }
        if events.block_len() != input.len() {
            return Err(ProcessError::EventBlockLengthMismatch);
        }

        let mut next_event = events.next();
        let mut last_nonzero = None;
        for (sample_index, (input, output)) in input.iter().copied().zip(output).enumerate() {
            while matches!(next_event, Some(event) if event.sample_offset == sample_index) {
                let event = next_event.expect("the matching event is present");
                self.apply_control(event.change);
                next_event = events.next();
            }

            let dry = dry_output(input);
            let gate = self.mix.next_sample();
            let wet_running = !self.mix.is_parked() || gate.action == GateAction::Park;
            let wet_post = wet_running.then(|| self.wet_post.next_frame());
            let feedback_frame = wet_running.then(|| self.feedback.next_frame()).flatten();
            let pause_response_crossfade = self.feedback_transition_stage
                == FeedbackTransitionStage::Crossfade
                && self.feedback.is_ramping_down();
            // Do not even read the delay on the exact-zero branch. The final explicit zero frame
            // from a downward ramp clears it; the following sample returns to structural bypass.
            let loop_excitation = feedback_frame.map(|_| self.feedback_loop.delayed_return());
            if feedback_frame.is_none() {
                self.feedback_loop.reset();
                match self.feedback_transition_stage {
                    FeedbackTransitionStage::PreRamp => {
                        self.feedback_transition_stage = FeedbackTransitionStage::Crossfade;
                        self.feedback_transition_position = 0;
                    }
                    FeedbackTransitionStage::PostRamp => {
                        self.feedback_transition_stage = FeedbackTransitionStage::Idle;
                        self.feedback_transition_position = 0;
                    }
                    FeedbackTransitionStage::Idle | FeedbackTransitionStage::Crossfade => {}
                }
            }
            let loop_input_nonzero =
                loop_excitation.is_some_and(|frame| frame[0] != 0.0 || frame[1] != 0.0);
            let response_wet = if wet_running {
                let primary = self.primary.process(input, loop_excitation);
                if self.old.is_some() {
                    let old_engine = self.old.as_mut().expect("checked above");
                    let old_interpretation = old_engine.interpretation();
                    let old_edge = old_engine.feedback_edge();
                    let old_peak = old_engine.feedback_peak();
                    let old = old_engine.process(input, loop_excitation);
                    let alpha = match self.feedback_transition_stage {
                        FeedbackTransitionStage::PreRamp => 0.0,
                        FeedbackTransitionStage::Idle
                        | FeedbackTransitionStage::Crossfade
                        | FeedbackTransitionStage::PostRamp => {
                            transition_alpha(self.transition_position, self.transition_samples)
                        }
                    };
                    if let Some(feedback) = feedback_frame {
                        let old_return = feedback_return(old.raw, old_interpretation);
                        let new_return =
                            feedback_return(primary.raw, self.primary.interpretation());
                        let routed = [
                            old_return[0] * f64::from(1.0 - alpha)
                                + new_return[0] * f64::from(alpha),
                            old_return[1] * f64::from(1.0 - alpha)
                                + new_return[1] * f64::from(alpha),
                        ];
                        let (gain_divisor, contraction_peak) =
                            if self.feedback_transition_stage == FeedbackTransitionStage::PreRamp {
                                (
                                    lerp_bound(
                                        old_edge,
                                        self.feedback_bridge_bound,
                                        transition_alpha(
                                            self.feedback_transition_position,
                                            self.control_ramp_samples,
                                        ),
                                    ),
                                    old_peak,
                                )
                            } else {
                                (self.feedback_bridge_bound, self.feedback_bridge_bound)
                            };
                        self.feedback_loop.update(
                            routed,
                            feedback.q,
                            gain_divisor,
                            contraction_peak,
                        );
                    }
                    let mixed = [
                        flush(old.delayed[0] * (1.0 - alpha) + primary.delayed[0] * alpha),
                        flush(old.delayed[1] * (1.0 - alpha) + primary.delayed[1] * alpha),
                    ];
                    match self.feedback_transition_stage {
                        FeedbackTransitionStage::PreRamp => {
                            self.feedback_transition_position += 1;
                            if self.feedback_transition_position == self.control_ramp_samples {
                                self.feedback_transition_stage = FeedbackTransitionStage::Crossfade;
                                self.feedback_transition_position = 0;
                            }
                        }
                        FeedbackTransitionStage::Idle | FeedbackTransitionStage::Crossfade => {
                            if !pause_response_crossfade {
                                self.transition_position += 1;
                                if self.transition_position == self.transition_samples {
                                    self.retired = self.old.take();
                                    self.transition_position = 0;
                                    self.transition_samples = 0;
                                    if feedback_frame.is_some() {
                                        self.feedback_transition_stage =
                                            FeedbackTransitionStage::PostRamp;
                                        self.feedback_transition_position = 0;
                                    } else {
                                        self.feedback_transition_stage =
                                            FeedbackTransitionStage::Idle;
                                    }
                                }
                            }
                        }
                        FeedbackTransitionStage::PostRamp => {
                            debug_assert!(false, "post-ramp cannot retain an old engine")
                        }
                    }
                    mixed
                } else {
                    if let Some(feedback) = feedback_frame {
                        let routed = feedback_return(primary.raw, self.primary.interpretation());
                        let gain_divisor = if self.feedback_transition_stage
                            == FeedbackTransitionStage::PostRamp
                        {
                            lerp_bound(
                                self.feedback_bridge_bound,
                                self.primary.feedback_edge(),
                                transition_alpha(
                                    self.feedback_transition_position,
                                    self.control_ramp_samples,
                                ),
                            )
                        } else {
                            self.primary.feedback_edge()
                        };
                        self.feedback_loop.update(
                            routed,
                            feedback.q,
                            gain_divisor,
                            self.primary.feedback_peak(),
                        );
                        if self.feedback_transition_stage == FeedbackTransitionStage::PostRamp {
                            self.feedback_transition_position += 1;
                            if self.feedback_transition_position == self.control_ramp_samples {
                                self.feedback_transition_stage = FeedbackTransitionStage::Idle;
                                self.feedback_transition_position = 0;
                            }
                        }
                    }
                    primary.delayed
                }
            } else {
                [0.0; 2]
            };
            let wet = match (&mut self.wet_post_path, wet_post) {
                (Some(path), Some(controls)) => path.process(response_wet, controls),
                _ => response_wet,
            };

            let mix = gate.mix;
            for channel in 0..2 {
                // Defence rather than a path: the input, the convolver and the wet post stages each
                // clean their own output, and a blend of finite values cannot overflow. The one
                // route here was the wet gain, deleted on 2026-09-28 with the test that took it.
                let raw = dry[channel] * (1.0 - mix) + wet[channel] * mix;
                if !raw.is_finite() {
                    self.numeric_fault = true;
                }
                let finite = flush(raw);
                if finite.abs() > 1.0 {
                    self.clipped = true;
                }
                output[channel] = finite;
            }

            if loop_input_nonzero || self.feedback_loop.is_active() {
                self.feedback_history_active = true;
            }
            if wet_running && (dry[0] != 0.0 || dry[1] != 0.0) {
                last_nonzero = Some(sample_index);
            }
            if gate.action == GateAction::Park {
                self.reset_wet_histories();
                if self.retired.is_none() {
                    self.retired = self.old.take();
                }
                self.clear_response_transition();
                self.wet_post.reset();
                self.feedback.park();
                self.activity.park();
            }
        }

        let response_samples = self
            .old
            .as_ref()
            .map_or(self.primary.response_samples(), |old| {
                old.response_samples().max(self.primary.response_samples())
            });
        let transition_remaining = self.response_transition_remaining();
        let post_remaining = self.wet_post.tail_allowance(self.wet_post_tail);
        let q = self.feedback_loop.current_q().max(self.feedback.target());
        // Stability at the Nyquist edge and contraction are different claims. A response can be
        // linearly stable below q=1 while its H-infinity norm still permits transient growth. Only
        // the separate small-gain ratio licenses the geometric finite-tail certificate.
        let mut contraction =
            self.feedback_loop
                .current_contraction_bound()
                .max(feedback_contraction_bound(
                    q,
                    self.primary.feedback_peak(),
                    self.primary.feedback_edge(),
                ));
        if let Some(old) = &self.old {
            contraction = contraction.max(feedback_contraction_bound(
                q,
                old.feedback_peak(),
                old.feedback_edge(),
            ));
        }
        let convolution_active =
            self.primary.is_active() || self.old.as_ref().is_some_and(WetEngine::is_active);
        let recursive = if !self.feedback_history_active {
            // A nonzero control does not itself create recursive state. In particular, an exactly
            // cancelling return remains an ordinary feed-forward FIR tail even for q >= 1.
            RecursiveActivity::Empty
        } else if q == 0.0 {
            if convolution_active {
                RecursiveActivity::CertifiedTail(
                    (response_samples as u64)
                        .saturating_add(self.controls.pre_delay_samples)
                        .saturating_add(transition_remaining)
                        .saturating_add(post_remaining),
                )
            } else {
                self.feedback_history_active = false;
                RecursiveActivity::Empty
            }
        } else if !self.feedback_loop.is_active() && !convolution_active {
            self.feedback_history_active = false;
            RecursiveActivity::Empty
        } else if contraction >= 1.0 || !contraction.is_finite() {
            RecursiveActivity::Uncertified
        } else {
            RecursiveActivity::CertifiedTail(
                certified_feedback_tail(contraction, response_samples as u64)
                    // The loop damping decays on its own memory after the geometric term has
                    // expired, so its settling belongs in the declared horizon.
                    .saturating_add(self.feedback_loop.settling_samples())
                    .saturating_add(self.feedback.ramp_remaining())
                    .saturating_add(self.controls.pre_delay_samples)
                    .saturating_add(transition_remaining)
                    .saturating_add(post_remaining),
            )
        };
        Ok(self.activity.finish_block_with_recursive(
            input.len(),
            last_nonzero,
            TailBounds {
                response_samples: response_samples as u64,
                pre_delay_samples: self.controls.pre_delay_samples,
                transition_samples: transition_remaining.saturating_add(post_remaining),
            },
            recursive,
        ))
    }

    /// Empty every live history. Controls and prepared responses remain selected.
    pub fn reset(&mut self) {
        self.reset_wet_histories();
        self.mix = if self.controls.mix == 0.0 {
            MixGate::parked(self.control_ramp_samples)
        } else {
            MixGate::active(self.controls.mix, self.control_ramp_samples)
        };
        self.activity = if self.controls.mix == 0.0 {
            ActivityTracker::new_parked()
        } else {
            ActivityTracker::new_active()
        };
        self.clipped = false;
        self.numeric_fault = false;
        self.wet_post.reset();
        self.feedback.reset();
    }

    fn reset_wet_histories(&mut self) {
        self.primary.reset();
        if let Some(old) = &mut self.old {
            old.reset();
        }
        if let Some(path) = &mut self.wet_post_path {
            path.reset_history();
        }
        self.feedback_loop.reset();
        self.feedback_history_active = false;
    }

    fn clear_response_transition(&mut self) {
        self.transition_position = 0;
        self.transition_samples = 0;
        self.feedback_transition_stage = FeedbackTransitionStage::Idle;
        self.feedback_transition_position = 0;
        self.feedback_bridge_bound = self.primary.feedback_peak();
    }

    fn response_transition_remaining(&self) -> u64 {
        let crossfade = self
            .transition_samples
            .saturating_sub(self.transition_position);
        match self.feedback_transition_stage {
            FeedbackTransitionStage::Idle => crossfade,
            FeedbackTransitionStage::PreRamp => self
                .control_ramp_samples
                .saturating_sub(self.feedback_transition_position)
                .saturating_add(crossfade)
                .saturating_add(self.control_ramp_samples),
            FeedbackTransitionStage::Crossfade => crossfade
                .saturating_add(self.control_ramp_samples)
                .saturating_add(if self.feedback.is_ramping_down() {
                    self.feedback.ramp_remaining()
                } else {
                    0
                }),
            FeedbackTransitionStage::PostRamp => self
                .control_ramp_samples
                .saturating_sub(self.feedback_transition_position),
        }
    }
}

#[inline]
fn feedback_contraction_bound(q: f32, peak: f32, edge: f32) -> f32 {
    if q == 0.0 || peak == 0.0 {
        0.0
    } else if edge > 0.0 {
        (f64::from(q) * f64::from(peak) / f64::from(edge)) as f32
    } else {
        f32::INFINITY
    }
}

#[inline]
fn certified_feedback_tail(contraction: f32, response_samples: u64) -> u64 {
    debug_assert!(contraction > 0.0 && contraction < 1.0);
    let q = contraction;
    // The return curve is 1-Lipschitz and bounded by A. Solve the plan's geometric remainder
    // A*q^(K+1)/(1-q) <= epsilon at the exact-idle normal/subnormal seam, then allow one complete
    // response plus explicit delay per remaining pass.
    let epsilon = f64::from(f32::MIN_POSITIVE);
    let q = f64::from(q);
    let ratio = epsilon * (1.0 - q) / f64::from(FEEDBACK_SATURATION_SCALE);
    let passes = if ratio >= 1.0 {
        0.0
    } else {
        (ratio.ln() / q.ln()).ceil().max(0.0)
    };
    let passes = if passes >= u64::MAX as f64 {
        u64::MAX
    } else {
        passes as u64
    };
    passes.saturating_mul(response_samples.saturating_add(1))
}

#[inline]
fn lerp_bound(from: f32, to: f32, alpha: f32) -> f32 {
    let exact = f64::from(from) * f64::from(1.0 - alpha) + f64::from(to) * f64::from(alpha);
    let mut rounded = exact as f32;
    if f64::from(rounded) < exact {
        rounded = f32::from_bits(rounded.to_bits() + 1);
    }
    rounded
}

#[inline]
fn transition_alpha(position: u64, samples: u64) -> f32 {
    if samples <= 1 {
        1.0
    } else {
        position as f32 / (samples - 1) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::TimedControl;
    use crate::routing::ResponseInterpretation;

    fn mono_response(samples: &[f32]) -> PreparedResponse {
        PreparedResponse::from_channels(ResponseInterpretation::Mono, &[samples], 48_000.0).unwrap()
    }

    #[test]
    fn linear_mix_has_no_hidden_output_safety_processor() {
        let mut engine = AudioEngine::new(
            mono_response(&[4.0]),
            0,
            LiveControls {
                mix: 1.0,
                pre_delay_samples: 0,
            },
            4,
        );
        let mut output = [[0.0; 2]; 1];
        engine
            .process_block(
                &[InputFrame::Stereo([1.0, -1.0])],
                &mut output,
                ControlBlock::<0>::new(1),
            )
            .unwrap();
        assert_eq!(output[0], [4.0, -4.0]);
        assert!(engine.clipped());
    }

    #[test]
    fn sample_offset_mix_event_uses_phase_one_ordering() {
        let mut engine = AudioEngine::new(
            mono_response(&[2.0]),
            0,
            LiveControls {
                mix: 0.0,
                ..LiveControls::default()
            },
            1,
        );
        let mut events = ControlBlock::<1>::new(3);
        events
            .push(TimedControl {
                sample_offset: 1,
                change: ControlChange::Mix(1.0),
            })
            .unwrap();
        let mut output = [[0.0; 2]; 3];
        engine
            .process_block(&[InputFrame::Mono(1.0); 3], &mut output, events)
            .unwrap();
        assert_eq!(output, [[1.0, 1.0], [2.0, 2.0], [2.0, 2.0]]);
        assert_eq!(engine.controls().mix, 1.0);
    }

    #[test]
    fn sample_offset_feedback_event_updates_only_the_control_side() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            4,
        );
        let mut events = ControlBlock::<1>::new(1);
        events
            .push(TimedControl {
                sample_offset: 0,
                change: ControlChange::Feedback(1.5),
            })
            .unwrap();
        let mut output = [[0.0; 2]; 1];
        engine
            .process_block(&[InputFrame::Mono(1.0)], &mut output, events)
            .unwrap();

        assert_eq!(engine.feedback(), 1.5);
        assert_eq!(output, [[1.0, 1.0]]);
    }

    #[test]
    fn feedback_reconvolves_raw_response_with_one_explicit_sample_of_delay() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            3,
            LiveControls {
                mix: 1.0,
                pre_delay_samples: 3,
            },
            1,
        );
        engine.set_feedback_immediate(0.5);
        let mut input = [InputFrame::Mono(0.0); 8];
        input[0] = InputFrame::Mono(1.0);
        let mut output = [[0.0; 2]; 8];
        engine
            .process_block(&input, &mut output, ControlBlock::<0>::new(8))
            .unwrap();

        assert_eq!(&output[..3], &[[0.0; 2]; 3]);
        assert_eq!(output[3], [1.0, 1.0]);
        // The loop coefficient is the control divided by this response's measured Nyquist edge.
        // The return is damped before it is saturated, so the expectation runs the same one-pole
        // pair the loop does rather than widening the tolerance until the assertion stops noticing
        // it.
        let bound = f64::from(
            crate::convolver::PreparedResponse::from_channels(
                ResponseInterpretation::Mono,
                &[&[1.0]],
                48_000.0,
            )
            .unwrap()
            .bounds()
            .feedback_edge,
        );
        let coefficient = 0.5f64 / bound;
        // The loop saturator is `A * tanh(v / A)`, so the expectation has to carry the ceiling as
        // well as the filter. Modelling only `tanh(v)` encoded `A == 1.0` invisibly and broke the
        // first time the ceiling was retuned; naming the constant keeps this honest at any value.
        let scale = f64::from(crate::feedback::FEEDBACK_SATURATION_SCALE);
        let rate = 48_000.0f32;
        let lp_a =
            1.0 - (-core::f32::consts::TAU * crate::feedback::loop_high_cut_hz() / rate).exp();
        let hp_a = (-core::f32::consts::TAU * crate::feedback::loop_low_cut_hz() / rate).exp();
        let (mut lp, mut hp, mut prior) = (0.0f32, 0.0f32, 0.0f32);
        let mut previous = output[3][0];
        for (index, frame) in output[4..].iter().copied().enumerate() {
            lp += lp_a * (previous - lp);
            hp = hp_a * (hp + lp - prior);
            prior = lp;
            let expected = (scale * (coefficient / scale * f64::from(hp)).tanh()) as f32;
            for actual in frame {
                assert!(
                    actual.to_bits().abs_diff(expected.to_bits()) <= 4,
                    "wet sample {}: {actual} != {expected}",
                    index + 1
                );
            }
            previous = frame[0];
        }
    }

    #[test]
    fn feedback_uses_the_declared_return_matrix_before_reconvolution() {
        fn second_frame(interpretation: ResponseInterpretation) -> [f32; 2] {
            let left = [1.0];
            let right = [-1.0];
            let response = PreparedResponse::from_channels(
                interpretation,
                &[left.as_slice(), right.as_slice()],
                48_000.0,
            )
            .unwrap();
            let mut engine = AudioEngine::new(
                response,
                0,
                LiveControls {
                    mix: 1.0,
                    ..LiveControls::default()
                },
                1,
            );
            engine.set_feedback_immediate(0.5);
            let mut output = [[0.0; 2]; 2];
            engine
                .process_block(
                    &[InputFrame::Mono(1.0), InputFrame::Mono(0.0)],
                    &mut output,
                    ControlBlock::<0>::new(2),
                )
                .unwrap();
            output[1]
        }

        assert_eq!(
            second_frame(ResponseInterpretation::MonoToStereo),
            [0.0, 0.0]
        );
        let diagonal = second_frame(ResponseInterpretation::DiagonalStereo);
        assert!(diagonal[0] > 0.0);
        assert_eq!(diagonal[0], diagonal[1]);
    }

    #[test]
    fn an_exactly_cancelling_return_never_claims_uncertified_recursion() {
        let left = [1.0, 0.5, 0.25, 0.125];
        let right = left.map(|sample| -sample);
        let response = PreparedResponse::from_channels(
            ResponseInterpretation::MonoToStereo,
            &[&left, &right],
            48_000.0,
        )
        .unwrap();
        let mut engine = AudioEngine::new(
            response,
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            1,
        );
        engine.set_feedback_immediate(1.25);
        let mut output = [[0.0; 2]; 1];
        let activity = engine
            .process_block(
                &[InputFrame::Mono(1.0)],
                &mut output,
                ControlBlock::<0>::new(1),
            )
            .unwrap();
        assert_ne!(activity, Activity::KeepAlive);
        assert_eq!(engine.feedback_loop.delayed_return(), [0.0; 2]);
        assert!(!engine.feedback_history_active);
    }

    #[test]
    fn downward_feedback_automation_pauses_the_response_crossfade() {
        let controls = LiveControls {
            mix: 1.0,
            ..LiveControls::default()
        };
        let mut engine = AudioEngine::new(mono_response(&[1.0]), 0, controls, 4);
        engine.set_feedback_immediate(1.0);
        let candidate = AudioEngine::new(mono_response(&[2.0]), 0, controls, 4);
        assert!(
            engine
                .begin_engine_transition(candidate, controls, 8)
                .is_ok()
        );

        let mut output = [[0.0; 2]; 4];
        engine
            .process_block(
                &[InputFrame::Mono(0.0); 4],
                &mut output,
                ControlBlock::<0>::new(4),
            )
            .unwrap();
        assert_eq!(
            engine.feedback_transition_stage,
            FeedbackTransitionStage::Crossfade
        );
        assert_eq!(engine.transition_position, 0);

        engine.apply_control(ControlChange::Feedback(0.5));
        for expected_position in [0, 0, 0, 1] {
            let mut output = [[0.0; 2]; 1];
            engine
                .process_block(
                    &[InputFrame::Mono(0.0)],
                    &mut output,
                    ControlBlock::<0>::new(1),
                )
                .unwrap();
            assert_eq!(engine.transition_position, expected_position);
        }
    }

    #[test]
    fn measured_nyquist_edge_puts_one_tap_decay_and_growth_around_one() {
        fn render(q: f32) -> (f64, f64, Activity) {
            let mut engine = AudioEngine::new(
                mono_response(&[1.0]),
                0,
                LiveControls {
                    mix: 1.0,
                    ..LiveControls::default()
                },
                1,
            );
            engine.set_feedback_immediate(q);
            let mut early = 0.0f64;
            let mut late = 0.0f64;
            let mut activity = Activity::Normal;
            for index in 0..100_000 {
                let input = InputFrame::Mono(if index == 0 { 0.1 } else { 0.0 });
                let mut output = [[0.0; 2]; 1];
                activity = engine
                    .process_block(&[input], &mut output, ControlBlock::<0>::new(1))
                    .unwrap();
                let square = f64::from(output[0][0]).powi(2);
                if (1_000..2_000).contains(&index) {
                    early += square;
                }
                if index >= 90_000 {
                    late += square;
                }
            }
            (early, late, activity)
        }

        let (below_early, below_late, below_activity) = render(0.98);
        assert!(
            below_early > 0.0 && below_late < below_early * 0.1,
            "{below_early:e} -> {below_late:e}"
        );
        assert_eq!(below_activity, Activity::Normal);

        let (above_early, above_late, above_activity) = render(1.02);
        assert!(
            above_late > above_early * 0.9,
            "{above_early:e} -> {above_late:e}"
        );
        assert_eq!(above_activity, Activity::KeepAlive);
    }

    #[test]
    fn dense_response_feedback_decays_below_one_and_sustains_above_one() {
        // A room-scale response, not the original 21 ms one. With damping in the loop a 1,024-sample
        // impulse recirculates itself into silence long before this test's measurement windows at
        // 80,000 and 150,000 samples, so both read zero and the assertion says `0 -> 0` - measuring
        // an empty room rather than a decaying one. The defect this guards is response-shape
        // dependent, so the fixture has to be the shape an owner actually loads.
        fn response() -> Vec<f32> {
            let mut state = 0x1234_5678u32;
            (0..100_000)
                .map(|index| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                    noise * (-(index as f32) / 30_000.0).exp()
                })
                .collect()
        }

        fn render(q: f32) -> (f64, f64) {
            let response = response();
            let mut engine = AudioEngine::new(
                mono_response(&response),
                0,
                LiveControls {
                    mix: 1.0,
                    ..LiveControls::default()
                },
                1,
            );
            engine.set_feedback_immediate(q);
            let mut early_energy = 0.0f64;
            let mut late_energy = 0.0f64;
            for index in 0..160_000 {
                let input = InputFrame::Mono(if index == 0 { 0.5 } else { 0.0 });
                let mut output = [[0.0; 2]; 1];
                engine
                    .process_block(&[input], &mut output, ControlBlock::<0>::new(1))
                    .unwrap();
                let square = f64::from(output[0][0]).powi(2);
                if (80_000..90_000).contains(&index) {
                    early_energy += square;
                }
                if index >= 150_000 {
                    late_energy += square;
                }
            }
            (early_energy / 10_000.0, late_energy / 10_000.0)
        }

        // Probed at 0.6, not 0.95. The edge measures at a displayed 1.00-1.05, so 0.95 sits about
        // five percent below it and fades gently rather than collapsing - measured at 1.342e-3 ->
        // 3.324e-4, a fall to 24.8 percent, which is a real decay but not the tenfold one this
        // assertion demands. Moving the probe clear of the edge keeps the margin strict; relaxing
        // the margin instead would weaken the very thing this proof exists to catch.
        let (decaying_early, decaying_late) = render(0.6);
        assert!(
            decaying_late < decaying_early * 0.1,
            "{decaying_early} -> {decaying_late}"
        );
        let (sustaining_early, sustaining_late) = render(1.25);
        assert!(sustaining_late > 1.0e-6, "late energy {sustaining_late}");
        assert!(
            sustaining_late > sustaining_early * 0.5,
            "{sustaining_early} -> {sustaining_late}"
        );
    }

    #[test]
    fn real_length_response_feedback_changes_the_tail() {
        fn response(len: usize, decay_samples: f32) -> Vec<f32> {
            let mut state = 0x1234_5678u32;
            (0..len)
                .map(|index| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                    noise * (-(index as f32) / decay_samples).exp()
                })
                .collect()
        }

        fn render(samples: &[f32], q: f32) -> (f64, f64) {
            let mut engine = AudioEngine::new(
                mono_response(samples),
                0,
                LiveControls {
                    mix: 1.0,
                    ..LiveControls::default()
                },
                1,
            );
            engine.set_feedback_immediate(q);
            let mut early = 0.0f64;
            let mut late = 0.0f64;
            for index in 0..200_000 {
                let input = InputFrame::Mono(if index == 0 { 0.5 } else { 0.0 });
                let mut output = [[0.0; 2]; 1];
                engine
                    .process_block(&[input], &mut output, ControlBlock::<0>::new(1))
                    .unwrap();
                let square = f64::from(output[0][0]).powi(2);
                if (100_000..110_000).contains(&index) {
                    early += square;
                }
                if index >= 190_000 {
                    late += square;
                }
            }
            (early / 10_000.0, late / 10_000.0)
        }

        // A room-scale impulse: 100k samples at 48 kHz, decaying over roughly a second, which is
        // the shape and length the owner actually loads. The 1,024-sample proof above cannot
        // detect a defect whose size scales with response length.
        let long = response(100_000, 30_000.0);
        let (early_zero, late_zero) = render(&long, 0.0);
        let (early_hot, late_hot) = render(&long, 1.25);
        eprintln!("q=0.00 early={early_zero:e} late={late_zero:e}");
        eprintln!("q=1.25 early={early_hot:e} late={late_hot:e}");
        assert!(
            late_hot > late_zero * 10.0,
            "feedback at the maximum must audibly change a room-length tail: \
             q=0 late {late_zero:e} vs q=1.25 late {late_hot:e}"
        );
    }

    /// Where does the knob actually cross from decaying to sustaining?
    ///
    /// Normalising by the RMS rather than the peak buys audibility and gives up "q = 1 is the
    /// threshold". This prints the crossing so the usable range is a measured fact rather than an
    /// assumption, and so `MAX_FEEDBACK` can be chosen against it.
    #[test]
    #[ignore = "tuning instrument, not a regression test: asserts nothing, run explicitly"]
    fn feedback_threshold_sweep_on_a_room_length_response() {
        fn response(len: usize, decay_samples: f32) -> Vec<f32> {
            let mut state = 0x1234_5678u32;
            (0..len)
                .map(|index| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                    noise * (-(index as f32) / decay_samples).exp()
                })
                .collect()
        }

        fn ratio(samples: &[f32], q: f32) -> f64 {
            let mut engine = AudioEngine::new(
                mono_response(samples),
                0,
                LiveControls {
                    mix: 1.0,
                    ..LiveControls::default()
                },
                1,
            );
            engine.set_feedback_immediate(q);
            let mut early = 0.0f64;
            let mut late = 0.0f64;
            for index in 0..200_000 {
                let input = InputFrame::Mono(if index == 0 { 0.5 } else { 0.0 });
                let mut output = [[0.0; 2]; 1];
                engine
                    .process_block(&[input], &mut output, ControlBlock::<0>::new(1))
                    .unwrap();
                let square = f64::from(output[0][0]).powi(2);
                if (100_000..110_000).contains(&index) {
                    early += square;
                }
                if index >= 190_000 {
                    late += square;
                }
            }
            if early > 0.0 {
                late / early
            } else {
                f64::INFINITY
            }
        }

        // Three response shapes, because the owner wants a displayed 1.0 to be the threshold on
        // every impulse. Peak normalisation gave that by construction; RMS does not, so the spread
        // across shapes is the error a single internal scale factor cannot remove.
        let shapes: [(&str, Vec<f32>); 3] = [
            ("short 7ms", response(336, 100.0)),
            ("medium 1s", response(48_000, 12_000.0)),
            ("long 2s", response(100_000, 30_000.0)),
        ];
        for (label, samples) in &shapes {
            let mut crossing = f32::NAN;
            for step in 4..=30 {
                let q = step as f32 * 0.05;
                let r = ratio(samples, q);
                if r > 1.0 && crossing.is_nan() {
                    crossing = q;
                }
                eprintln!(
                    "{label}: q={q:.3}  late/early={r:.4}  {}",
                    if r > 1.0 { "SUSTAINS" } else { "decays" }
                );
            }
            eprintln!("{label}: THRESHOLD at q={crossing:.3}");
        }
    }

    /// Reports how loud the oscillation sits against the input, for the current saturation scale.
    ///
    /// This is a measurement, not an assertion: `FEEDBACK_SATURATION_SCALE` is a `const`, so the
    /// sweep that chooses it recompiles this crate per value. The scale in force is printed so a run
    /// that did not actually recompile is visible in its own output rather than being mistaken for a
    /// tuning result.
    #[test]
    #[ignore = "tuning instrument, not a regression test: recompile per scale and run explicitly"]
    fn oscillation_ceiling_against_the_input_for_the_current_scale() {
        fn response(len: usize, decay_samples: f32) -> Vec<f32> {
            let mut state = 0x1234_5678u32;
            (0..len)
                .map(|index| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                    noise * (-(index as f32) / decay_samples).exp()
                })
                .collect()
        }

        // The player fixture's excitation, so these figures land on the scale already reported.
        fn drive(index: usize) -> f32 {
            (index as f32 * 0.01).sin() * 0.25
        }

        fn measure(samples: &[f32], q: f32, mix: f32) -> (f32, f64) {
            let mut engine = AudioEngine::new(
                mono_response(samples),
                0,
                LiveControls {
                    mix,
                    ..LiveControls::default()
                },
                1,
            );
            engine.set_feedback_immediate(q);
            let mut peak = 0.0f32;
            let mut energy = 0.0f64;
            for index in 0..400_000 {
                let input = InputFrame::Mono(drive(index));
                let mut output = [[0.0; 2]; 1];
                engine
                    .process_block(&[input], &mut output, ControlBlock::<0>::new(1))
                    .unwrap();
                // Late window only: the ceiling is the steady state, not the initial build-up.
                if index >= 300_000 {
                    peak = peak.max(output[0][0].abs());
                    energy += f64::from(output[0][0]).powi(2);
                }
            }
            (peak, energy)
        }

        let room = response(100_000, 30_000.0);
        let input_peak = (0..400_000).map(drive).fold(0.0f32, |a, b| a.max(b.abs()));

        eprintln!("SCALE_IN_BINARY={FEEDBACK_SATURATION_SCALE}");
        eprintln!("input peak={input_peak:.6}");
        let (dry_peak, dry_energy) = measure(&room, 0.0, 0.0);
        eprintln!("mix 0.00 q=0.00 (dry only): peak={dry_peak:.6} energy={dry_energy:.6e}");
        for q in [0.0f32, 1.0, 1.25] {
            let (peak, energy) = measure(&room, q, 0.38);
            eprintln!(
                "mix 0.38 q={q:.2}: peak={peak:.6} energy={energy:.6e} \
                 ratio_to_input={:.2}",
                peak / input_peak
            );
        }
    }

    #[test]
    fn feedback_through_the_ramped_control_path_matches_the_immediate_path() {
        fn response(len: usize, decay_samples: f32) -> Vec<f32> {
            let mut state = 0x1234_5678u32;
            (0..len)
                .map(|index| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                    noise * (-(index as f32) / decay_samples).exp()
                })
                .collect()
        }

        // `ramped` drives the seam the live plugin actually uses: `process_slices` calls
        // `apply_control(ControlChange::Feedback(..))` every block, never `set_feedback_immediate`.
        // Every existing proof takes the immediate path, so this seam has never been measured.
        fn render(samples: &[f32], q: f32, ramped: bool) -> f64 {
            let mut engine = AudioEngine::new(
                mono_response(samples),
                0,
                LiveControls {
                    mix: 1.0,
                    ..LiveControls::default()
                },
                1,
            );
            if ramped {
                engine.apply_control(ControlChange::Feedback(q));
            } else {
                engine.set_feedback_immediate(q);
            }
            let mut late = 0.0f64;
            for index in 0..200_000 {
                let input = InputFrame::Mono(if index == 0 { 0.5 } else { 0.0 });
                let mut output = [[0.0; 2]; 1];
                engine
                    .process_block(&[input], &mut output, ControlBlock::<0>::new(1))
                    .unwrap();
                if index >= 190_000 {
                    late += f64::from(output[0][0]).powi(2);
                }
            }
            late / 10_000.0
        }

        let long = response(100_000, 30_000.0);
        let immediate = render(&long, 1.25, false);
        let ramped = render(&long, 1.25, true);
        eprintln!("immediate late={immediate:e}  ramped late={ramped:e}");
        assert!(
            ramped > immediate * 0.5,
            "the ramped control path must reach the same sustaining loop as the immediate path: \
             immediate {immediate:e} vs ramped {ramped:e}"
        );
    }

    #[test]
    fn response_relative_feedback_has_the_same_loop_excitation_for_scaled_responses() {
        fn render(coefficient: f32) -> [[f32; 2]; 3] {
            let mut engine = AudioEngine::new(
                mono_response(&[coefficient]),
                0,
                LiveControls {
                    mix: 1.0,
                    ..LiveControls::default()
                },
                1,
            );
            engine.set_feedback_immediate(0.75);
            let mut output = [[0.0; 2]; 3];
            engine
                .process_block(
                    &[
                        InputFrame::Mono(1.0),
                        InputFrame::Mono(0.0),
                        InputFrame::Mono(0.0),
                    ],
                    &mut output,
                    ControlBlock::<0>::new(3),
                )
                .unwrap();
            output
        }

        let unit = render(1.0);
        let doubled = render(2.0);
        for sample in 0..3 {
            assert!((doubled[sample][0] - 2.0 * unit[sample][0]).abs() < 1.0e-6);
        }
    }

    #[test]
    fn above_certificate_feedback_sustains_but_reset_returns_exact_silence() {
        // A room-scale response rather than a one-tap. On `H = [1]` the RMS divisor equals the peak,
        // so reaching unity would need a displayed 2.38 - past the product's 1.25 maximum - and the
        // fixture could only sustain by asserting something unreachable. The control's edge is
        // calibrated for the response shape an owner loads, so the proof uses that shape.
        let response: Vec<f32> = {
            let mut state = 0x1234_5678u32;
            (0..100_000)
                .map(|index| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                    noise * (-(index as f32) / 30_000.0).exp()
                })
                .collect()
        };
        let mut engine = AudioEngine::new(
            mono_response(&response),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            1,
        );
        engine.set_feedback_immediate(1.25);
        // Sample 511 is 1% into a 100,000-sample response: the convolver has barely begun and the
        // loop has not established, so the original `output[511] > 0.5` asserted something that
        // cannot be true of a room-scale impulse. Sustain is what the name claims, so sustain is
        // what is measured - late energy against early, as the sibling proofs do.
        let mut activity = Activity::Normal;
        let mut early = 0.0f64;
        let mut late = 0.0f64;
        for index in 0..160_000 {
            let input = InputFrame::Mono(if index == 0 { 1.0 } else { 0.0 });
            let mut frame = [[0.0; 2]; 1];
            activity = engine
                .process_block(&[input], &mut frame, ControlBlock::<0>::new(1))
                .unwrap();
            let square = f64::from(frame[0][0]).powi(2);
            if (80_000..90_000).contains(&index) {
                early += square;
            }
            if index >= 150_000 {
                late += square;
            }
        }
        assert_eq!(activity, Activity::KeepAlive);
        assert!(
            late > early * 0.5,
            "above the edge the loop must sustain: {early:e} -> {late:e}"
        );

        engine.reset();
        let mut silence = [[1.0; 2]; 64];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0); 64],
                    &mut silence,
                    ControlBlock::<0>::new(64),
                )
                .unwrap(),
            Activity::Normal
        );
        assert_eq!(silence, [[0.0; 2]; 64]);
    }

    #[test]
    fn feedback_response_replacement_runs_pre_crossfade_and_post_phases() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            2,
        );
        engine.set_feedback_immediate(0.75);
        engine
            .begin_response_transition(mono_response(&[2.0]), 3)
            .unwrap();

        let mut output = [[0.0; 2]; 1];
        for sample in 0..7 {
            engine
                .process_block(
                    &[InputFrame::Mono(if sample == 0 { 1.0 } else { 0.0 })],
                    &mut output,
                    ControlBlock::<0>::new(1),
                )
                .unwrap();
            assert!(output[0].into_iter().all(f32::is_finite));
            assert_eq!(engine.transition_active(), sample < 6, "sample {sample}");
            if sample == 4 {
                assert!(engine.take_retired());
                assert!(
                    engine
                        .begin_response_transition(mono_response(&[0.5]), 1)
                        .is_err(),
                    "post-ramp must remain part of the bounded publication transition"
                );
            }
        }
    }

    #[test]
    fn wet_post_events_extend_activity_only_by_configured_stateful_bounds() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            10,
        );
        engine.set_wet_post_tail_config(WetPostTailConfig {
            wet_eq_settle_samples: 100,
            modulation_history_samples: 40,
        });
        let mut events = ControlBlock::<2>::new(1);
        events
            .push(TimedControl {
                sample_offset: 0,
                change: ControlChange::LowCutHz(90.0),
            })
            .unwrap();
        events
            .push(TimedControl {
                sample_offset: 0,
                change: ControlChange::Width(1.5),
            })
            .unwrap();
        let mut output = [[0.0; 2]; 1];
        assert_eq!(
            engine
                .process_block(&[InputFrame::Mono(1.0)], &mut output, events)
                .unwrap(),
            Activity::Normal
        );
        assert_eq!(engine.wet_post_controls().wet_eq.low_cut_hz, 90.0);
        let mut silence = [[0.0; 2]; 1];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0)],
                    &mut silence,
                    ControlBlock::<0>::new(1),
                )
                .unwrap(),
            Activity::Tail(108)
        );
    }

    #[test]
    fn enabling_wet_eq_during_a_tail_extends_existing_activity() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0; 5]),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            10,
        );
        engine.set_wet_post_tail_config(WetPostTailConfig {
            wet_eq_settle_samples: 100,
            modulation_history_samples: 40,
        });
        let mut first = [[0.0; 2]; 1];
        engine
            .process_block(
                &[InputFrame::Mono(1.0)],
                &mut first,
                ControlBlock::<0>::new(1),
            )
            .unwrap();

        let mut events = ControlBlock::<1>::new(1);
        events
            .push(TimedControl {
                sample_offset: 0,
                change: ControlChange::LowCutHz(90.0),
            })
            .unwrap();
        let mut silence = [[0.0; 2]; 1];
        assert_eq!(
            engine
                .process_block(&[InputFrame::Mono(0.0)], &mut silence, events)
                .unwrap(),
            Activity::Tail(113)
        );
    }

    #[test]
    fn wet_post_width_acts_after_the_stereo_response_pair() {
        let left = [1.0];
        let right = [-1.0];
        let response = PreparedResponse::from_channels(
            ResponseInterpretation::MonoToStereo,
            &[left.as_slice(), right.as_slice()],
            48_000.0,
        )
        .unwrap();
        let mut engine = AudioEngine::new(
            response,
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            1,
        );
        engine.try_enable_wet_post(48_000.0).unwrap();
        engine.set_wet_post_controls_immediate(WetPostControls {
            width: 0.0,
            ..WetPostControls::default()
        });
        let mut output = [[1.0; 2]; 1];
        engine
            .process_block(
                &[InputFrame::Mono(1.0)],
                &mut output,
                ControlBlock::<0>::new(1),
            )
            .unwrap();
        assert_eq!(output[0], [0.0, 0.0]);
    }

    #[test]
    fn predelay_moves_the_wet_impulse_without_moving_dry() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            8,
            LiveControls {
                mix: 1.0,
                pre_delay_samples: 3,
            },
            1,
        );
        let input = [
            InputFrame::Mono(1.0),
            InputFrame::Mono(0.0),
            InputFrame::Mono(0.0),
            InputFrame::Mono(0.0),
            InputFrame::Mono(0.0),
        ];
        let mut output = [[0.0; 2]; 5];
        engine
            .process_block(&input, &mut output, ControlBlock::<0>::new(5))
            .unwrap();
        assert_eq!(output[0], [0.0, 0.0]);
        assert_eq!(output[3], [1.0, 1.0]);
    }

    #[test]
    fn increasing_predelay_during_silence_extends_tail_until_exposed_history_is_gone() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            16,
            LiveControls {
                mix: 1.0,
                pre_delay_samples: 0,
            },
            1,
        );
        let mut output = [[0.0; 2]; 1];
        engine
            .process_block(
                &[InputFrame::Mono(1.0)],
                &mut output,
                ControlBlock::<0>::new(1),
            )
            .unwrap();
        assert_eq!(output[0], [1.0, 1.0]);
        engine
            .process_block(
                &[InputFrame::Mono(0.0)],
                &mut output,
                ControlBlock::<0>::new(1),
            )
            .unwrap();

        engine.apply_control(ControlChange::PreDelaySamples(8));
        let mut before = [[0.0; 2]; 6];
        assert!(matches!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0); 6],
                    &mut before,
                    ControlBlock::<0>::new(6),
                )
                .unwrap(),
            Activity::Tail(_)
        ));
        assert!(before.iter().all(|frame| *frame == [0.0, 0.0]));
        let mut exposed = [[0.0; 2]; 1];
        assert!(matches!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0)],
                    &mut exposed,
                    ControlBlock::<0>::new(1),
                )
                .unwrap(),
            Activity::Tail(_)
        ));
        assert_eq!(exposed[0], [1.0, 1.0]);

        let mut silence = [[0.0; 2]; 32];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0); 32],
                    &mut silence,
                    ControlBlock::<0>::new(32),
                )
                .unwrap(),
            Activity::Normal
        );
        assert!(silence.iter().all(|frame| *frame == [0.0, 0.0]));
        let mut wake = [[1.0; 2]; 4];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0); 4],
                    &mut wake,
                    ControlBlock::<0>::new(4),
                )
                .unwrap(),
            Activity::Normal
        );
        assert!(wake.iter().all(|frame| *frame == [0.0, 0.0]));
    }

    #[test]
    fn response_change_is_convex_and_old_storage_waits_for_retirement() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            1,
        );
        engine
            .begin_response_transition(mono_response(&[-1.0]), 3)
            .unwrap();
        let mut output = [[0.0; 2]; 3];
        engine
            .process_block(
                &[InputFrame::Mono(1.0); 3],
                &mut output,
                ControlBlock::<0>::new(3),
            )
            .unwrap();
        assert_eq!(output[0], [1.0, 1.0]);
        assert_eq!(output[1], [0.0, 0.0]);
        assert_eq!(output[2], [-1.0, -1.0]);
        assert!(!engine.transition_active());
        assert!(
            engine
                .begin_response_transition(mono_response(&[0.5]), 2)
                .is_err()
        );
        assert!(engine.take_retired());
        assert!(
            engine
                .begin_response_transition(mono_response(&[0.5]), 2)
                .is_ok()
        );
    }

    #[test]
    fn maximum_early_tier_replacement_runs_both_engines_for_the_full_crossfade() {
        let old: Vec<_> = (0..crate::convolver::LATE_OFFSET_SAMPLES)
            .map(|index| (index as f32 * 0.013).sin() * 0.01)
            .collect();
        let new: Vec<_> = old.iter().map(|sample| -*sample).collect();
        let controls = LiveControls {
            mix: 1.0,
            ..LiveControls::default()
        };
        let mut engine = AudioEngine::new(mono_response(&old), 0, controls, 64);
        let candidate = AudioEngine::new(mono_response(&new), 0, controls, 64);
        let transition_samples = 7_680; // 20 ms at the maximum 384 kHz processing rate.
        assert!(
            engine
                .begin_engine_transition(candidate, controls, transition_samples)
                .is_ok()
        );

        let input = [InputFrame::Stereo([0.1, -0.1]); 64];
        let mut output = [[0.0; 2]; 64];
        for block in 0..transition_samples / 64 {
            assert!(engine.transition_active());
            engine
                .process_block(&input, &mut output, ControlBlock::<0>::new(64))
                .unwrap();
            assert!(output.iter().flatten().all(|sample| sample.is_finite()));
            if block + 1 < transition_samples / 64 {
                assert!(engine.transition_active());
            }
        }
        assert!(!engine.transition_active());
        assert!(engine.take_retired());
    }

    #[test]
    fn reset_makes_later_silence_exact_after_partial_fft_block() {
        let response = vec![0.25; crate::convolver::PARTITION_SAMPLES + 3];
        let mut engine = AudioEngine::new(
            mono_response(&response),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            1,
        );
        let input = vec![InputFrame::Mono(1.0); 17];
        let mut output = vec![[0.0; 2]; 17];
        engine
            .process_block(&input, &mut output, ControlBlock::<0>::new(17))
            .unwrap();
        engine.reset();

        let silence = vec![InputFrame::Mono(0.0); 100];
        let mut after = vec![[1.0; 2]; 100];
        engine
            .process_block(&silence, &mut after, ControlBlock::<0>::new(100))
            .unwrap();
        assert!(after.iter().all(|frame| *frame == [0.0, 0.0]));
    }

    #[test]
    fn off_response_change_commits_without_a_stalled_audio_transition() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            0,
            LiveControls {
                mix: 0.0,
                ..LiveControls::default()
            },
            4,
        );
        engine
            .begin_response_transition(mono_response(&[2.0]), 100)
            .unwrap();
        assert!(!engine.transition_active());
        assert!(engine.take_retired());

        engine.apply_control(ControlChange::Mix(1.0));
        let mut output = [[0.0; 2]; 4];
        engine
            .process_block(
                &[InputFrame::Mono(1.0); 4],
                &mut output,
                ControlBlock::<0>::new(4),
            )
            .unwrap();
        assert_eq!(output[3], [2.0, 2.0]);
    }

    #[test]
    fn mix_off_finishes_a_long_response_transition_and_retires_old_storage() {
        let mut engine = AudioEngine::new(
            mono_response(&[1.0]),
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            2,
        );
        engine
            .begin_response_transition(mono_response(&[-1.0]), 100)
            .unwrap();
        engine.apply_control(ControlChange::Mix(0.0));
        let mut output = [[0.0; 2]; 2];
        engine
            .process_block(
                &[InputFrame::Mono(1.0); 2],
                &mut output,
                ControlBlock::<0>::new(2),
            )
            .unwrap();
        assert!(!engine.transition_active());
        assert!(engine.take_retired());
        assert_eq!(output[1], [1.0, 1.0]);
    }

    #[test]
    fn malformed_initial_controls_recover_to_finite_defaults() {
        let engine = AudioEngine::new(
            mono_response(&[1.0]),
            0,
            LiveControls {
                mix: f32::NAN,
                pre_delay_samples: 99,
            },
            1,
        );
        assert_eq!(engine.controls(), LiveControls::default());
    }
}
