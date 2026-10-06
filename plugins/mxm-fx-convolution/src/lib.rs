//! `mxm-fx-convolution` — an original WAV-response convolution effect.
//!
//! The framework-free DSP implements Gardner's zero-delay partitioned FIR architecture. This shell
//! owns permanent host identity, legacy and wet-post live controls, path-independent response
//! state, explicit mono/stereo layouts, bounded callback spans, activity reporting, and atomic
//! telemetry. Response acquisition and reversible preparation are prepared completely off audio.

macro_rules! plugin_name {
    () => {
        "mxm-fx-convolution"
    };
}

pub const NAME: &str = plugin_name!();
pub const CLAP_ID: &str = concat!("dk.mxm.", plugin_name!());

pub mod deferred;
pub mod editor;
pub mod impulses;
pub mod params;
pub mod preset;
pub mod response;
pub mod telemetry;

use mxm_fx_convolution_dsp::{
    Activity, AudioEngine, ControlBlock, ControlChange, InputFrame, LiveControls, PreparedResponse,
    ResponseInterpretation, RetiredResponse, WetEqControls, WetPostControls,
};
use nice_plug::prelude::*;
use params::{HIGH_CUT_OPEN_HZ, MAX_PRE_DELAY_SECONDS, MxmFxConvolutionParams};
use std::path::PathBuf;
use std::sync::Arc;
use telemetry::Telemetry;

const MAX_BLOCK_SIZE: usize = 64;
pub(crate) const CONTROL_RAMP_SECONDS: f32 = 0.012;
const RESPONSE_TRANSITION_SECONDS: f32 = 0.020;
const REVISION_FIVE_PARAMETER_IDS: [&str; 3] = ["mix", "wetgain", "predelay"];
const REVISION_SIX_NEUTRAL_BASELINE: [(&str, f32); 5] = [
    ("lowcut", 0.0),
    ("highcut", 1.0),
    ("tone", 0.5),
    ("width", 0.5),
    ("modulation", 0.0),
];
const REVISION_SEVEN_NEUTRAL_BASELINE: (&str, f32) = ("feedback", 0.0);
const REVISION_EIGHT_NEUTRAL_BASELINE: (&str, f32) = ("predelaysync", 0.0);

// Boxing `RetiredResponse` here would allocate on the process callback. The large variant only
// moves already-allocated response storage into nice-plug's bounded background queue.
#[allow(clippy::large_enum_variant)]
pub enum EditorTask {
    LoadWav {
        request: u64,
        path: PathBuf,
    },
    PrepareResponse {
        request: u64,
        model: response::ResponseModel,
    },
    PreparePreset {
        work: deferred::DeferredPresetWork,
    },
    AcknowledgePreset {
        request: u64,
    },
    /// Moves response allocation destruction off the process callback.
    RetireResponse {
        response: RetiredResponse,
    },
    /// Carries nothing: scheduled on the GUI path it asks the host for a process callback, which
    /// wakes an effect a host has put to sleep (`vendor/nice-plug`'s GUI-task process-wake patch).
    /// Sent after a preset is published, whose acknowledgement needs that callback.
    Wake,
}

pub struct MxmFxConvolution {
    pub params: Arc<MxmFxConvolutionParams>,
    telemetry: Arc<Telemetry>,
    deferred_presets: Arc<deferred::DeferredPresetController>,
    engine: AudioEngine,
    sample_rate: f32,
    input_channels: usize,
    activated: bool,
    wet_available: bool,
    response_fingerprint: u64,
    /// nice-plug calls `reset()` immediately after a state-triggered reactivation. Preserve the
    /// old response leg for that one reset so the prepared publication can crossfade.
    preserve_transition_on_reset: bool,
    /// One fully prepared latest-wins successor. It is stored but never processed until the current
    /// two-engine transition has retired, so rapid restores preserve both sounding histories.
    queued_engine: Option<AudioEngine>,
    input_scratch: [InputFrame; MAX_BLOCK_SIZE],
    output_scratch: [[f32; 2]; MAX_BLOCK_SIZE],
    /// Pre-delay as its sync resolved it for this call, or `None` for the knob.
    synced_pre_delay_s: Option<f32>,
}

impl Default for MxmFxConvolution {
    fn default() -> Self {
        // Plugin construction stays cheap. `activate()` prepares the embedded response once the
        // host's processing rate is known. A one-sample `[1.0]` has a flat spectrum, so its loop
        // bound is the damping peak alone and is the same at any plausible rate.
        let fallback =
            PreparedResponse::from_channels(ResponseInterpretation::Mono, &[&[1.0]], 48_000.0)
                .expect("the one-sample finite fallback response is valid");
        Self {
            params: Arc::new(MxmFxConvolutionParams::default()),
            telemetry: Telemetry::shared(),
            deferred_presets: Arc::new(deferred::DeferredPresetController::with_impulses(
                impulses::root(),
            )),
            engine: AudioEngine::new(fallback, 1, LiveControls::default(), 1),
            sample_rate: 48_000.0,
            input_channels: 1,
            activated: false,
            wet_available: true,
            response_fingerprint: 0,
            preserve_transition_on_reset: false,
            queued_engine: None,
            input_scratch: [InputFrame::Mono(0.0); MAX_BLOCK_SIZE],
            output_scratch: [[0.0; 2]; MAX_BLOCK_SIZE],
            synced_pre_delay_s: None,
        }
    }
}

impl MxmFxConvolution {
    /// Pre-delay in force: its division while synced to a tempo, the knob otherwise. The DSP owns
    /// the tap's crossfade, so a division arrives as smoothly as a turned knob.
    fn pre_delay_s(&self) -> f32 {
        self.synced_pre_delay_s
            .unwrap_or_else(|| self.params.pre_delay.value())
    }

    fn target_controls(&self, sample_rate: f32) -> LiveControls {
        LiveControls {
            mix: self.params.mix.value(),
            pre_delay_samples: seconds_to_samples(self.pre_delay_s(), sample_rate),
        }
    }

    fn target_wet_post_controls(&self) -> WetPostControls {
        WetPostControls {
            wet_eq: WetEqControls {
                low_cut_hz: self.params.low_cut.value(),
                high_cut_hz: self.params.high_cut.value(),
                tone_db: self.params.tone.value(),
            },
            modulation: self.params.modulation.value(),
            width: self.params.width.value(),
        }
    }

    fn apply_wet_post_controls(&mut self) {
        let controls = self.target_wet_post_controls();
        self.engine
            .apply_control(ControlChange::LowCutHz(controls.wet_eq.low_cut_hz));
        self.engine
            .apply_control(ControlChange::HighCutHz(controls.wet_eq.high_cut_hz));
        self.engine
            .apply_control(ControlChange::ToneDb(controls.wet_eq.tone_db));
        self.engine
            .apply_control(ControlChange::Modulation(controls.modulation));
        self.engine
            .apply_control(ControlChange::Width(controls.width));
    }

    fn apply_feedback_control(&mut self) {
        self.engine
            .apply_control(ControlChange::Feedback(self.params.feedback.value()));
    }

    fn activate_without_response(
        &mut self,
        sample_rate: f32,
        input_channels: usize,
        fingerprint: u64,
        rejection: telemetry::ResponseRejection,
    ) -> bool {
        // Host activation has a wider envelope than response preparation. Keep the canonical source
        // untouched and make the unsupported wet path inert until a later supported reactivation.
        self.engine.take_retired();
        self.queued_engine = None;
        self.engine.reset();
        self.sample_rate = sample_rate;
        self.input_channels = input_channels;
        self.response_fingerprint = fingerprint;
        self.preserve_transition_on_reset = false;
        self.wet_available = false;
        self.activated = true;
        self.telemetry.reject_response(rejection);
        true
    }

    fn prepare(&mut self, sample_rate: f32, input_channels: usize) -> bool {
        // Forget the last activation's tempo too: nice-plug resets right after activating, and a
        // division resolved from a tempo the host may since have changed would seed the engine.
        self.telemetry.tempo.publish(None);
        // A restored state is resolved afresh by the next block: activation must not seed the
        // engine with the division the previous state was synced to.
        self.synced_pre_delay_s = None;
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return false;
        }
        self.params.response.set_processing_rate(sample_rate);
        if self.params.response.take_preparation_rejected() {
            return false;
        }
        let fingerprint = self.params.response.map(response::fingerprint);
        let input_channels = input_channels.clamp(1, 2);
        if !(8_000.0..=response::MAX_SOURCE_RATE as f32).contains(&sample_rate) {
            return self.activate_without_response(
                sample_rate,
                input_channels,
                fingerprint,
                telemetry::ResponseRejection::UnsupportedRate(sample_rate),
            );
        }

        let controls = self.target_controls(sample_rate);
        let same_engine = self.activated
            && self.wet_available
            && self.sample_rate == sample_rate
            && self.input_channels == input_channels
            && self.response_fingerprint == fingerprint;
        if same_engine {
            self.engine.apply_control(ControlChange::Mix(controls.mix));
            self.engine
                .apply_control(ControlChange::PreDelaySamples(controls.pre_delay_samples));
            self.apply_wet_post_controls();
            self.apply_feedback_control();
            self.telemetry.clear_response_rejection();
            return true;
        }

        // Editor/preset publication consumes the fully allocated background candidate. Initial
        // activation and a genuine sample-rate change prepare here, where activation permits work.
        let mut candidate = if let Some(engine) = self
            .params
            .response
            .take_prepared_engine(fingerprint, sample_rate)
        {
            engine
        } else {
            let prepared = match self
                .params
                .response
                .map(|state| response::prepare(state, sample_rate))
            {
                Ok(prepared) => prepared,
                Err(response::StateError::Deadline) => {
                    return self.activate_without_response(
                        sample_rate,
                        input_channels,
                        fingerprint,
                        telemetry::ResponseRejection::Deadline(sample_rate),
                    );
                }
                Err(_) => return false,
            };
            let maximum_pre_delay = seconds_to_samples(MAX_PRE_DELAY_SECONDS, sample_rate) as usize;
            let ramp_samples = seconds_to_samples(CONTROL_RAMP_SECONDS, sample_rate).max(1);
            let Ok(mut engine) =
                AudioEngine::try_new(prepared, maximum_pre_delay, controls, ramp_samples)
            else {
                return false;
            };
            if engine.try_enable_wet_post(sample_rate).is_err() {
                return false;
            }
            engine
        };
        candidate.set_controls_immediate(controls);
        candidate.set_wet_post_controls_immediate(self.target_wet_post_controls());
        candidate.set_feedback_immediate(self.params.feedback.value());

        let can_transition = self.activated
            && self.wet_available
            && self.sample_rate == sample_rate
            && self.input_channels == input_channels
            && self.response_fingerprint != fingerprint;
        self.preserve_transition_on_reset = false;
        if can_transition {
            self.engine.take_retired();
            let transition_samples =
                seconds_to_samples(RESPONSE_TRANSITION_SECONDS, sample_rate).max(1);
            match self
                .engine
                .begin_engine_transition(candidate, controls, transition_samples)
            {
                Ok(()) => {
                    self.queued_engine = None;
                    self.preserve_transition_on_reset = true;
                }
                Err(candidate) => {
                    // Keep only the newest completely prepared successor. The sounding transition
                    // and its recursive histories continue untouched; the callback starts this
                    // candidate only after moving the first retired leg to background destruction.
                    self.queued_engine = Some(candidate);
                    self.preserve_transition_on_reset = true;
                }
            }
        } else {
            self.queued_engine = None;
            self.engine = candidate;
            self.engine.reset();
        }
        self.sample_rate = sample_rate;
        self.input_channels = input_channels;
        self.response_fingerprint = fingerprint;
        self.wet_available = true;
        self.activated = true;
        self.telemetry.clear_response_rejection();
        true
    }

    pub fn prepare_for_test(&mut self, sample_rate: f32, input_channels: usize) -> bool {
        self.prepare(sample_rate, input_channels)
    }

    pub fn process_block_for_test(&mut self, channels: &mut [&mut [f32]]) -> ProcessStatus {
        let status = self.process_slices(channels);
        // Tests call from an ordinary thread, so retired storage may be destroyed synchronously.
        let _ = self.service_queued_transition();
        status
    }

    /// Start a retained successor once the audible transition has completed. The returned retired
    /// storage must be destroyed off audio; moving it into a background task is allocation-free.
    fn service_queued_transition(&mut self) -> Option<RetiredResponse> {
        if self.queued_engine.is_none() || self.engine.transition_active() {
            return None;
        }
        let retired = self.engine.take_retired_response();
        let mut candidate = self.queued_engine.take().expect("checked above");
        let controls = self.target_controls(self.sample_rate);
        candidate.set_controls_immediate(controls);
        candidate.set_wet_post_controls_immediate(self.target_wet_post_controls());
        candidate.set_feedback_immediate(self.params.feedback.value());
        let transition_samples =
            seconds_to_samples(RESPONSE_TRANSITION_SECONDS, self.sample_rate).max(1);
        match self
            .engine
            .begin_engine_transition(candidate, controls, transition_samples)
        {
            Ok(()) => retired,
            Err(candidate) => {
                self.queued_engine = Some(candidate);
                retired
            }
        }
    }

    pub fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }

    /// Process host-owned in-place slices in bounded spans.
    fn process_slices(&mut self, channels: &mut [&mut [f32]]) -> ProcessStatus {
        let Some(first) = channels.first() else {
            return ProcessStatus::Normal;
        };
        let samples = first.len();
        if samples == 0 {
            return ProcessStatus::Normal;
        }
        if !self.wet_available {
            return self.process_dry_slices(channels);
        }

        // nice-plug splits process calls at host automation events. Mix, the predelay tap and the
        // five wet-post controls begin their DSP-owned ramps at that boundary.
        self.engine
            .apply_control(ControlChange::Mix(self.params.mix.value()));
        self.engine
            .apply_control(ControlChange::PreDelaySamples(seconds_to_samples(
                self.pre_delay_s(),
                self.sample_rate,
            )));
        self.apply_wet_post_controls();
        self.apply_feedback_control();

        let mut start = 0;
        let mut activity = Activity::Normal;
        let mut peak = 0.0f32;
        let mut input_fault = false;
        let mut has_input = false;
        while start < samples {
            let count = (samples - start).min(MAX_BLOCK_SIZE);
            for offset in 0..count {
                let index = start + offset;
                let left = channels[0][index];
                input_fault |= !left.is_finite();
                has_input |= left.is_finite() && left.abs() >= f32::MIN_POSITIVE;
                self.input_scratch[offset] = if self.input_channels == 1 || channels.len() < 2 {
                    InputFrame::Mono(left)
                } else {
                    let right = channels[1][index];
                    input_fault |= !right.is_finite();
                    has_input |= right.is_finite() && right.abs() >= f32::MIN_POSITIVE;
                    InputFrame::Stereo([left, right])
                };
            }

            let controls = ControlBlock::<0>::new(count);

            activity = match self.engine.process_block(
                &self.input_scratch[..count],
                &mut self.output_scratch[..count],
                controls,
            ) {
                Ok(activity) => activity,
                Err(_) => return ProcessStatus::Error("convolution block shape mismatch"),
            };

            if channels.len() >= 2 {
                let (left, rest) = channels.split_at_mut(1);
                let right = &mut rest[0];
                for offset in 0..count {
                    let index = start + offset;
                    let frame = self.output_scratch[offset];
                    left[0][index] = frame[0];
                    right[index] = frame[1];
                    peak = peak.max(frame[0].abs()).max(frame[1].abs());
                }
            } else {
                for offset in 0..count {
                    let index = start + offset;
                    let sample = self.output_scratch[offset][0];
                    channels[0][index] = sample;
                    peak = peak.max(sample.abs());
                }
            }
            start += count;
        }

        let tail_samples = match activity {
            Activity::Normal => 0,
            Activity::Tail(samples) => samples,
            Activity::KeepAlive => u64::MAX,
        };
        self.telemetry.publish(
            peak,
            tail_samples,
            self.engine.clipped(),
            input_fault || self.engine.numeric_fault(),
        );
        self.engine.clear_telemetry();

        match activity {
            Activity::KeepAlive => ProcessStatus::KeepAlive,
            Activity::Normal | Activity::Tail(_) if has_input => ProcessStatus::Normal,
            Activity::Normal => ProcessStatus::Normal,
            Activity::Tail(samples) => ProcessStatus::Tail(samples.min(u32::MAX as u64) as u32),
        }
    }

    fn process_dry_slices(&mut self, channels: &mut [&mut [f32]]) -> ProcessStatus {
        let samples = channels[0].len();
        let mut peak = 0.0f32;
        let mut input_fault = false;
        if channels.len() >= 2 {
            let (left, rest) = channels.split_at_mut(1);
            let right = &mut rest[0];
            for index in 0..samples {
                let dry_left = finite_sample(left[0][index], &mut input_fault);
                let dry_right = if self.input_channels == 1 {
                    dry_left
                } else {
                    finite_sample(right[index], &mut input_fault)
                };
                left[0][index] = dry_left;
                right[index] = dry_right;
                peak = peak.max(dry_left.abs()).max(dry_right.abs());
            }
        } else {
            for sample in &mut channels[0][..samples] {
                *sample = finite_sample(*sample, &mut input_fault);
                peak = peak.max(sample.abs());
            }
        }
        self.telemetry.publish(peak, 0, false, input_fault);
        ProcessStatus::Normal
    }
}

/// Canonicalize a valid response and retain enough information to migrate an exact Revision 5
/// loaded-preset identity without laundering an already-modified response back to Clean.
fn canonicalize_response_state(serialized: &mut String) -> Option<(bool, u64, u64)> {
    let was_revision_five = serde_json::from_str::<serde_json::Value>(serialized)
        .ok()
        .and_then(|value| value.get("preparation").cloned())
        .and_then(|preparation| preparation.as_object().cloned())
        .is_some_and(|preparation| {
            !preparation.contains_key("decay_percent")
                && !preparation.contains_key("damping_percent")
        });
    let response =
        nice_plug::params::persist::deserialize_field::<response::ResponseState>(serialized)
            .ok()
            .filter(|response| response::validate(response).is_ok())?;
    let old_fingerprint = response::revision_five_fingerprint(&response);
    let canonical_fingerprint = response::fingerprint(&response);
    *serialized = nice_plug::params::persist::serialize_field(&response).ok()?;
    Some((was_revision_five, old_fingerprint, canonical_fingerprint))
}

/// Extend only exact clean legacy identity shapes. A mismatched response fingerprint remains
/// Modified rather than being laundered back to Clean.
fn migrate_legacy_identity(
    state: &mut PluginState,
    was_revision_five: bool,
    old_fingerprint: u64,
    canonical_fingerprint: u64,
    migrate_high_cut_open: bool,
) {
    let Some(serialized) = state.fields.get_mut("preset") else {
        return;
    };
    let Ok(mut identity) =
        nice_plug::params::persist::deserialize_field::<mxm_preset::PresetIdentity>(serialized)
    else {
        return;
    };
    if identity.loaded.is_none() {
        return;
    }

    if migrate_high_cut_open && let Some(high_cut) = identity.baseline.get_mut("highcut") {
        *high_cut = params::migrate_legacy_high_cut_normalized(*high_cut);
    }

    let exact_revision_five_baseline = identity.baseline.len() == REVISION_FIVE_PARAMETER_IDS.len()
        && REVISION_FIVE_PARAMETER_IDS
            .iter()
            .all(|id| identity.baseline.contains_key(*id));
    if was_revision_five
        && exact_revision_five_baseline
        && identity.state_fingerprint == Some(old_fingerprint)
    {
        identity.baseline.extend(
            REVISION_SIX_NEUTRAL_BASELINE
                .into_iter()
                .map(|(id, value)| (id.to_owned(), value)),
        );
        identity.state_fingerprint = Some(canonical_fingerprint);
    }

    let exact_revision_six_baseline = identity.baseline.len()
        == REVISION_FIVE_PARAMETER_IDS.len() + REVISION_SIX_NEUTRAL_BASELINE.len()
        && REVISION_FIVE_PARAMETER_IDS
            .iter()
            .chain(REVISION_SIX_NEUTRAL_BASELINE.iter().map(|(id, _)| id))
            .all(|id| identity.baseline.contains_key(*id));
    if exact_revision_six_baseline && identity.state_fingerprint == Some(canonical_fingerprint) {
        identity.baseline.insert(
            REVISION_SEVEN_NEUTRAL_BASELINE.0.to_owned(),
            REVISION_SEVEN_NEUTRAL_BASELINE.1,
        );
    }

    // Revision 8 added Pre-delay's tempo sync, off in every earlier state.
    let exact_revision_seven_baseline = identity.baseline.len()
        == REVISION_FIVE_PARAMETER_IDS.len() + REVISION_SIX_NEUTRAL_BASELINE.len() + 1
        && REVISION_FIVE_PARAMETER_IDS
            .iter()
            .chain(REVISION_SIX_NEUTRAL_BASELINE.iter().map(|(id, _)| id))
            .chain(std::iter::once(&REVISION_SEVEN_NEUTRAL_BASELINE.0))
            .all(|id| identity.baseline.contains_key(*id));
    if exact_revision_seven_baseline && identity.state_fingerprint == Some(canonical_fingerprint) {
        identity.baseline.insert(
            REVISION_EIGHT_NEUTRAL_BASELINE.0.to_owned(),
            REVISION_EIGHT_NEUTRAL_BASELINE.1,
        );
    }

    // Wet gain was deleted on 2026-09-28; a baseline that still names it would read as modified
    // for ever, since no live parameter can match it.
    identity.baseline.remove("wetgain");

    if let Ok(canonical) = nice_plug::params::persist::serialize_field(&identity) {
        *serialized = canonical;
    }
}

impl Plugin for MxmFxConvolution {
    const NAME: &'static str = NAME;
    const VENDOR: &'static str = "mxm";
    const URL: &'static str = "https://mxm.dk";
    const EMAIL: &'static str = "plugins@mxm.dk";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    // The wrapper splits callbacks at parameter events. Internal convolution spans remain capped.
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type Editor = editor::MxmFxConvolutionEditor;
    type SysExMessage = ();
    type BackgroundTask = EditorTask;

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = Arc::clone(&self.params);
        let deferred_presets = Arc::clone(&self.deferred_presets);
        Box::new(move |task| match task {
            EditorTask::LoadWav { request, path } => {
                if let Err(error) = params.response.import_wav(request, &path) {
                    params.response.fail_request(request, error);
                }
            }
            EditorTask::PrepareResponse { request, model } => {
                if let Err(error) = params.response.prepare_edit(request, model) {
                    params.response.fail_request(request, error);
                }
            }
            EditorTask::PreparePreset { work } => deferred_presets.prepare(&params, work),
            EditorTask::AcknowledgePreset { request } => {
                let _ = deferred_presets.acknowledge(request, &params);
            }
            EditorTask::RetireResponse { response } => drop(response),
            EditorTask::Wake => {}
        })
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        editor::create(
            Arc::clone(&self.params),
            Arc::clone(&self.telemetry),
            Arc::clone(&self.deferred_presets),
            async_executor,
        )
    }

    fn filter_state(state: &mut PluginState) {
        // Revision 6 represented Open High cut as plain zero. Its corrected control puts a real
        // closed 0 Hz cutoff at the bottom and the real 20 kHz maximum at the conventional top.
        let legacy_high_cut_layout = (state.version.is_empty() || state.version == "0.1.0")
            && state.params.contains_key("highcut");
        if legacy_high_cut_layout
            && let Some(nice_plug::plugin::ParamValue::F32(high_cut)) =
                state.params.get_mut("highcut")
            && *high_cut == 0.0
        {
            *high_cut = HIGH_CUT_OPEN_HZ;
        }

        let migration = state
            .fields
            .get_mut("response")
            .map(canonicalize_response_state);
        match migration {
            None => {}
            Some(Some((was_revision_five, old_fingerprint, canonical_fingerprint))) => {
                migrate_legacy_identity(
                    state,
                    was_revision_five,
                    old_fingerprint,
                    canonical_fingerprint,
                    legacy_high_cut_layout,
                );
                // Missing Revision 7 Feedback means exact structural bypass even when an older
                // state is loaded over a currently recursive patch.
                state.params.entry("feedback".to_owned()).or_insert(
                    nice_plug::plugin::ParamValue::F32(REVISION_SEVEN_NEUTRAL_BASELINE.1),
                );
                // And a state from before Revision 8 is unsynced, whatever this instance was.
                state
                    .params
                    .entry(REVISION_EIGHT_NEUTRAL_BASELINE.0.to_owned())
                    .or_insert(nice_plug::plugin::ParamValue::Bool(false));
            }
            Some(None) => {
                // nice-plug's hook cannot reject. Clearing both maps makes malformed embedded source
                // a complete no-op instead of applying parameters around the previous response.
                state.params.clear();
                state.fields.clear();
            }
        }
    }

    fn activate(
        &mut self,
        layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.prepare(
            config.sample_rate,
            layout
                .main_input_channels
                .map_or(1, |channels| channels.get() as usize),
        )
    }

    fn reset(&mut self) {
        // A host resets without a callback between (a bypass, a transport restart), and a parameter
        // flush may have moved a sync meanwhile: re-resolve every sync from the parameters as they
        // stand and the last tempo seen, so nothing is seeded from the previous division.
        self.synced_pre_delay_s = self.params.synced_pre_delay(self.telemetry.tempo.get());
        if !self.wet_available {
            self.engine.reset();
            self.telemetry.publish(0.0, 0, false, false);
            return;
        }
        self.engine
            .apply_control(ControlChange::Mix(self.params.mix.value()));
        self.engine
            .apply_control(ControlChange::PreDelaySamples(seconds_to_samples(
                self.pre_delay_s(),
                self.sample_rate,
            )));
        self.apply_wet_post_controls();
        self.apply_feedback_control();
        self.engine
            .set_wet_post_controls_immediate(self.target_wet_post_controls());
        self.engine
            .set_feedback_immediate(self.params.feedback.value());
        if self.preserve_transition_on_reset {
            // State restore has already published a complete new primary. This one wrapper-issued
            // reset is not a transport reset; keeping both histories makes the later audio a
            // bounded old/new response crossfade rather than a hard coefficient switch.
            self.preserve_transition_on_reset = false;
        } else {
            self.engine.reset();
        }
        self.telemetry.publish(0.0, 0, false, false);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Observe unmodulated bases at every wrapper-split process boundary. Parameter modulation
        // may change the audible value, but cannot advance deferred-preset base revisions.
        self.params.observe_parameter_edits();
        // Pre-delay's tempo sync, once per call, and the tempo in force for the editor's reading.
        let tempo = context.transport().tempo;
        self.synced_pre_delay_s = self.params.synced_pre_delay(tempo);
        self.telemetry.tempo.publish(tempo);
        if let Some(request) = self.deferred_presets.acknowledge_process_boundary() {
            context.execute_background(EditorTask::AcknowledgePreset { request });
        }
        let status = self.process_slices(buffer.as_slice());
        if let Some(response) = self.service_queued_transition() {
            context.execute_background(EditorTask::RetireResponse { response });
        }
        status
    }
}

impl ClapPlugin for MxmFxConvolution {
    const CLAP_ID: &'static str = CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A convolution reverb for your own impulse responses, with controls to reshape them");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Reverb,
        ClapFeature::Stereo,
    ];
}

nice_export_clap!(MxmFxConvolution);

fn finite_sample(sample: f32, fault: &mut bool) -> f32 {
    *fault |= !sample.is_finite();
    if sample.is_finite() && sample.abs() >= f32::MIN_POSITIVE {
        sample
    } else {
        0.0
    }
}

fn seconds_to_samples(seconds: f32, sample_rate: f32) -> u64 {
    if seconds.is_finite() && sample_rate.is_finite() {
        (seconds.max(0.0) as f64 * sample_rate.max(0.0) as f64)
            .round()
            .clamp(0.0, u64::MAX as f64) as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug::params::internals::ParamPtr;
    use nice_plug::params::{InternalParamMut, Param};
    use std::sync::Mutex;

    struct ApplyingHost;

    impl nice_plug::context::gui::GuiContextInner for ApplyingHost {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            unsafe {
                let _ = param._internal_set_normalized_value(normalized);
            }
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            PluginState {
                version: String::new(),
                params: Default::default(),
                fields: Default::default(),
            }
        }
        fn set_state(&self, _state: PluginState) {}
    }

    struct TestProcessContext {
        tasks: Mutex<Vec<EditorTask>>,
        transport: Transport,
    }

    impl TestProcessContext {
        fn new() -> Self {
            Self {
                tasks: Mutex::new(Vec::new()),
                transport: Transport::new(48_000.0),
            }
        }
    }

    impl ProcessContext<MxmFxConvolution> for TestProcessContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute_background(&self, task: EditorTask) {
            self.tasks.lock().unwrap().push(task);
        }
        fn execute_gui(&self, _task: EditorTask) {}
        fn transport(&self) -> &Transport {
            &self.transport
        }
        fn next_event(&mut self) -> Option<NoteEvent<()>> {
            None
        }
        fn send_event(&mut self, _event: NoteEvent<()>) {}
        fn set_latency_samples(&self, _samples: u32) {}
        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

    fn set(parameter: &FloatParam, value: f32) {
        set_normalized(parameter, parameter.preview_normalized(value));
    }

    fn set_normalized(parameter: &FloatParam, normalized: f32) {
        unsafe {
            let _ = parameter._internal_set_normalized_value(normalized);
            parameter._internal_update_smoother(48_000.0, true);
        }
    }

    fn prepared(input_channels: usize) -> MxmFxConvolution {
        let mut plugin = MxmFxConvolution::default();
        assert!(plugin.prepare_for_test(48_000.0, input_channels));
        plugin
    }

    #[test]
    fn process_callback_queues_the_ack_that_alone_changes_deferred_init_identity() {
        let mut plugin = prepared(2);
        mxm_preset::mark_loaded(&*plugin.params, "Before", mxm_preset::Origin::Factory);
        let (work, problems) = plugin
            .deferred_presets
            .begin(&plugin.params, mxm_preset::DeferredPresetUiRequest::Init)
            .unwrap();
        assert!(problems.is_empty());
        plugin.deferred_presets.prepare(&plugin.params, work);
        plugin
            .deferred_presets
            .service(&plugin.params, &ParamSetter::new(&ApplyingHost));
        assert_eq!(mxm_preset::loaded(&*plugin.params).name(), Some("Before"));

        let mut buffer = Buffer::default();
        let mut inputs = [];
        let mut outputs = [];
        let mut aux = AuxiliaryBuffers {
            inputs: &mut inputs,
            outputs: &mut outputs,
        };
        let mut context = TestProcessContext::new();
        assert_eq!(
            plugin.process(&mut buffer, &mut aux, &mut context),
            ProcessStatus::Normal
        );
        assert_eq!(mxm_preset::loaded(&*plugin.params).name(), Some("Before"));
        let task = context
            .tasks
            .lock()
            .unwrap()
            .pop()
            .expect("callback ack task");
        assert!(matches!(task, EditorTask::AcknowledgePreset { .. }));

        let executor = plugin.task_executor();
        executor(task);
        assert_eq!(
            mxm_preset::loaded(&*plugin.params),
            mxm_preset::Loaded::None
        );
    }

    #[test]
    fn name_id_and_bundle_have_one_source() {
        assert_eq!(NAME, "mxm-fx-convolution");
        assert_eq!(CLAP_ID, format!("dk.mxm.{NAME}"));

        mxm_plugin_test::bundle::is_named(env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"), NAME);
    }

    #[test]
    fn effect_has_two_explicit_layouts_and_no_note_port() {
        assert_eq!(MxmFxConvolution::AUDIO_IO_LAYOUTS.len(), 2);
        assert_eq!(
            MxmFxConvolution::AUDIO_IO_LAYOUTS[0].main_input_channels,
            NonZeroU32::new(1)
        );
        assert_eq!(
            MxmFxConvolution::AUDIO_IO_LAYOUTS[1].main_input_channels,
            NonZeroU32::new(2)
        );
        assert_eq!(MxmFxConvolution::MIDI_INPUT, MidiConfig::None);
    }

    #[test]
    fn mix_zero_settles_to_exact_dry_in_both_layouts() {
        for input_channels in [1, 2] {
            let mut plugin = prepared(input_channels);
            set(&plugin.params.mix, 0.0);
            let mut left = vec![0.25; 768];
            let mut right = vec![-0.125; 768];
            plugin.process_slices(&mut [&mut left, &mut right]);

            let expected_left: Vec<f32> = (0..257).map(|i| (i as f32 * 0.13).sin() * 0.3).collect();
            let expected_right: Vec<f32> =
                (0..257).map(|i| (i as f32 * 0.07).cos() * 0.2).collect();
            let mut left = expected_left.clone();
            let mut right = expected_right.clone();
            plugin.process_slices(&mut [&mut left, &mut right]);
            assert_eq!(left, expected_left);
            assert_eq!(
                right,
                if input_channels == 1 {
                    expected_left.clone()
                } else {
                    expected_right
                }
            );
        }
    }

    #[test]
    fn engaged_starter_response_changes_audio_and_reports_a_finite_tail() {
        let mut plugin = prepared(1);
        let mut left = vec![0.0; 127];
        let mut right = vec![0.0; 127];
        left[0] = 0.5;
        assert_eq!(
            plugin.process_slices(&mut [&mut left, &mut right]),
            ProcessStatus::Normal
        );
        assert_ne!(left[0], 0.5);

        left.fill(0.0);
        right.fill(0.0);
        assert!(matches!(
            plugin.process_slices(&mut [&mut left, &mut right]),
            ProcessStatus::Tail(samples) if samples > 0
        ));
    }

    fn render_digest(mut plugin: MxmFxConvolution) -> u64 {
        let frames = 32_768;
        let mut left: Vec<f32> = (0..frames)
            .map(|index| {
                if index < 513 {
                    (index as f32 * 0.037).sin() * 0.19
                } else {
                    0.0
                }
            })
            .collect();
        let mut right = vec![0.0; frames];
        plugin.process_slices(&mut [&mut left, &mut right]);
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for sample in left.into_iter().chain(right) {
            for byte in sample.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    /// **The starter's render, pinned.** Recaptured 2026-09-28 when every response was normalised
    /// and Wet gain deleted (measured: the starter's reverb rises 4.84 dB at 48 kHz, mean energy 0.33
    /// to one; the default Mix went from 38 % at -3 dB to 30 %); before that, the owner's
    /// 2026-09-17 High cut correction moved it. The fifty recipe digests beside it went with the
    /// recipes the same day, when the impulses folder became the factory set. This hash guards
    /// deterministic rendering and does not constitute listening approval.
    #[test]
    fn the_starter_render_matches_the_normalised_golden_digest() {
        const GOLDEN: u64 = 6364300162752919026;
        let rendered = render_digest(prepared(1));
        // Windows' bits: each platform's maths library rounds in its own way, so Linux and macOS
        // render other bits (the owner, 2026-10-06: pin on Windows only).
        if cfg!(target_os = "windows") {
            assert_eq!(rendered, GOLDEN);
        }
    }

    #[test]
    fn high_cut_public_endpoints_reach_the_dsp_as_real_frequencies() {
        let plugin = MxmFxConvolution::default();
        assert_eq!(
            plugin.target_wet_post_controls().wet_eq.high_cut_hz,
            HIGH_CUT_OPEN_HZ
        );
        set(&plugin.params.high_cut, 0.0);
        assert_eq!(plugin.target_wet_post_controls().wet_eq.high_cut_hz, 0.0);
    }

    #[test]
    fn wet_post_controls_are_live_and_finite() {
        let mut neutral = prepared(1);
        assert!(neutral.engine.wet_post_enabled());
        let mut engaged = MxmFxConvolution::default();
        set(&engaged.params.low_cut, 120.0);
        set(&engaged.params.high_cut, 8_000.0);
        set(&engaged.params.tone, 2.0);
        set(&engaged.params.width, 0.4);
        set(&engaged.params.modulation, 0.35);
        assert!(engaged.prepare_for_test(48_000.0, 1));
        assert!(engaged.engine.wet_post_enabled());

        let mut neutral_left = vec![0.0; 2_048];
        let mut neutral_right = vec![0.0; 2_048];
        let mut engaged_left = vec![0.0; 2_048];
        let mut engaged_right = vec![0.0; 2_048];
        neutral_left[0] = 0.5;
        engaged_left[0] = 0.5;
        neutral.process_slices(&mut [&mut neutral_left, &mut neutral_right]);
        engaged.process_slices(&mut [&mut engaged_left, &mut engaged_right]);
        assert_ne!(
            (engaged_left, engaged_right),
            (neutral_left, neutral_right),
            "engaged post path was inert"
        );

        let mut left = vec![0.0; 64];
        let mut right = vec![0.0; 64];
        engaged.process_slices(&mut [&mut left, &mut right]);
        assert!(left.iter().chain(&right).all(|sample| sample.is_finite()));
    }

    #[test]
    fn feedback_is_live_and_uncertified_recursion_reports_keep_alive() {
        let mut plugin = MxmFxConvolution::default();
        set(&plugin.params.mix, 1.0);
        set(&plugin.params.feedback, params::MAX_FEEDBACK);
        assert!(plugin.prepare_for_test(48_000.0, 1));

        let mut left = vec![0.0; 256];
        let mut right = vec![0.0; 256];
        left[0] = 0.5;
        let status = plugin.process_slices(&mut [&mut left, &mut right]);
        assert_eq!(status, ProcessStatus::KeepAlive);
        assert!(left.iter().chain(&right).all(|sample| sample.is_finite()));

        set(&plugin.params.feedback, 0.0);
        let mut silent_left = vec![0.0; 256];
        let mut silent_right = vec![0.0; 256];
        let status = plugin.process_slices(&mut [&mut silent_left, &mut silent_right]);
        assert_ne!(status, ProcessStatus::KeepAlive);
    }

    #[test]
    fn rapid_response_successor_preserves_recursive_history_and_waits_its_turn() {
        let mut plugin = MxmFxConvolution::default();
        set(&plugin.params.mix, 1.0);
        set(&plugin.params.feedback, 1.1);
        set(&plugin.params.pre_delay, 0.005);
        assert!(plugin.prepare_for_test(48_000.0, 2));

        let mut seed_left = vec![0.0; 1];
        let mut seed_right = vec![0.0; 1];
        seed_left[0] = 0.5;
        plugin.process_block_for_test(&mut [&mut seed_left, &mut seed_right]);
        assert_eq!(
            seed_left,
            [0.0],
            "the seeded wet sample is held by pre-delay"
        );

        let mut first = plugin.params.response.snapshot();
        first.preparation.reverse = !first.preparation.reverse;
        nice_plug::params::persist::PersistentField::set(&plugin.params.response, first);
        assert!(plugin.prepare_for_test(48_000.0, 2));
        assert!(plugin.engine.transition_active());

        let mut latest = plugin.params.response.snapshot();
        latest.preparation.decay_percent = 125;
        nice_plug::params::persist::PersistentField::set(&plugin.params.response, latest.clone());
        assert!(plugin.prepare_for_test(48_000.0, 2));
        assert!(plugin.queued_engine.is_some());

        let mut live_left = vec![0.0; 320];
        let mut live_right = vec![0.0; 320];
        plugin.process_block_for_test(&mut [&mut live_left, &mut live_right]);
        assert!(
            live_left
                .iter()
                .chain(&live_right)
                .any(|sample| *sample != 0.0),
            "the latest request must not reset the sounding pre-delay and recursive histories"
        );

        let mut settle_left = vec![0.0; 3_000];
        let mut settle_right = vec![0.0; 3_000];
        plugin.process_block_for_test(&mut [&mut settle_left, &mut settle_right]);
        assert!(plugin.queued_engine.is_none());
        assert!(plugin.engine.transition_active());
        plugin.process_block_for_test(&mut [&mut settle_left, &mut settle_right]);
        assert!(!plugin.engine.transition_active());
        assert_eq!(plugin.params.response.snapshot(), latest);
        assert_eq!(plugin.response_fingerprint, response::fingerprint(&latest));
    }

    #[test]
    fn active_response_restore_commits_before_return_and_survives_wrapper_reset() {
        let mut plugin = prepared(2);
        let old_fingerprint = plugin.response_fingerprint;
        {
            let mut state = plugin.params.response.snapshot();
            state.channels.swap(0, 1);
            nice_plug::params::persist::PersistentField::set(&plugin.params.response, state);
        }

        assert!(plugin.prepare_for_test(48_000.0, 2));
        assert_ne!(plugin.response_fingerprint, old_fingerprint);
        assert!(plugin.engine.transition_active());
        assert!(plugin.preserve_transition_on_reset);

        // This is the reset nice-plug issues after reactivation from active state load.
        plugin.reset();
        assert!(plugin.engine.transition_active());
        assert!(!plugin.preserve_transition_on_reset);
    }

    #[test]
    fn legacy_three_id_host_state_and_response_schema_restore_loaded_preset_clean() {
        let response = response::ResponseState::default();
        let legacy_name = "Revision 5 room";
        let legacy_values = [("mix", 0.25), ("wetgain", 0.75), ("predelay", 0.1)];
        let baseline_params = MxmFxConvolutionParams::default();
        let identity = mxm_preset::PresetIdentity {
            version: 1,
            loaded: Some(mxm_preset::LoadedPreset {
                name: legacy_name.to_owned(),
                origin: mxm_preset::Origin::Factory,
            }),
            baseline: [
                (
                    "mix".to_owned(),
                    baseline_params.mix.preview_normalized(legacy_values[0].1),
                ),
                // Stored before Wet gain was deleted: its normalised value, which nothing reads now.
                ("wetgain".to_owned(), 0.62),
                (
                    "predelay".to_owned(),
                    baseline_params
                        .pre_delay
                        .preview_normalized(legacy_values[2].1),
                ),
            ]
            .into_iter()
            .collect(),
            state_fingerprint: Some(response::revision_five_fingerprint(&response)),
        };
        let mut response_json = serde_json::to_value(&response).unwrap();
        response_json["preparation"]
            .as_object_mut()
            .unwrap()
            .remove("decay_percent");
        response_json["preparation"]
            .as_object_mut()
            .unwrap()
            .remove("damping_percent");
        let encoded = serde_json::to_string(&response_json).unwrap();
        let mut state = PluginState {
            version: "0.1.0".to_owned(),
            params: legacy_values
                .into_iter()
                .map(|(id, value)| (id.to_owned(), nice_plug::plugin::ParamValue::F32(value)))
                .collect(),
            fields: [
                ("response".to_owned(), encoded.clone()),
                (
                    "preset".to_owned(),
                    nice_plug::params::persist::serialize_field(&identity).unwrap(),
                ),
            ]
            .into_iter()
            .collect(),
        };
        MxmFxConvolution::filter_state(&mut state);
        assert_eq!(
            state.params.keys().map(String::as_str).collect::<Vec<_>>(),
            ["feedback", "mix", "predelay", "predelaysync", "wetgain"]
        );
        assert!(matches!(
            state.params["feedback"],
            nice_plug::plugin::ParamValue::F32(value) if value == 0.0
        ));
        assert_ne!(state.fields["response"], encoded);
        let restored_response = nice_plug::params::persist::deserialize_field::<
            response::ResponseState,
        >(&state.fields["response"])
        .unwrap();
        assert_eq!(restored_response.preparation.decay_percent, 100);
        assert_eq!(restored_response.preparation.damping_percent, 0);
        assert_eq!(restored_response.interpretation, response.interpretation);
        assert_eq!(restored_response.channels, response.channels);
        let restored_identity = nice_plug::params::persist::deserialize_field::<
            mxm_preset::PresetIdentity,
        >(&state.fields["preset"])
        .unwrap();
        assert_eq!(restored_identity.baseline.len(), 9);
        assert!(!restored_identity.baseline.contains_key("wetgain"));
        assert_eq!(restored_identity.baseline["highcut"], 1.0);
        assert_eq!(restored_identity.baseline["feedback"], 0.0);
        assert_eq!(restored_identity.baseline["predelaysync"], 0.0);
        assert_eq!(
            restored_identity.state_fingerprint,
            Some(response::fingerprint(&restored_response))
        );

        let restored_params = MxmFxConvolutionParams::default();
        set(&restored_params.mix, legacy_values[0].1);
        set(&restored_params.pre_delay, legacy_values[2].1);
        nice_plug::params::persist::PersistentField::set(
            &restored_params.response,
            restored_response,
        );
        nice_plug::params::persist::PersistentField::set(
            &restored_params.preset,
            restored_identity,
        );
        assert!(matches!(
            mxm_preset::loaded(&restored_params),
            mxm_preset::Loaded::Clean { name, .. } if name == legacy_name
        ));
    }

    #[test]
    fn bottom_open_high_cut_state_moves_to_the_top_without_dirtying_identity() {
        let response = response::ResponseState::default();
        let params = MxmFxConvolutionParams::default();
        let mut baseline: std::collections::BTreeMap<String, f32> =
            mxm_preset::Instrument::parameters(&params)
                .into_iter()
                .map(|(id, parameter)| (id.to_owned(), parameter.default_normalised()))
                .collect();
        baseline.insert("highcut".to_owned(), 0.0);
        let identity = mxm_preset::PresetIdentity {
            version: 1,
            loaded: Some(mxm_preset::LoadedPreset {
                name: "Bottom-open room".to_owned(),
                origin: mxm_preset::Origin::User,
            }),
            baseline,
            state_fingerprint: Some(response::fingerprint(&response)),
        };
        let mut state = PluginState {
            version: "0.1.0".to_owned(),
            params: [(
                "highcut".to_owned(),
                nice_plug::plugin::ParamValue::F32(0.0),
            )]
            .into_iter()
            .collect(),
            fields: [
                (
                    "response".to_owned(),
                    nice_plug::params::persist::serialize_field(&response).unwrap(),
                ),
                (
                    "preset".to_owned(),
                    nice_plug::params::persist::serialize_field(&identity).unwrap(),
                ),
            ]
            .into_iter()
            .collect(),
        };

        MxmFxConvolution::filter_state(&mut state);

        assert!(matches!(
            state.params["highcut"],
            nice_plug::plugin::ParamValue::F32(value) if value == HIGH_CUT_OPEN_HZ
        ));
        let migrated = nice_plug::params::persist::deserialize_field::<mxm_preset::PresetIdentity>(
            &state.fields["preset"],
        )
        .unwrap();
        assert_eq!(migrated.baseline["highcut"], 1.0);

        let mut current_state = state.clone();
        MxmFxConvolution::filter_state(&mut current_state);
        let current = nice_plug::params::persist::deserialize_field::<mxm_preset::PresetIdentity>(
            &current_state.fields["preset"],
        )
        .unwrap();
        assert_eq!(current.baseline["highcut"], 1.0);

        let mut closed_state = state;
        closed_state.version = "0.1.1".to_owned();
        closed_state.params.insert(
            "highcut".to_owned(),
            nice_plug::plugin::ParamValue::F32(0.0),
        );
        let mut closed_identity = migrated;
        closed_identity.baseline.insert("highcut".to_owned(), 0.0);
        closed_state.fields.insert(
            "preset".to_owned(),
            nice_plug::params::persist::serialize_field(&closed_identity).unwrap(),
        );
        MxmFxConvolution::filter_state(&mut closed_state);
        assert!(matches!(
            closed_state.params["highcut"],
            nice_plug::plugin::ParamValue::F32(value) if value == 0.0
        ));
        let closed = nice_plug::params::persist::deserialize_field::<mxm_preset::PresetIdentity>(
            &closed_state.fields["preset"],
        )
        .unwrap();
        assert_eq!(closed.baseline["highcut"], 0.0);
    }

    #[test]
    fn legacy_identity_migration_does_not_launder_a_modified_response() {
        let response = response::ResponseState::default();
        let params = MxmFxConvolutionParams::default();
        let legacy_name = "Modified Revision 5 room";
        let identity = mxm_preset::PresetIdentity {
            version: 1,
            loaded: Some(mxm_preset::LoadedPreset {
                name: legacy_name.to_owned(),
                origin: mxm_preset::Origin::Factory,
            }),
            baseline: [
                (
                    "mix".to_owned(),
                    params
                        .mix
                        .preview_normalized(params.mix.default_plain_value()),
                ),
                // Stored before Wet gain was deleted.
                ("wetgain".to_owned(), 0.57),
                (
                    "predelay".to_owned(),
                    params
                        .pre_delay
                        .preview_normalized(params.pre_delay.default_plain_value()),
                ),
            ]
            .into_iter()
            .collect(),
            state_fingerprint: Some(response::revision_five_fingerprint(&response) ^ 1),
        };
        let mut response_json = serde_json::to_value(&response).unwrap();
        response_json["preparation"]
            .as_object_mut()
            .unwrap()
            .remove("decay_percent");
        response_json["preparation"]
            .as_object_mut()
            .unwrap()
            .remove("damping_percent");
        let mut state = PluginState {
            version: "0.1.0".to_owned(),
            params: Default::default(),
            fields: [
                (
                    "response".to_owned(),
                    serde_json::to_string(&response_json).unwrap(),
                ),
                (
                    "preset".to_owned(),
                    nice_plug::params::persist::serialize_field(&identity).unwrap(),
                ),
            ]
            .into_iter()
            .collect(),
        };
        MxmFxConvolution::filter_state(&mut state);
        let restored_identity = nice_plug::params::persist::deserialize_field::<
            mxm_preset::PresetIdentity,
        >(&state.fields["preset"])
        .unwrap();
        // Unchanged but for the deleted Wet gain, which every restore drops.
        let mut expected = identity.clone();
        expected.baseline.remove("wetgain");
        assert_eq!(restored_identity, expected);
        let restored_response = nice_plug::params::persist::deserialize_field::<
            response::ResponseState,
        >(&state.fields["response"])
        .unwrap();
        nice_plug::params::persist::PersistentField::set(&params.response, restored_response);
        nice_plug::params::persist::PersistentField::set(&params.preset, restored_identity);
        assert!(matches!(
            mxm_preset::loaded(&params),
            mxm_preset::Loaded::Modified { name, .. } if name == legacy_name
        ));
    }

    #[test]
    fn malformed_response_state_is_a_complete_no_op() {
        let mut state = PluginState {
            version: "0.1.0".to_owned(),
            params: [("mix".to_owned(), nice_plug::plugin::ParamValue::F32(0.9))]
                .into_iter()
                .collect(),
            fields: [("response".to_owned(), "{not response json".to_owned())]
                .into_iter()
                .collect(),
        };
        MxmFxConvolution::filter_state(&mut state);
        assert!(state.params.is_empty());
        assert!(state.fields.is_empty());
    }

    #[test]
    fn hostile_input_recovers_to_finite_output_and_latches_fault() {
        let mut plugin = prepared(2);
        let telemetry = plugin.telemetry();
        let mut left = vec![f32::NAN, f32::INFINITY, f32::MAX];
        let mut right = vec![0.5, -0.5, f32::MAX];
        plugin.process_slices(&mut [&mut left, &mut right]);
        assert!(left.iter().chain(&right).all(|sample| sample.is_finite()));
        assert!(telemetry.numeric_fault());
    }

    #[test]
    fn positive_out_of_range_rates_activate_with_visible_inert_response_and_exact_dry() {
        let mut plugin = prepared(2);
        let canonical = plugin.params.response.snapshot();
        let telemetry = plugin.telemetry();
        for rate in [1_000.0, 1_234.57, 768_000.0] {
            assert!(plugin.prepare_for_test(rate, 2), "activation at {rate} Hz");
            assert!(!plugin.wet_available);
            assert_eq!(plugin.params.response.snapshot(), canonical);
            let rejection = telemetry.response_rejection().expect("visible rejection");
            assert_eq!(
                rejection,
                telemetry::ResponseRejection::UnsupportedRate(rate)
            );
            assert!(rejection.message().contains("runs from 8 to 384 kHz"));

            let mut left = vec![0.25, f32::NAN, f32::from_bits(1)];
            let mut right = vec![-0.125, 0.5, -f32::from_bits(1)];
            assert_eq!(
                plugin.process_slices(&mut [&mut left, &mut right]),
                ProcessStatus::Normal
            );
            assert_eq!(left, [0.25, 0.0, 0.0]);
            assert_eq!(right, [-0.125, 0.5, 0.0]);
            assert!(
                left.iter()
                    .chain(&right)
                    .all(|sample| sample.is_finite() && !sample.is_subnormal())
            );
        }

        assert!(plugin.prepare_for_test(48_000.0, 2));
        assert!(plugin.wet_available);
        assert_eq!(telemetry.response_rejection(), None);
        assert_eq!(plugin.params.response.snapshot(), canonical);
    }

    #[test]
    fn over_budget_response_keeps_canonical_source_and_retries_at_a_supported_rate() {
        let mut plugin = prepared(2);
        let mut state = plugin.params.response.snapshot();
        state.preparation.time_percent = 400;
        nice_plug::params::persist::PersistentField::set(&plugin.params.response, state);
        assert!(plugin.prepare_for_test(48_000.0, 2));
        let canonical = plugin.params.response.snapshot();

        assert!(plugin.prepare_for_test(384_000.0, 2));
        assert!(!plugin.wet_available);
        assert_eq!(plugin.params.response.snapshot(), canonical);
        let rejection = plugin
            .telemetry
            .response_rejection()
            .expect("deadline rejection is visible");
        assert_eq!(rejection, telemetry::ResponseRejection::Deadline(384_000.0));
        assert!(rejection.message().contains("too long to run here"));

        let mut left = vec![0.25; 64];
        let mut right = vec![-0.125; 64];
        plugin.process_slices(&mut [&mut left, &mut right]);
        assert_eq!(left, vec![0.25; 64]);
        assert_eq!(right, vec![-0.125; 64]);

        assert!(plugin.prepare_for_test(96_000.0, 2));
        assert!(plugin.wet_available);
        assert_eq!(plugin.telemetry.response_rejection(), None);
        assert_eq!(plugin.params.response.snapshot(), canonical);
    }
}

/// **A reset re-resolves the tempo syncs**: a sync turned off while the host held the effect
/// unprocessed does not seed the reset from the previous division, and one still on stays on it.
#[cfg(test)]
mod reset_resolves_the_syncs {
    use super::*;

    #[test]
    fn a_reset_re_resolves_the_tempo_syncs() {
        let mut plugin = MxmFxConvolution {
            synced_pre_delay_s: Some(0.4),
            ..Default::default()
        };
        plugin.reset();
        assert_eq!(plugin.pre_delay_s(), plugin.params.pre_delay.value());
        plugin.telemetry.tempo.publish(Some(120.0));
        unsafe {
            let _ = plugin
                .params
                .pre_delay_sync
                .as_ptr()
                ._internal_set_normalized_value(1.0);
        }
        plugin.reset();
        assert_eq!(
            Some(plugin.pre_delay_s()),
            plugin.params.synced_pre_delay(Some(120.0))
        );
    }

    /// **Reactivation forgets the old tempo**: nice-plug resets right after activating, and that
    /// reset must not resolve from the tempo the host reported before it was deactivated — the first
    /// callback's tempo is the first one used.
    #[test]
    fn reactivation_forgets_the_previous_tempo() {
        let mut plugin = MxmFxConvolution::default();
        plugin.telemetry.tempo.publish(Some(120.0));
        unsafe {
            let _ = plugin
                .params
                .pre_delay_sync
                .as_ptr()
                ._internal_set_normalized_value(1.0);
        }
        assert!(plugin.prepare(48_000.0, 2));
        plugin.reset();
        assert_eq!(plugin.synced_pre_delay_s, None);
    }
}

/// What a player reads — on hover in the editor, and in a host's plugin browser — speaks to the
/// player about the sound, never about the machine or the code (`mxm_plugin_test::hover_text`).
#[cfg(test)]
mod speaks_to_the_player {
    #[test]
    fn hover_text() {
        mxm_plugin_test::hover_text::speaks_to_the_player(env!("CARGO_MANIFEST_DIR"));
    }

    #[test]
    fn host_description() {
        mxm_plugin_test::hover_text::host_description_speaks_to_the_player(env!(
            "CARGO_MANIFEST_DIR"
        ));
    }
}
