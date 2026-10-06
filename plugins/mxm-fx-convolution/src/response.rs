//! Canonical WAV response state, reversible preparation, and failure-atomic editor publication.
//!
//! Source samples remain immutable and path-free. WAV decoding, model transforms, band-limited
//! sample-rate conversion and FIR preparation all run away from `process()`. Editor and preset
//! changes stage a complete state and publish it through nice-plug's state transaction, which
//! reactivates the plugin under the wrapper's control-thread lock.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mxm_fx_convolution_dsp::{
    AudioEngine, LiveControls, PrepareError, PreparedResponse, ResponseInterpretation,
};
use nice_plug::context::gui::GuiContext;
use nice_plug::params::persist::{PersistentField, serialize_field};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

pub const STATE_SCHEMA: u32 = 1;
pub const OVERLAY_SCHEMA: u32 = 1;
pub const MAX_SOURCE_RATE: u32 = 384_000;
pub const MAX_SOURCE_SECONDS: f32 = 10.0;
pub const FULL_DURATION_MAX_RATE: u32 = 96_000;
const MIN_RATE: u32 = 8_000;
const IMPORT_FADE_SECONDS: f32 = 0.050;
const STARTER_RATE: u32 = 48_000;
const STARTER_SECONDS: f32 = 0.007;
const MAX_NAME_BYTES: usize = 256;
const HIGH_RATE_BUDGET_SAMPLES: usize = 4_096;
const RESAMPLER_LOBES: usize = 16;
/// Bins across the fixed ten-second response display: 50 ms each.
pub const DISPLAY_BINS: usize = 200;
/// The display's level floor. 72 dB carries a normalised room tail down into its noise floor.
pub const DISPLAY_FLOOR_DB: f32 = -72.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Interpretation {
    Mono,
    MonoToStereo,
    DiagonalStereo,
}

impl Interpretation {
    pub const fn dsp(self) -> ResponseInterpretation {
        match self {
            Self::Mono => ResponseInterpretation::Mono,
            Self::MonoToStereo => ResponseInterpretation::MonoToStereo,
            Self::DiagonalStereo => ResponseInterpretation::DiagonalStereo,
        }
    }

    pub const fn channels(self) -> usize {
        self.dsp().source_channels()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TailShape {
    #[default]
    Natural,
    Fade,
    Swell,
    Gate,
}

/// Reversible, recipe-owned operations. Integer fields keep state equality and fingerprints exact.
/// They are deliberately not host parameters because each edit rebuilds the prepared FIR.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preparation {
    /// First retained source frame.
    pub onset: u32,
    /// Retained source frames; zero means through the source end for old/default state.
    pub extent: u32,
    pub reverse: bool,
    /// Size: linked rate/time scaling, 25–400 percent. Spectrum moves with duration.
    pub time_percent: u16,
    pub tail_shape: TailShape,
    /// Tail-energy envelope after the legacy Tail shape, 25–400 percent. One hundred is exact
    /// neutral. The default preserves schema-1 host states that predate this field.
    #[serde(default = "neutral_decay_percent")]
    pub decay_percent: u16,
    /// Progressive time-varying low-pass amount, 0–100 percent. Zero is exact neutral.
    #[serde(default)]
    pub damping_percent: u16,
}

impl Default for Preparation {
    fn default() -> Self {
        Self {
            onset: 0,
            extent: 0,
            reverse: false,
            time_percent: 100,
            tail_shape: TailShape::Natural,
            decay_percent: neutral_decay_percent(),
            damping_percent: 0,
        }
    }
}

const fn neutral_decay_percent() -> u16 {
    100
}

/// The explicit operation carried by a deferred recipe/Init overlay.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayOperation {
    MergeWithCommittedSource,
}

/// Recipe-owned response model that deliberately carries no source coefficients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparationOverlay {
    pub schema: u32,
    pub operation: OverlayOperation,
    pub preparation: Preparation,
}

impl PreparationOverlay {
    pub fn from_model(model: ResponseModel) -> Self {
        Self {
            schema: OVERLAY_SCHEMA,
            operation: OverlayOperation::MergeWithCommittedSource,
            preparation: model.preparation,
        }
    }

    pub fn init() -> Self {
        Self {
            schema: OVERLAY_SCHEMA,
            operation: OverlayOperation::MergeWithCommittedSource,
            preparation: Preparation::default(),
        }
    }
}

/// Versioned, path-independent source truth stored in host state and user presets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseState {
    pub schema: u32,
    pub name: String,
    pub sample_rate: u32,
    pub frames: u32,
    pub interpretation: Interpretation,
    /// One base64 string per source channel, each containing little-endian finite `f32` samples.
    pub channels: Vec<String>,
    #[serde(default)]
    pub preparation: Preparation,
}

impl Default for ResponseState {
    fn default() -> Self {
        starter_state()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Idle,
    Loading(String),
    Ready(String),
    /// A successful publication whose requested duration was reduced to the measured budget.
    Information(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseModel {
    pub interpretation: Interpretation,
    pub preparation: Preparation,
}

/// Bounded paint model. The persisted base64 payload never enters a paint-frame clone or decode.
#[derive(Debug, Clone)]
pub struct ResponseDisplay {
    pub name: String,
    pub sample_rate: u32,
    pub frames: u32,
    pub channel_count: usize,
    pub model: ResponseModel,
    /// Per-bin `[peak, rms]` level in dB over the fixed ten-second axis, floored at
    /// [`DISPLAY_FLOOR_DB`]; bins after the response sit at the floor.
    pub energy: [[f32; 2]; DISPLAY_BINS],
}

struct PreparedCandidate {
    state: ResponseState,
    rate_bits: u32,
    fingerprint: u64,
    engine: AudioEngine,
    display: ResponseDisplay,
    adjustment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BudgetAxis {
    Size,
    Extent,
    Onset,
}

/// Persistent source truth plus a complete off-audio engine candidate.
///
/// A staged candidate retains its prepared FFT histories and pre-delay allocation. nice-plug's
/// failure-atomic state transaction calls `set()`, which consumes that exact candidate; activation
/// then takes the engine without repeating decode, FFT work, or allocation.
pub struct ResponseField {
    committed: RwLock<ResponseState>,
    display: RwLock<ResponseDisplay>,
    staged: Mutex<Option<PreparedCandidate>>,
    published_engine: Mutex<Option<PreparedCandidate>>,
    status: RwLock<LoadState>,
    context: RwLock<Option<GuiContext>>,
    processing_rate_bits: AtomicU32,
    revision: AtomicU64,
    latest_request: AtomicU64,
    /// Advances only for an external persistent-field write. Ordinary source/model requests share
    /// `latest_request` but must not invalidate an already-published preset's delayed identity.
    host_restore_generation: AtomicU64,
    /// True only around this field's own synchronous host-state publication. An unrelated host
    /// restore must invalidate staged editor work; the publication carrying that work must not.
    publishing_staged: AtomicBool,
    preparation_rejected: AtomicBool,
    display_builds: AtomicU64,
    #[cfg(test)]
    reject_next_preparation: AtomicBool,
}

impl Default for ResponseField {
    fn default() -> Self {
        let committed = ResponseState::default();
        let display = make_display(&committed).expect("bounded starter display");
        Self {
            committed: RwLock::new(committed),
            display: RwLock::new(display),
            staged: Mutex::new(None),
            published_engine: Mutex::new(None),
            status: RwLock::new(LoadState::Idle),
            context: RwLock::new(None),
            processing_rate_bits: AtomicU32::new(48_000.0f32.to_bits()),
            revision: AtomicU64::new(0),
            latest_request: AtomicU64::new(0),
            host_restore_generation: AtomicU64::new(0),
            publishing_staged: AtomicBool::new(false),
            preparation_rejected: AtomicBool::new(false),
            display_builds: AtomicU64::new(1),
            #[cfg(test)]
            reject_next_preparation: AtomicBool::new(false),
        }
    }
}

impl ResponseField {
    pub fn snapshot(&self) -> ResponseState {
        self.committed
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn map<R>(&self, f: impl FnOnce(&ResponseState) -> R) -> R {
        f(&self
            .committed
            .read()
            .unwrap_or_else(|error| error.into_inner()))
    }

    pub fn display(&self) -> ResponseDisplay {
        self.display
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn display_builds_for_test(&self) -> u64 {
        self.display_builds.load(Ordering::Acquire)
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// Whether this request still owns response publication ordering.
    pub fn request_is_current(&self, request: u64) -> bool {
        self.latest_request.load(Ordering::Acquire) == request
    }

    /// Generation of external host-state restores, excluding this field's own staged publication.
    pub fn host_restore_generation(&self) -> u64 {
        self.host_restore_generation.load(Ordering::Acquire)
    }

    pub fn status(&self) -> LoadState {
        self.status
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn set_processing_rate(&self, rate: f32) {
        self.processing_rate_bits
            .store(rate.to_bits(), Ordering::Release);
    }

    pub fn connect_context(&self, context: Option<GuiContext>) {
        *self
            .context
            .write()
            .unwrap_or_else(|error| error.into_inner()) = context;
    }

    pub fn set_loading(&self, path: &Path) -> u64 {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("response.wav")
            .to_owned();
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Loading(name);
        self.revision.fetch_add(1, Ordering::Release);
        self.latest_request.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn begin_edit(&self, name: &str) -> u64 {
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Loading(name.to_owned());
        self.revision.fetch_add(1, Ordering::Release);
        self.latest_request.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn fail(&self, message: impl Into<String>) {
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Failed(message.into());
        self.revision.fetch_add(1, Ordering::Release);
    }

    pub fn fail_request(&self, request: u64, message: impl Into<String>) {
        if self.latest_request.load(Ordering::Acquire) == request {
            self.fail(message);
        }
    }

    /// Decode and prepare a complete engine on the background worker without altering durable state.
    pub fn import_wav(&self, request: u64, path: &Path) -> Result<(), String> {
        let candidate = decode_wav(path)?;
        if !self.stage_request(request, candidate, None)? {
            return Ok(());
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("response.wav")
            .to_owned();
        self.mark_ready(name);
        Ok(())
    }

    pub fn prepare_edit(&self, request: u64, model: ResponseModel) -> Result<(), String> {
        let mut candidate = self.snapshot();
        let axis = edited_budget_axis(&candidate, model);
        candidate.interpretation = model.interpretation;
        candidate.preparation = model.preparation;
        let name = candidate.name.clone();
        if !self.stage_request(request, candidate, axis)? {
            return Ok(());
        }
        self.mark_ready(name);
        Ok(())
    }

    /// Prepare a deferred preset's complete source without exposing Ready to the ordinary response
    /// editor path. The transaction owns publication and identity ordering.
    pub fn prepare_deferred_state(
        &self,
        request: u64,
        state: ResponseState,
    ) -> Result<bool, String> {
        self.stage_request(request, state, None)
    }

    /// Prepare a source-preserving recipe or Init overlay for a deferred transaction.
    pub fn prepare_deferred_overlay(
        &self,
        request: u64,
        overlay: &PreparationOverlay,
    ) -> Result<bool, String> {
        if overlay.schema != OVERLAY_SCHEMA {
            return Err("unsupported response-model overlay schema".to_owned());
        }
        let mut candidate = self.snapshot();
        candidate.preparation = overlay.preparation;
        self.stage_request(request, candidate, None)
    }

    /// Invalidate one still-preparing request without allowing a late worker to stage it.
    pub fn cancel_request(&self, request: u64) {
        if self
            .latest_request
            .compare_exchange(
                request,
                request.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.staged
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            *self
                .status
                .write()
                .unwrap_or_else(|error| error.into_inner()) = LoadState::Idle;
            self.revision.fetch_add(1, Ordering::Release);
        }
    }

    fn mark_ready(&self, name: String) {
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Ready(name);
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn prepare_candidate(
        &self,
        state: ResponseState,
        rate: f32,
        axis: Option<BudgetAxis>,
    ) -> Result<PreparedCandidate, StateError> {
        #[cfg(test)]
        if self.reject_next_preparation.swap(false, Ordering::AcqRel) {
            return Err(StateError::Allocation);
        }
        prepare_candidate(state, rate, axis)
    }

    #[cfg(test)]
    fn reject_next_preparation_for_test(&self) {
        self.reject_next_preparation.store(true, Ordering::Release);
    }

    fn stage_request(
        &self,
        request: u64,
        state: ResponseState,
        axis: Option<BudgetAxis>,
    ) -> Result<bool, String> {
        let rate = f32::from_bits(self.processing_rate_bits.load(Ordering::Acquire));
        let candidate = self
            .prepare_candidate(state, rate, axis)
            .map_err(|error| preparation_message(error, rate))?;
        if self.latest_request.load(Ordering::Acquire) != request {
            return Ok(false);
        }
        *self
            .staged
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(candidate);
        Ok(true)
    }

    /// Stage a complete preset candidate before the shared seam emits parameter gestures.
    pub fn stage(&self, state: ResponseState) -> Result<(), String> {
        self.latest_request.fetch_add(1, Ordering::AcqRel);
        let rate = f32::from_bits(self.processing_rate_bits.load(Ordering::Acquire));
        let candidate = self
            .prepare_candidate(state, rate, None)
            .map_err(|error| preparation_message(error, rate))?;
        *self
            .staged
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(candidate);
        Ok(())
    }

    /// Merge recipe-owned preparation with the immutable committed source, then stage the complete
    /// candidate. The overlay cannot replace coefficients, interpretation, provenance, or bounds.
    pub fn stage_overlay(&self, overlay: &PreparationOverlay) -> Result<(), String> {
        if overlay.schema != OVERLAY_SCHEMA {
            return Err("unsupported response-model overlay schema".to_owned());
        }
        let mut candidate = self.snapshot();
        candidate.preparation = overlay.preparation;
        self.stage(candidate)
    }

    /// Publish through nice-plug's off-audio, rollback-capable state transaction. `set()` consumes
    /// the staged engine, and `Plugin::activate()` takes it without any second preparation.
    pub fn commit(&self) -> Result<bool, String> {
        let encoded = {
            let staged = self
                .staged
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(candidate) = staged.as_ref() else {
                return Ok(false);
            };
            serialize_field(&candidate.state)
                .map_err(|error| format!("response state encode failed: {error}"))?
        };
        let context = self
            .context
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        self.publishing_staged.store(true, Ordering::Release);
        if let Some(context) = context {
            let mut state = context.get_state();
            state.fields.insert("response".to_owned(), encoded);
            context.set_state(state);
        } else {
            let state = self
                .staged
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .expect("staged candidate remains present")
                .state
                .clone();
            self.set(state);
        }
        self.publishing_staged.store(false, Ordering::Release);
        if self.preparation_rejected.load(Ordering::Acquire) {
            return Err("response publication failed; the previous patch was restored".to_owned());
        }
        Ok(true)
    }

    pub fn take_prepared_engine(&self, fingerprint: u64, rate: f32) -> Option<AudioEngine> {
        let mut slot = self
            .published_engine
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot.as_ref().is_some_and(|candidate| {
            candidate.fingerprint == fingerprint && candidate.rate_bits == rate.to_bits()
        }) {
            slot.take().map(|candidate| candidate.engine)
        } else {
            None
        }
    }

    pub fn take_preparation_rejected(&self) -> bool {
        self.preparation_rejected.swap(false, Ordering::AcqRel)
    }

    fn install(&self, candidate: PreparedCandidate) {
        let adjustment = candidate.adjustment.clone();
        *self
            .committed
            .write()
            .unwrap_or_else(|error| error.into_inner()) = candidate.state.clone();
        *self
            .display
            .write()
            .unwrap_or_else(|error| error.into_inner()) = candidate.display.clone();
        *self
            .published_engine
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(candidate);
        self.display_builds.fetch_add(1, Ordering::Release);
        self.preparation_rejected.store(false, Ordering::Release);
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = adjustment
            .map(LoadState::Information)
            .unwrap_or(LoadState::Idle);
        self.revision.fetch_add(1, Ordering::Release);
    }
}

impl<'a> PersistentField<'a, ResponseState> for ResponseField {
    fn set(&self, new_value: ResponseState) {
        let rate = f32::from_bits(self.processing_rate_bits.load(Ordering::Acquire));
        let internal_publication = self.publishing_staged.load(Ordering::Acquire);

        if internal_publication {
            // The staged candidate is the response half of our own state transaction. Consume it
            // without advancing the request generation that made it eligible.
            let candidate = {
                let mut staged = self
                    .staged
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if staged.as_ref().is_some_and(|candidate| {
                    candidate.state == new_value && candidate.rate_bits == rate.to_bits()
                }) {
                    staged.take()
                } else {
                    None
                }
            };
            if let Some(candidate) = candidate {
                self.install(candidate);
                return;
            }
        } else {
            // PersistentField::set outside commit() is a newer host restore. It participates in the
            // same request generation as editor WAV/model/preset work and in a separate restore-only
            // generation, even when the restored response is value-equal to the current one.
            self.host_restore_generation.fetch_add(1, Ordering::AcqRel);
            self.latest_request.fetch_add(1, Ordering::AcqRel);
            self.staged
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            *self
                .status
                .write()
                .unwrap_or_else(|error| error.into_inner()) = LoadState::Idle;
            self.revision.fetch_add(1, Ordering::Release);
        }

        if self.map(|current| current == &new_value) {
            self.preparation_rejected.store(false, Ordering::Release);
            *self
                .status
                .write()
                .unwrap_or_else(|error| error.into_inner()) = LoadState::Idle;
            return;
        }
        match self.prepare_candidate(new_value.clone(), rate, None) {
            Ok(candidate) if candidate.state != new_value => {
                self.install(candidate);
                // nice-plug's defect-9 guard normally interprets any changed serialized field as a
                // rejection. This field accepted the complete response and installed its visible,
                // prepared canonical model, so acknowledge only this stable key in the wrapper's
                // scoped restore transaction. Outside wrapper restoration this call is a no-op.
                nice_plug::wrapper::accept_canonicalized_persistent_field("response");
            }
            Ok(candidate) => self.install(candidate),
            Err(error) => {
                self.preparation_rejected.store(true, Ordering::Release);
                self.fail(format!(
                    "{}; the previous patch was kept",
                    preparation_message(error, rate)
                ));
            }
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&ResponseState) -> R,
    {
        f(&self
            .committed
            .read()
            .unwrap_or_else(|error| error.into_inner()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    Schema,
    SampleRate,
    NameTooLong,
    Empty,
    TooLong,
    ChannelCount,
    EncodedLength,
    InvalidBase64,
    NonFinite,
    Preparation,
    Deadline,
    Allocation,
    WetPost,
    Prepared(PrepareError),
}

pub fn validate(state: &ResponseState) -> Result<(), StateError> {
    decode(state).map(|_| ())
}

/// Stable fingerprint of source and reversible preparation. Provenance text is deliberately absent.
pub fn fingerprint(state: &ResponseState) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    add(&state.schema.to_le_bytes());
    add(&state.sample_rate.to_le_bytes());
    add(&state.frames.to_le_bytes());
    add(&[match state.interpretation {
        Interpretation::Mono => 0,
        Interpretation::MonoToStereo => 1,
        Interpretation::DiagonalStereo => 2,
    }]);
    add(&state.preparation.onset.to_le_bytes());
    add(&state.preparation.extent.to_le_bytes());
    add(&[u8::from(state.preparation.reverse)]);
    add(&state.preparation.time_percent.to_le_bytes());
    add(&[state.preparation.tail_shape as u8]);
    add(&state.preparation.decay_percent.to_le_bytes());
    add(&state.preparation.damping_percent.to_le_bytes());
    for channel in &state.channels {
        add(channel.as_bytes());
        add(&[0xff]);
    }
    hash
}

/// Revision 5's exact identity fingerprint, before neutral Decay and Damping joined the response
/// model. State migration uses it only to distinguish a clean legacy identity from one whose
/// response had already changed after its preset was loaded.
pub(crate) fn revision_five_fingerprint(state: &ResponseState) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    add(&state.schema.to_le_bytes());
    add(&state.sample_rate.to_le_bytes());
    add(&state.frames.to_le_bytes());
    add(&[match state.interpretation {
        Interpretation::Mono => 0,
        Interpretation::MonoToStereo => 1,
        Interpretation::DiagonalStereo => 2,
    }]);
    add(&state.preparation.onset.to_le_bytes());
    add(&state.preparation.extent.to_le_bytes());
    add(&[u8::from(state.preparation.reverse)]);
    add(&state.preparation.time_percent.to_le_bytes());
    add(&[state.preparation.tail_shape as u8]);
    for channel in &state.channels {
        add(channel.as_bytes());
        add(&[0xff]);
    }
    hash
}

fn edited_budget_axis(current: &ResponseState, requested: ResponseModel) -> Option<BudgetAxis> {
    let current_model = ResponseModel {
        interpretation: current.interpretation,
        preparation: current.preparation,
    };

    let mut without = requested;
    without.preparation.time_percent = current.preparation.time_percent;
    if without == current_model {
        return Some(BudgetAxis::Size);
    }
    let mut without = requested;
    without.preparation.extent = current.preparation.extent;
    if without == current_model {
        return Some(BudgetAxis::Extent);
    }
    let mut without = requested;
    without.preparation.onset = current.preparation.onset;
    if requested.preparation.extent == 0 && without == current_model {
        return Some(BudgetAxis::Onset);
    }
    None
}

fn converted_sample_count(
    source_samples: usize,
    source_rate: f32,
    target_rate: f32,
) -> Option<usize> {
    let duration = source_samples as f64 / source_rate as f64;
    let converted = duration * target_rate as f64;
    if !duration.is_finite()
        || !converted.is_finite()
        || converted < 0.0
        || converted > usize::MAX as f64
    {
        return None;
    }
    Some((converted.round() as usize).max(1))
}

fn prepared_sample_count(state: &ResponseState, processing_rate: f32) -> Result<usize, StateError> {
    let processing_rate = valid_processing_rate(processing_rate)?;
    let prep = normalized_preparation(state)?;
    let scaled_rate = state.sample_rate as f32 / (prep.time_percent as f32 / 100.0);
    converted_sample_count(prep.extent as usize, scaled_rate, processing_rate)
        .ok_or(StateError::EncodedLength)
}

fn fits_processing_budget(state: &ResponseState, rate: f32) -> Result<bool, StateError> {
    Ok(prepared_sample_count(state, rate)?
        <= maximum_processing_samples(rate).ok_or(StateError::SampleRate)?)
}

fn clamp_extent(state: &mut ResponseState, rate: f32) -> Result<Option<u32>, StateError> {
    let requested = normalized_preparation(state)?.extent;
    let mut low = 1u32;
    let mut high = requested;
    let mut best = None;
    while low <= high {
        let middle = low + (high - low) / 2;
        state.preparation.extent = middle;
        if fits_processing_budget(state, rate)? {
            best = Some(middle);
            low = middle.saturating_add(1);
        } else {
            high = middle.saturating_sub(1);
        }
    }
    if let Some(value) = best {
        state.preparation.extent = value;
    } else {
        state.preparation.extent = 1;
    }
    Ok(best)
}

fn clamp_onset(state: &mut ResponseState, rate: f32) -> Result<Option<u32>, StateError> {
    if state.preparation.extent != 0 {
        return Ok(None);
    }
    let requested = state.preparation.onset;
    let mut low = requested;
    let mut high = state.frames - 1;
    let mut best = None;
    while low <= high {
        let middle = low + (high - low) / 2;
        state.preparation.onset = middle;
        if fits_processing_budget(state, rate)? {
            best = Some(middle);
            high = middle.saturating_sub(1);
        } else {
            low = middle.saturating_add(1);
        }
    }
    if let Some(value) = best {
        state.preparation.onset = value;
    } else {
        state.preparation.onset = state.frames - 1;
    }
    Ok(best)
}

fn normalize_budget(
    mut state: ResponseState,
    rate: f32,
    direct_axis: Option<BudgetAxis>,
) -> Result<(ResponseState, Option<String>), StateError> {
    // Validate domains before searching. Above 96 kHz the fixed 4,096-sample envelope is a measured
    // architecture boundary, not an ordinary model edit, so it remains a visible refusal.
    let _ = normalized_preparation(&state)?;
    if fits_processing_budget(&state, rate)? {
        return Ok((state, None));
    }
    if rate > FULL_DURATION_MAX_RATE as f32 {
        return Err(StateError::Deadline);
    }

    let requested = state.preparation;
    let axes: &[BudgetAxis] = match direct_axis {
        Some(BudgetAxis::Size) => &[BudgetAxis::Size],
        Some(BudgetAxis::Extent) => &[BudgetAxis::Extent],
        Some(BudgetAxis::Onset) => &[BudgetAxis::Onset],
        None => &[BudgetAxis::Size, BudgetAxis::Extent, BudgetAxis::Onset],
    };
    let mut adjusted = Vec::new();
    for axis in axes {
        match axis {
            BudgetAxis::Size => {
                let mut fitting = None;
                for value in (25..=requested.time_percent).rev() {
                    state.preparation.time_percent = value;
                    if fits_processing_budget(&state, rate)? {
                        fitting = Some(value);
                        break;
                    }
                }
                if fitting.is_none() {
                    state.preparation.time_percent = 25;
                }
                if state.preparation.time_percent != requested.time_percent {
                    adjusted.push("Size");
                }
            }
            BudgetAxis::Extent => {
                let before = normalized_preparation(&state)?.extent;
                let _ = clamp_extent(&mut state, rate)?;
                if state.preparation.extent != before || requested.extent == 0 {
                    adjusted.push("Extent");
                }
            }
            BudgetAxis::Onset => {
                let before = state.preparation.onset;
                let _ = clamp_onset(&mut state, rate)?;
                if state.preparation.onset != before {
                    adjusted.push("Onset");
                }
            }
        }
        if fits_processing_budget(&state, rate)? {
            let fields = adjusted.join(" and ");
            return Ok((
                state,
                Some(format!(
                    "Too long to run here; {fields} set to the longest that fits"
                )),
            ));
        }
    }
    Err(StateError::Deadline)
}

/// Measured two-tier scheduling envelope. Every supported rate through 96 kHz receives ten full
/// seconds. Above 96 kHz only the 4,096-sample early tier is accepted: the large late-tier FFT
/// cannot meet the 384 kHz 64-sample deadline on the measured machine. Audio is rejected with an
/// explicit error, never truncated. The full-duration ceiling uses the conversion function's exact
/// duration-first rounding law so fractional supported rates cannot reject their final sample.
pub fn maximum_processing_samples(rate: f32) -> Option<usize> {
    let rate = valid_processing_rate(rate).ok()?;
    if rate <= FULL_DURATION_MAX_RATE as f32 {
        let canonical_samples = converted_sample_count(
            (MAX_SOURCE_RATE as f64 * MAX_SOURCE_SECONDS as f64).round() as usize,
            MAX_SOURCE_RATE as f32,
            rate,
        )?;
        Some(canonical_samples)
    } else {
        Some(HIGH_RATE_BUDGET_SAMPLES)
    }
}

fn preparation_message(error: StateError, rate: f32) -> String {
    if error == StateError::Deadline {
        let maximum = maximum_processing_samples(rate).unwrap_or_default();
        format!(
            "response exceeds the measured {:.3} s processing budget at {:.0} Hz",
            maximum as f64 / rate as f64,
            rate
        )
    } else {
        format!("response could not be prepared: {error:?}")
    }
}

fn prepare_candidate(
    state: ResponseState,
    rate: f32,
    axis: Option<BudgetAxis>,
) -> Result<PreparedCandidate, StateError> {
    let (state, adjustment) = normalize_budget(state, rate, axis)?;
    let prepared = prepare(&state, rate)?;
    let display = make_display(&state)?;
    let fingerprint = fingerprint(&state);
    let maximum_pre_delay =
        (crate::params::MAX_PRE_DELAY_SECONDS as f64 * rate as f64).round() as usize;
    let ramp_samples = (crate::CONTROL_RAMP_SECONDS as f64 * rate as f64)
        .round()
        .max(1.0) as u64;
    let mut engine = AudioEngine::try_new(
        prepared,
        maximum_pre_delay,
        LiveControls::default(),
        ramp_samples,
    )
    .map_err(|error| match error {
        PrepareError::Allocation => StateError::Allocation,
        other => StateError::Prepared(other),
    })?;
    engine
        .try_enable_wet_post(rate)
        .map_err(|_| StateError::WetPost)?;
    Ok(PreparedCandidate {
        state,
        rate_bits: rate.to_bits(),
        fingerprint,
        engine,
        display,
        adjustment,
    })
}

fn make_display(state: &ResponseState) -> Result<ResponseDisplay, StateError> {
    let energy = energy_profile(state)?;
    Ok(ResponseDisplay {
        name: state.name.clone(),
        sample_rate: state.sample_rate,
        frames: state.frames,
        channel_count: state.channels.len(),
        model: ResponseModel {
            interpretation: state.interpretation,
            preparation: state.preparation,
        },
        energy,
    })
}

/// Decode, apply the reversible model, band-limit sample-rate conversion, and build complete spectra.
pub fn prepare(
    state: &ResponseState,
    processing_rate: f32,
) -> Result<PreparedResponse, StateError> {
    let processing_rate = valid_processing_rate(processing_rate)?;
    let expected_samples = prepared_sample_count(state, processing_rate)?;
    if expected_samples
        > maximum_processing_samples(processing_rate).ok_or(StateError::SampleRate)?
    {
        return Err(StateError::Deadline);
    }
    let mut source = decode(state)?;
    let scale = loudness_scale(&source, state.sample_rate as f32, processing_rate);
    for sample in source.iter_mut().flatten() {
        *sample *= scale;
    }
    let prep = normalized_preparation(state)?;
    let mut channels = Vec::new();
    channels
        .try_reserve_exact(source.len())
        .map_err(|_| StateError::Allocation)?;
    for channel in &source {
        let shaped = shape(channel, prep, state.sample_rate as f32)?;
        let scaled_rate = state.sample_rate as f32 / (prep.time_percent as f32 / 100.0);
        channels.push(resample_bandlimited(&shaped, scaled_rate, processing_rate)?);
    }
    let response_samples = channels.first().map_or(0, Vec::len);
    debug_assert_eq!(response_samples, expected_samples);
    let mut refs = Vec::new();
    refs.try_reserve_exact(channels.len())
        .map_err(|_| StateError::Allocation)?;
    refs.extend(channels.iter().map(Vec::as_slice));
    // The coefficients above are already resampled to `processing_rate`, and the loop bound is
    // measured through the loop's own damping, whose corners are in Hz - so the rate has to travel
    // with them.
    PreparedResponse::from_channels(state.interpretation.dsp(), &refs, processing_rate).map_err(
        |error| match error {
            PrepareError::Allocation => StateError::Allocation,
            other => StateError::Prepared(other),
        },
    )
}

/// Below this mean energy a response is silence, and is left as it is rather than raised by an
/// unbounded gain: 120 dB under a full-scale impulse.
const SILENT_ENERGY: f64 = 1.0e-12;

/// **Every response at one loudness** (the owner, 2026-09-28: *normalise the impulses*, with Mix the
/// one level): the gain that gives the whole source a mean energy of one per channel **at the
/// processing rate**, so the reverb of broadband sound is as loud as the sound itself, whichever
/// room is loaded. Energy, not peak: a response's peak is usually its direct click, and two rooms
/// with the same click can differ by twenty decibels in their tails.
///
/// The resampler keeps DC gain, so energy moves with the rate ratio — a response prepared at twice
/// its source rate has half its source energy. Normalising against that puts the same room recorded
/// at 48 and at 96 kHz at one level. Measured at Size 100 % and over the whole source, before any
/// preparation: Size, Onset, Extent, Decay and Damping keep the effect on level they always had.
/// One gain for every channel, so no left/right or matrix ratio moves.
fn loudness_scale(source: &[Vec<f32>], source_rate: f32, processing_rate: f32) -> f32 {
    let channels = source.len().max(1) as f64;
    let energy = source
        .iter()
        .flatten()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>()
        / channels;
    let at_processing = energy * f64::from(source_rate) / f64::from(processing_rate);
    if at_processing.is_finite() && at_processing > SILENT_ENERGY {
        (1.0 / at_processing.sqrt()) as f32
    } else {
        1.0
    }
}

fn valid_processing_rate(rate: f32) -> Result<f32, StateError> {
    if rate.is_finite() && (MIN_RATE as f32..=MAX_SOURCE_RATE as f32).contains(&rate) {
        Ok(rate)
    } else {
        Err(StateError::SampleRate)
    }
}

fn normalized_preparation(state: &ResponseState) -> Result<Preparation, StateError> {
    let mut prep = state.preparation;
    if !(25..=400).contains(&prep.time_percent)
        || !(25..=400).contains(&prep.decay_percent)
        || prep.damping_percent > 100
        || prep.onset >= state.frames
    {
        return Err(StateError::Preparation);
    }
    let available = state.frames - prep.onset;
    prep.extent = if prep.extent == 0 {
        available
    } else {
        prep.extent.min(available)
    };
    if prep.extent == 0 {
        return Err(StateError::Preparation);
    }
    Ok(prep)
}

fn decode(state: &ResponseState) -> Result<Vec<Vec<f32>>, StateError> {
    if state.schema != STATE_SCHEMA {
        return Err(StateError::Schema);
    }
    if !(MIN_RATE..=MAX_SOURCE_RATE).contains(&state.sample_rate) {
        return Err(StateError::SampleRate);
    }
    if state.name.len() > MAX_NAME_BYTES {
        return Err(StateError::NameTooLong);
    }
    let frames = state.frames as usize;
    if frames == 0 {
        return Err(StateError::Empty);
    }
    let maximum = (state.sample_rate as f32 * MAX_SOURCE_SECONDS).round() as usize;
    if frames > maximum {
        return Err(StateError::TooLong);
    }
    if state.channels.len() != state.interpretation.channels() {
        return Err(StateError::ChannelCount);
    }
    normalized_preparation(state)?;

    let byte_len = frames.checked_mul(4).ok_or(StateError::EncodedLength)?;
    let encoded_len = byte_len
        .checked_add(2)
        .and_then(|length| length.checked_div(3))
        .and_then(|length| length.checked_mul(4))
        .ok_or(StateError::EncodedLength)?;
    let mut decoded = Vec::new();
    decoded
        .try_reserve_exact(state.channels.len())
        .map_err(|_| StateError::Allocation)?;
    for encoded in &state.channels {
        if encoded.len() != encoded_len {
            return Err(StateError::EncodedLength);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(byte_len)
            .map_err(|_| StateError::Allocation)?;
        BASE64
            .decode_vec(encoded.as_bytes(), &mut bytes)
            .map_err(|_| StateError::InvalidBase64)?;
        if bytes.len() != byte_len {
            return Err(StateError::EncodedLength);
        }
        let mut channel = Vec::new();
        channel
            .try_reserve_exact(frames)
            .map_err(|_| StateError::Allocation)?;
        let (words, remainder) = bytes.as_chunks::<4>();
        if !remainder.is_empty() {
            return Err(StateError::EncodedLength);
        }
        for bytes in words {
            let value = f32::from_le_bytes(*bytes);
            if !value.is_finite() {
                return Err(StateError::NonFinite);
            }
            channel.push(if value.abs() < f32::MIN_POSITIVE {
                0.0
            } else {
                value
            });
        }
        decoded.push(channel);
    }
    Ok(decoded)
}

/// Energy over time for the editor: each 50 ms bin's peak and RMS level in dB across every channel,
/// on a fixed ten-second axis, so responses compare by where their energy lies instead of each one
/// filling the plot. A linear trace showed only the normalised onset spike; the tail that makes a
/// room was a flat line in every file.
pub fn energy_profile(state: &ResponseState) -> Result<[[f32; 2]; DISPLAY_BINS], StateError> {
    let channels = decode(state)?;
    let axis_frames =
        ((state.sample_rate as f64 * MAX_SOURCE_SECONDS as f64).round() as usize).max(DISPLAY_BINS);
    let frames = state.frames as usize;
    let mut energy = [[DISPLAY_FLOOR_DB; 2]; DISPLAY_BINS];
    for (bin, level) in energy.iter_mut().enumerate() {
        let start = bin * axis_frames / DISPLAY_BINS;
        if start >= frames {
            break;
        }
        let end = ((bin + 1) * axis_frames / DISPLAY_BINS)
            .min(frames)
            .max(start + 1);
        let mut peak = 0.0f32;
        let mut sum = 0.0f64;
        for channel in &channels {
            for &sample in &channel[start..end] {
                peak = peak.max(sample.abs());
                sum += f64::from(sample) * f64::from(sample);
            }
        }
        let count = ((end - start) * channels.len()).max(1) as f64;
        *level = [display_db(peak), display_db((sum / count).sqrt() as f32)];
    }
    Ok(energy)
}

fn display_db(amplitude: f32) -> f32 {
    if amplitude > 0.0 {
        (20.0 * amplitude.log10()).max(DISPLAY_FLOOR_DB)
    } else {
        DISPLAY_FLOOR_DB
    }
}

fn shape(source: &[f32], prep: Preparation, sample_rate: f32) -> Result<Vec<f32>, StateError> {
    let start = prep.onset as usize;
    let length = prep.extent as usize;
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| StateError::Allocation)?;
    for index in 0..length {
        let source_index = if prep.reverse {
            start + length - index - 1
        } else {
            start + index
        };
        let phase = if length <= 1 {
            0.0
        } else {
            index as f32 / (length - 1) as f32
        };
        let envelope = match prep.tail_shape {
            TailShape::Natural => 1.0,
            TailShape::Fade => (1.0 - phase).powi(2),
            TailShape::Swell => phase.powi(2),
            TailShape::Gate => f32::from(phase < 0.75),
        };
        // Project-derived Decay envelope. Neutral is a separate branch so every shipped Tail law
        // remains bit-identical. Shortening attenuates toward the end; lengthening can only raise
        // energy already present because it remains multiplication of the immutable source.
        let decay = if prep.decay_percent == 100 {
            1.0
        } else if prep.decay_percent < 100 {
            let strength = (100 - prep.decay_percent) as f32 / 25.0;
            (1.0 - phase).powf(strength)
        } else {
            1.0 + phase * (prep.decay_percent - 100) as f32 / 100.0
        };
        let value = source[source_index] * envelope * decay;
        result.push(if value.abs() < f32::MIN_POSITIVE {
            0.0
        } else {
            value
        });
    }

    if prep.damping_percent != 0 {
        // Project-derived time-varying damping: a one-pole low pass whose cutoff falls over source
        // time. It operates only on selected finite support and cannot extend or invent a tail.
        let amount = prep.damping_percent as f32 / 100.0;
        let nyquist_ceiling = sample_rate * 0.45;
        let floor = 200.0f32.min(nyquist_ceiling);
        let mut state = 0.0f32;
        let denominator = result.len().saturating_sub(1).max(1) as f32;
        for (index, value) in result.iter_mut().enumerate() {
            let phase = index as f32 / denominator;
            let cutoff = (nyquist_ceiling * (1.0 - amount * phase * 0.98)).max(floor);
            let coefficient = 1.0 - (-core::f32::consts::TAU * cutoff / sample_rate).exp();
            state += coefficient * (*value - state);
            state = if state.abs() < f32::MIN_POSITIVE {
                0.0
            } else {
                state
            };
            *value = state;
        }
    }
    Ok(result)
}

/// Windowed-sinc sample-rate conversion. The cutoff follows the lower Nyquist limit; the kernel
/// expands for large decimation ratios so 384 kHz content above an 8 kHz target cannot fold into the
/// prepared FIR. Per-output normalization preserves DC and boundary amplitude without peak scaling.
fn resample_bandlimited(
    source: &[f32],
    source_rate: f32,
    target_rate: f32,
) -> Result<Vec<f32>, StateError> {
    if source_rate == target_rate {
        let mut copy = Vec::new();
        copy.try_reserve_exact(source.len())
            .map_err(|_| StateError::Allocation)?;
        copy.extend_from_slice(source);
        return Ok(copy);
    }
    let target_len = converted_sample_count(source.len(), source_rate, target_rate)
        .ok_or(StateError::EncodedLength)?;
    let step = source_rate as f64 / target_rate as f64;
    let cutoff = 0.5 * (target_rate as f64 / source_rate as f64).min(1.0);
    let radius = (RESAMPLER_LOBES as f64 * step.max(1.0)).ceil() as isize;
    let mut target = Vec::new();
    target
        .try_reserve_exact(target_len)
        .map_err(|_| StateError::Allocation)?;
    for index in 0..target_len {
        let position = index as f64 * step;
        let centre = position.floor() as isize;
        let mut sum = 0.0f64;
        let mut weight_sum = 0.0f64;
        for source_index in centre - radius + 1..=centre + radius {
            if !(0..source.len() as isize).contains(&source_index) {
                continue;
            }
            let distance = position - source_index as f64;
            let window_position = distance / radius as f64;
            if window_position.abs() > 1.0 {
                continue;
            }
            let window = 0.42
                + 0.5 * (core::f64::consts::PI * window_position).cos()
                + 0.08 * (2.0 * core::f64::consts::PI * window_position).cos();
            let argument = 2.0 * cutoff * distance;
            let sinc = if argument.abs() < 1.0e-12 {
                1.0
            } else {
                (core::f64::consts::PI * argument).sin() / (core::f64::consts::PI * argument)
            };
            let weight = 2.0 * cutoff * sinc * window;
            sum += source[source_index as usize] as f64 * weight;
            weight_sum += weight;
        }
        let value = if weight_sum.abs() > 1.0e-12 {
            (sum / weight_sum) as f32
        } else {
            0.0
        };
        target.push(if value.abs() < f32::MIN_POSITIVE {
            0.0
        } else {
            value
        });
    }
    Ok(target)
}

#[cfg(test)]
pub(crate) fn maximum_display_state_for_test() -> ResponseState {
    // The owner's stated bounded-state case: ten seconds of stereo at 48 kHz is about 5 MiB once
    // its little-endian samples are base64 encoded.
    let sample_rate = 48_000;
    let frames = (sample_rate as f32 * MAX_SOURCE_SECONDS) as usize;
    let zeros = vec![0.0; frames];
    let encoded = encode_channel(&zeros).expect("bounded maximum test channel encodes");
    ResponseState {
        schema: STATE_SCHEMA,
        name: "Maximum display source".to_owned(),
        sample_rate,
        frames: frames as u32,
        interpretation: Interpretation::MonoToStereo,
        channels: vec![encoded.clone(), encoded],
        preparation: Preparation {
            time_percent: 25,
            ..Preparation::default()
        },
    }
}

fn encode_channel(samples: &[f32]) -> Result<String, StateError> {
    let byte_len = samples
        .len()
        .checked_mul(4)
        .ok_or(StateError::EncodedLength)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(byte_len)
        .map_err(|_| StateError::Allocation)?;
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let encoded_len = bytes
        .len()
        .checked_add(2)
        .and_then(|length| length.checked_div(3))
        .and_then(|length| length.checked_mul(4))
        .ok_or(StateError::EncodedLength)?;
    let mut encoded = String::new();
    encoded
        .try_reserve_exact(encoded_len)
        .map_err(|_| StateError::Allocation)?;
    BASE64.encode_string(bytes, &mut encoded);
    Ok(encoded)
}

fn fade_import_boundary(samples: &mut [f32], sample_rate: u32) {
    if samples.len() < 2 {
        samples.fill(0.0);
        return;
    }
    let fade_frames =
        ((sample_rate as f32 * IMPORT_FADE_SECONDS).round() as usize).clamp(2, samples.len());
    let start = samples.len().saturating_sub(fade_frames);
    let denominator = fade_frames.saturating_sub(1).max(1) as f32;
    for (offset, sample) in samples[start..].iter_mut().enumerate() {
        let phase = offset as f32 / denominator;
        let gain = 0.5 * (1.0 + (core::f32::consts::PI * phase).cos());
        *sample = flush_source_sample(*sample * gain);
    }
}

fn flush_source_sample(value: f32) -> f32 {
    if value.abs() < f32::MIN_POSITIVE {
        0.0
    } else {
        value
    }
}

pub(crate) fn decode_wav(path: &Path) -> Result<ResponseState, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("wav") && !extension.eq_ignore_ascii_case("wave") {
        return Err("unsupported file type; choose a WAV file".to_owned());
    }
    let mut reader =
        hound::WavReader::open(path).map_err(|error| format!("could not open WAV: {error}"))?;
    let spec = reader.spec();
    if !(1..=2).contains(&spec.channels) {
        return Err("WAV must contain one or two channels".to_owned());
    }
    if !(MIN_RATE..=MAX_SOURCE_RATE).contains(&spec.sample_rate) {
        return Err(format!(
            "WAV sample rate must be {MIN_RATE}–{MAX_SOURCE_RATE} Hz"
        ));
    }
    let source_frames = reader.duration() as usize;
    if source_frames == 0 {
        return Err("WAV contains no audio frames".to_owned());
    }
    // Owner ruling, 2026-09-14: long files are accepted, but canonical state owns only the first
    // ten seconds. A deterministic smooth fade is applied only when source audio exists beyond the
    // boundary, keeping ordinary <=10 s responses bit-for-bit unfaded.
    let maximum_frames = (spec.sample_rate as f32 * MAX_SOURCE_SECONDS).round() as usize;
    let frames = source_frames.min(maximum_frames);
    let was_truncated = source_frames > maximum_frames;
    let values = frames
        .checked_mul(spec.channels as usize)
        .ok_or_else(|| "WAV dimensions overflow".to_owned())?;
    let mut interleaved = Vec::new();
    interleaved.try_reserve_exact(values).map_err(|_| {
        "not enough memory to decode WAV; the previous response was kept".to_owned()
    })?;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for value in reader.samples::<f32>().take(values) {
                let value = value.map_err(|error| format!("WAV decode failed: {error}"))?;
                if !value.is_finite() {
                    return Err("WAV contains a non-finite sample".to_owned());
                }
                interleaved.push(value);
            }
        }
        hound::SampleFormat::Int => {
            let scale = 2.0f32
                .powi(spec.bits_per_sample.saturating_sub(1) as i32)
                .max(1.0);
            for value in reader.samples::<i32>().take(values) {
                interleaved.push(
                    value.map_err(|error| format!("WAV decode failed: {error}"))? as f32 / scale,
                );
            }
        }
    }
    if interleaved.len() != values {
        return Err("WAV is truncated or has inconsistent frame bounds".to_owned());
    }
    let channel_count = spec.channels as usize;
    let mut channels = Vec::new();
    channels.try_reserve_exact(channel_count).map_err(|_| {
        "not enough memory to decode WAV; the previous response was kept".to_owned()
    })?;
    for channel in 0..channel_count {
        let mut samples = Vec::new();
        samples.try_reserve_exact(frames).map_err(|_| {
            "not enough memory to decode WAV; the previous response was kept".to_owned()
        })?;
        samples.extend(
            interleaved
                .iter()
                .skip(channel)
                .step_by(channel_count)
                .copied(),
        );
        if was_truncated {
            fade_import_boundary(&mut samples, spec.sample_rate);
        }
        channels.push(
            encode_channel(&samples).map_err(|error| format!("WAV encode failed: {error:?}"))?,
        );
    }
    Ok(ResponseState {
        schema: STATE_SCHEMA,
        name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("response.wav")
            .chars()
            .take(MAX_NAME_BYTES)
            .collect(),
        sample_rate: spec.sample_rate,
        frames: frames as u32,
        interpretation: if channel_count == 1 {
            Interpretation::Mono
        } else {
            Interpretation::MonoToStereo
        },
        channels,
        preparation: Preparation::default(),
    })
}

/// A short decorrelated response generated entirely by this project.
fn starter_state() -> ResponseState {
    let frames = (STARTER_RATE as f32 * STARTER_SECONDS).round() as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 0.52;
    right[0] = 0.50;
    for (milliseconds, gain_left, gain_right) in [
        (0.7, 0.20, -0.11),
        (1.3, -0.12, 0.18),
        (2.3, 0.10, 0.07),
        (3.7, -0.07, 0.09),
        (5.3, 0.05, -0.06),
    ] {
        let index = (milliseconds * STARTER_RATE as f32 / 1_000.0).round() as usize;
        if index < frames {
            left[index] += gain_left;
            right[index] += gain_right;
        }
    }
    let mut random = 0x6d2b_79f5u32;
    for index in (STARTER_RATE as usize / 2_000)..frames {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        let noise = (random as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let seconds = index as f32 / STARTER_RATE as f32;
        let envelope = (-seconds / 0.0034).exp() * 0.014;
        left[index] += noise * envelope;
        right[index] += noise.mul_add(-0.57, (noise * 3.1).sin() * 0.43) * envelope;
    }
    ResponseState {
        schema: STATE_SCHEMA,
        name: "Starter response".to_owned(),
        sample_rate: STARTER_RATE,
        frames: frames as u32,
        interpretation: Interpretation::MonoToStereo,
        channels: vec![
            encode_channel(&left).expect("starter response is bounded"),
            encode_channel(&right).expect("starter response is bounded"),
        ],
        preparation: Preparation::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every response is prepared at one loudness** (the owner, 2026-09-28): the same room
    /// recorded quiet or loud prepares to the same response, with an energy of one, and a silent
    /// file stays silent rather than being raised without limit.
    #[test]
    fn every_response_is_prepared_at_one_loudness() {
        let room: Vec<f32> = (0..480)
            .map(|i| (-(i as f32) / 80.0).exp() * if i % 3 == 0 { 1.0 } else { -0.5 })
            .collect();
        let at_level = |level: f32| {
            let samples: Vec<f32> = room.iter().map(|sample| sample * level).collect();
            ResponseState {
                schema: STATE_SCHEMA,
                name: "Level".to_owned(),
                sample_rate: 48_000,
                frames: 480,
                interpretation: Interpretation::Mono,
                channels: vec![encode_channel(&samples).unwrap()],
                preparation: Preparation::default(),
            }
        };
        let played = |state: &ResponseState| -> Vec<f32> {
            let mut convolver = prepare(state, 48_000.0).unwrap().into_convolver();
            (0..600)
                .map(|i| {
                    let input = if i == 0 { 1.0 } else { 0.0 };
                    convolver.process_excitation([input, input])[0]
                })
                .collect()
        };
        let quiet = played(&at_level(0.05));
        let loud = played(&at_level(3.0));
        let energy: f64 = quiet.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
        assert!((energy - 1.0).abs() < 1.0e-3, "prepared energy {energy}");
        for (index, (a, b)) in quiet.iter().zip(&loud).enumerate() {
            assert!((a - b).abs() < 1.0e-5, "at {index}: {a} quiet, {b} loud");
        }
        assert!(played(&at_level(0.0)).iter().all(|&x| x == 0.0));
    }

    fn tone(rate: f32, frequency: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|index| (core::f32::consts::TAU * frequency * index as f32 / rate).sin())
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|value| value * value).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn write_constant_wav(label: &str, sample_rate: u32, frames: usize) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mxm-fx-convolution-{label}-{}-{nonce}.wav",
            std::process::id()
        ));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..frames {
            writer.write_sample::<i16>(16_384).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn starter_state_is_finite_preparable_and_deadline_bounded() {
        let state = ResponseState::default();
        validate(&state).unwrap();
        let prepared = prepare(&state, 96_000.0).unwrap();
        assert_eq!(
            prepared.interpretation(),
            ResponseInterpretation::MonoToStereo
        );
        assert!(prepared.response_samples() <= maximum_processing_samples(96_000.0).unwrap());
    }

    #[test]
    fn malformed_state_and_impossible_reservation_preserve_the_field() {
        let field = ResponseField::default();
        let before = field.snapshot();
        let mut malformed = before.clone();
        malformed.channels.pop();
        assert!(field.stage(malformed).is_err());
        assert_eq!(field.snapshot(), before);

        let mut oversized = before.clone();
        oversized.frames = u32::MAX;
        assert!(field.stage(oversized).is_err());
        assert_eq!(field.snapshot(), before);
    }

    #[test]
    fn wav_import_stages_then_commits_path_independent_source_truth() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mxm-fx-convolution-{}-{nonce}.wav",
            std::process::id()
        ));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        writer.write_sample::<i16>(16_384).unwrap();
        writer.write_sample::<i16>(-8_192).unwrap();
        writer.finalize().unwrap();

        let field = ResponseField::default();
        let before = field.snapshot();
        let request = field.set_loading(&path);
        field.import_wav(request, &path).unwrap();
        assert_eq!(field.snapshot(), before, "staging leaked into active state");
        assert!(matches!(field.status(), LoadState::Ready(_)));
        assert_eq!(field.commit(), Ok(true));
        let committed = field.snapshot();
        assert_eq!(committed.frames, 2);
        assert_eq!(committed.interpretation, Interpretation::Mono);
        assert_eq!(committed.channels.len(), 1);
        assert!(!committed.channels[0].contains(path.to_string_lossy().as_ref()));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn long_wav_is_accepted_canonicalized_and_smoothly_zeroed_at_ten_seconds() {
        let rate = 8_000;
        let maximum = rate as usize * 10;
        let long_path = write_constant_wav("long", rate, maximum + 1);
        let long = decode_wav(&long_path).unwrap();
        std::fs::remove_file(long_path).unwrap();
        assert_eq!(long.frames as usize, maximum);
        let samples = decode(&long).unwrap().remove(0);
        let fade_frames = (rate as f32 * IMPORT_FADE_SECONDS).round() as usize;
        let start = maximum - fade_frames;
        assert_eq!(samples[start - 1], 0.5);
        assert_eq!(samples[start], 0.5);
        assert_eq!(samples[maximum - 1], 0.0);
        let largest_step = samples[start - 1..]
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(largest_step < 0.0021, "fade step {largest_step}");

        let exact_path = write_constant_wav("exact", rate, maximum);
        let exact = decode_wav(&exact_path).unwrap();
        std::fs::remove_file(exact_path).unwrap();
        let exact_samples = decode(&exact).unwrap().remove(0);
        assert!(
            exact_samples.iter().all(|sample| *sample == 0.5),
            "ten-second source samples were changed"
        );
    }

    #[test]
    fn only_the_latest_background_preparation_can_publish() {
        let field = ResponseField::default();
        let mut stale = ResponseModel {
            interpretation: Interpretation::MonoToStereo,
            preparation: Preparation::default(),
        };
        stale.preparation.reverse = true;
        let stale_request = field.begin_edit("stale");
        let mut latest = stale;
        latest.preparation.reverse = false;
        latest.preparation.time_percent = 75;
        let latest_request = field.begin_edit("latest");
        field.prepare_edit(stale_request, stale).unwrap();
        assert!(!matches!(field.status(), LoadState::Ready(_)));
        field.prepare_edit(latest_request, latest).unwrap();
        assert!(matches!(field.status(), LoadState::Ready(name) if name == "Starter response"));
        field.commit().unwrap();
        assert_eq!(field.snapshot().preparation, latest.preparation);
    }

    #[test]
    fn staged_engine_is_reused_and_a_later_allocation_failure_preserves_working_state() {
        let field = ResponseField::default();
        let mut first = field.snapshot();
        first.preparation.reverse = true;
        field.stage(first.clone()).unwrap();
        field.reject_next_preparation_for_test();
        field.commit().unwrap();
        let first_fingerprint = fingerprint(&first);
        assert!(
            field
                .take_prepared_engine(first_fingerprint, 48_000.0)
                .is_some()
        );
        assert_eq!(field.snapshot(), first);

        let mut rejected = first.clone();
        rejected.preparation.time_percent = 75;
        nice_plug::params::persist::PersistentField::set(&field, rejected);
        assert_eq!(field.snapshot(), first);
        assert!(field.take_preparation_rejected());
    }

    #[test]
    fn ten_second_stereo_display_is_cached_and_repeated_reads_do_not_decode() {
        let state = maximum_display_state_for_test();
        let field = ResponseField::default();
        nice_plug::params::persist::PersistentField::set(&field, state);
        assert!(!field.take_preparation_rejected());
        let builds = field.display_builds_for_test();
        for _ in 0..100 {
            let display = field.display();
            assert_eq!(display.frames, 480_000);
            assert_eq!(display.energy.len(), DISPLAY_BINS);
        }
        assert_eq!(field.display_builds_for_test(), builds);
        // Two levels per bin: still a small fixed copy, and never the embedded payload.
        assert!(core::mem::size_of::<ResponseDisplay>() < 4_096);
    }

    #[test]
    fn the_energy_display_separates_a_short_room_from_a_long_one_at_equal_peak() {
        let rate = 48_000u32;
        let room = |seconds: f32, t60: f32| {
            let frames = (rate as f32 * seconds) as usize;
            let mut random = 0x2545_f491u32;
            let samples: Vec<f32> = (0..frames)
                .map(|index| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    let noise = (random as f32 / u32::MAX as f32) * 2.0 - 1.0;
                    noise * 10f32.powf(-3.0 * index as f32 / rate as f32 / t60)
                })
                .collect();
            ResponseState {
                schema: STATE_SCHEMA,
                name: "room".to_owned(),
                sample_rate: rate,
                frames: frames as u32,
                interpretation: Interpretation::Mono,
                channels: vec![encode_channel(&samples).unwrap()],
                preparation: Preparation::default(),
            }
        };
        let short = energy_profile(&room(0.9, 0.3)).unwrap();
        let long = energy_profile(&room(8.0, 4.0)).unwrap();
        // Both open near full scale, with noise RMS below its peak.
        assert!(short[0][0] > -3.0 && long[0][0] > -3.0, "{short:?}");
        assert!(short[0][1] < short[0][0] && long[0][1] < long[0][0]);
        // A quarter-second in, the short room is already far below the long one.
        assert!(
            short[5][1] < long[5][1] - 20.0,
            "{:?} {:?}",
            short[5],
            long[5]
        );
        // Bin 40 is two seconds: nothing after the short room, a live tail in the long one.
        assert_eq!(short[40], [DISPLAY_FLOOR_DB; 2]);
        assert!(long[40][1] > -40.0, "{:?}", long[40]);
    }

    #[test]
    fn legacy_schema_one_preparation_defaults_new_model_fields_to_neutral() {
        let state = ResponseState::default();
        let mut value = serde_json::to_value(&state).unwrap();
        let preparation = value["preparation"].as_object_mut().unwrap();
        preparation.remove("decay_percent");
        preparation.remove("damping_percent");
        let restored: ResponseState = serde_json::from_value(value).unwrap();
        assert_eq!(restored.preparation.decay_percent, 100);
        assert_eq!(restored.preparation.damping_percent, 0);
        assert_eq!(
            prepare(&restored, 48_000.0).unwrap().response_samples(),
            336
        );
    }

    #[test]
    fn neutral_decay_and_damping_preserve_every_legacy_tail_shape_exactly() {
        let source = [0.7, -0.4, 0.25, -0.1, 0.03];
        for tail_shape in [
            TailShape::Natural,
            TailShape::Fade,
            TailShape::Swell,
            TailShape::Gate,
        ] {
            let prep = Preparation {
                extent: source.len() as u32,
                tail_shape,
                ..Preparation::default()
            };
            let actual = shape(&source, prep, 48_000.0).unwrap();
            let expected: Vec<f32> = source
                .iter()
                .enumerate()
                .map(|(index, sample)| {
                    let phase = index as f32 / (source.len() - 1) as f32;
                    let envelope = match tail_shape {
                        TailShape::Natural => 1.0,
                        TailShape::Fade => (1.0 - phase).powi(2),
                        TailShape::Swell => phase.powi(2),
                        TailShape::Gate => f32::from(phase < 0.75),
                    };
                    sample * envelope
                })
                .collect();
            assert_eq!(actual, expected, "{tail_shape:?}");
        }
    }

    #[test]
    fn decay_and_damping_remain_inside_selected_support_and_damping_reduces_late_hf() {
        let source: Vec<f32> = (0..512)
            .map(|index| if index % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let neutral = shape(
            &source,
            Preparation {
                extent: source.len() as u32,
                ..Preparation::default()
            },
            48_000.0,
        )
        .unwrap();
        let shortened = shape(
            &source,
            Preparation {
                extent: source.len() as u32,
                decay_percent: 50,
                ..Preparation::default()
            },
            48_000.0,
        )
        .unwrap();
        let damped = shape(
            &source,
            Preparation {
                extent: source.len() as u32,
                damping_percent: 100,
                ..Preparation::default()
            },
            48_000.0,
        )
        .unwrap();
        assert_eq!(shortened.len(), source.len());
        assert_eq!(damped.len(), source.len());
        assert!(shortened[500].abs() < neutral[500].abs());
        let late_rms = |values: &[f32]| rms(&values[384..]);
        assert!(late_rms(&damped) < late_rms(&neutral) * 0.5);
    }

    #[test]
    fn model_overlay_preserves_committed_source_and_interpretation() {
        let field = ResponseField::default();
        let before = field.snapshot();
        let mut model = field.display().model;
        model.preparation.time_percent = 75;
        model.preparation.decay_percent = 60;
        model.preparation.damping_percent = 40;
        let overlay = PreparationOverlay::from_model(model);
        field.stage_overlay(&overlay).unwrap();
        field.commit().unwrap();
        let after = field.snapshot();
        assert_eq!(after.name, before.name);
        assert_eq!(after.sample_rate, before.sample_rate);
        assert_eq!(after.frames, before.frames);
        assert_eq!(after.interpretation, before.interpretation);
        assert_eq!(after.channels, before.channels);
        assert_eq!(after.preparation, model.preparation);
    }

    #[test]
    fn reversible_preparation_uses_the_original_every_time() {
        let mut state = ResponseState::default();
        state.preparation.onset = 10;
        state.preparation.extent = 100;
        state.preparation.reverse = true;
        state.preparation.time_percent = 50;
        state.preparation.tail_shape = TailShape::Swell;
        let first = prepare(&state, 48_000.0).unwrap();
        let second = prepare(&state, 48_000.0).unwrap();
        assert_eq!(first.response_samples(), 50);
        assert_eq!(second.response_samples(), first.response_samples());
    }

    #[test]
    fn band_limited_conversion_retains_passband_and_rejects_above_target_nyquist() {
        let pass = tone(384_000.0, 1_000.0, 38_400);
        let stop = tone(384_000.0, 80_000.0, 38_400);
        let pass = resample_bandlimited(&pass, 384_000.0, 48_000.0).unwrap();
        let stop = resample_bandlimited(&stop, 384_000.0, 48_000.0).unwrap();
        assert!(rms(&pass[128..pass.len() - 128]) > 0.65);
        assert!(rms(&stop[128..stop.len() - 128]) < 0.02);
    }

    #[test]
    fn up_and_down_conversion_preserve_duration_and_impulse_onset() {
        for (source_rate, target_rate) in [(8_000.0, 384_000.0), (384_000.0, 8_000.0)] {
            let frames = source_rate as usize / 10;
            let mut impulse = vec![0.0; frames];
            impulse[frames / 4] = 1.0;
            let converted = resample_bandlimited(&impulse, source_rate, target_rate).unwrap();
            assert!(
                (converted.len() as f64 / target_rate as f64 - 0.1).abs()
                    <= 1.0 / target_rate as f64
            );
            let peak = converted
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.abs().total_cmp(&right.1.abs()))
                .unwrap()
                .0;
            assert!((peak as f64 / target_rate as f64 - 0.025).abs() < 2.0 / target_rate as f64);
        }
    }

    #[test]
    fn ten_seconds_is_guaranteed_at_fractional_rates_through_96_khz_and_high_rates_are_bounded() {
        assert_eq!(maximum_processing_samples(8_000.0), Some(80_000));
        assert_eq!(maximum_processing_samples(48_000.0), Some(480_000));
        assert_eq!(maximum_processing_samples(96_000.0), Some(960_000));
        assert_eq!(maximum_processing_samples(128_000.0), Some(4_096));
        assert_eq!(maximum_processing_samples(192_000.0), Some(4_096));
        assert_eq!(maximum_processing_samples(384_000.0), Some(4_096));

        let fractional_samples = vec![0.0; 80_000];
        let fractional_state = ResponseState {
            schema: STATE_SCHEMA,
            name: "Fractional-rate ten seconds".to_owned(),
            sample_rate: 8_000,
            frames: 80_000,
            interpretation: Interpretation::Mono,
            channels: vec![encode_channel(&fractional_samples).unwrap()],
            preparation: Preparation::default(),
        };
        for rate in [12_345.67, 95_999.99] {
            let expected = converted_sample_count(80_000, 8_000.0, rate).unwrap();
            assert_eq!(maximum_processing_samples(rate), Some(expected));
            assert_eq!(
                prepare(&fractional_state, rate).unwrap().response_samples(),
                expected
            );
        }

        let samples = vec![0.0; 960_000];
        let state = ResponseState {
            schema: STATE_SCHEMA,
            name: "Ten seconds".to_owned(),
            sample_rate: 96_000,
            frames: 960_000,
            interpretation: Interpretation::Mono,
            channels: vec![encode_channel(&samples).unwrap()],
            preparation: Preparation::default(),
        };
        assert_eq!(
            prepare(&state, 96_000.0).unwrap().response_samples(),
            960_000
        );

        let mut playback_samples = vec![0.0; 80_000];
        playback_samples[79_999] = 0.25;
        let playback = ResponseState {
            sample_rate: 8_000,
            frames: 80_000,
            channels: vec![encode_channel(&playback_samples).unwrap()],
            ..state.clone()
        };
        let mut convolver = prepare(&playback, 8_000.0).unwrap().into_convolver();
        let mut final_sample = 0.0;
        for index in 0..80_000 {
            let input = if index == 0 { 1.0 } else { 0.0 };
            final_sample = convolver.process_excitation([input, input])[0];
        }
        // One impulse of 0.25 is normalised to unit energy, so it plays back at one.
        assert!((final_sample - 1.0).abs() < 1.2e-4);

        let accepted_samples = vec![0.0; 4_096];
        let accepted = ResponseState {
            sample_rate: 384_000,
            frames: 4_096,
            channels: vec![encode_channel(&accepted_samples).unwrap()],
            ..state.clone()
        };
        assert_eq!(
            prepare(&accepted, 384_000.0).unwrap().response_samples(),
            4_096
        );

        let rejected_samples = vec![0.0; 4_097];
        let rejected = ResponseState {
            sample_rate: 384_000,
            frames: 4_097,
            channels: vec![encode_channel(&rejected_samples).unwrap()],
            ..state
        };
        assert!(matches!(
            prepare(&rejected, 384_000.0),
            Err(StateError::Deadline)
        ));
        assert!(
            preparation_message(StateError::Deadline, 384_000.0)
                .contains("measured 0.011 s processing budget at 384000 Hz")
        );
        let field = ResponseField::default();
        field.set_processing_rate(384_000.0);
        let before = field.snapshot();
        let message = field.stage(rejected).unwrap_err();
        assert!(message.contains("measured 0.011 s processing budget at 384000 Hz"));
        assert_eq!(
            field.snapshot(),
            before,
            "deadline rejection truncated state"
        );
    }

    #[test]
    fn ordinary_budget_edits_choose_the_exact_fitting_boundary_and_publish_information() {
        let samples = vec![0.0; 80_000];
        let mut state = ResponseState {
            schema: STATE_SCHEMA,
            name: "Clamp source".to_owned(),
            sample_rate: 8_000,
            frames: 80_000,
            interpretation: Interpretation::Mono,
            channels: vec![encode_channel(&samples).unwrap()],
            preparation: Preparation {
                time_percent: 400,
                ..Preparation::default()
            },
        };

        let (size, information) =
            normalize_budget(state.clone(), 48_000.0, Some(BudgetAxis::Size)).unwrap();
        assert_eq!(size.preparation.time_percent, 100);
        assert!(information.unwrap().contains("Size"));
        let mut one_more = size.clone();
        one_more.preparation.time_percent += 1;
        assert!(fits_processing_budget(&size, 48_000.0).unwrap());
        assert!(!fits_processing_budget(&one_more, 48_000.0).unwrap());
        assert!(prepare(&size, 48_000.0).is_ok());

        state.preparation.time_percent = 200;
        let (extent, information) =
            normalize_budget(state.clone(), 48_000.0, Some(BudgetAxis::Extent)).unwrap();
        assert_eq!(extent.preparation.extent, 40_000);
        assert!(information.unwrap().contains("Extent"));
        one_more = extent.clone();
        one_more.preparation.extent += 1;
        assert!(fits_processing_budget(&extent, 48_000.0).unwrap());
        assert!(!fits_processing_budget(&one_more, 48_000.0).unwrap());

        state.preparation.extent = 0;
        let (onset, information) =
            normalize_budget(state.clone(), 48_000.0, Some(BudgetAxis::Onset)).unwrap();
        assert_eq!(onset.preparation.onset, 40_000);
        assert!(information.unwrap().contains("Onset"));
        let mut one_earlier = onset.clone();
        one_earlier.preparation.onset -= 1;
        assert!(fits_processing_budget(&onset, 48_000.0).unwrap());
        assert!(!fits_processing_budget(&one_earlier, 48_000.0).unwrap());

        let field = ResponseField::default();
        field.set_processing_rate(48_000.0);
        field.stage(state.clone()).unwrap();
        field.commit().unwrap();
        assert_eq!(field.snapshot().preparation.time_percent, 100);
        assert_eq!(field.display().model.preparation.time_percent, 100);
        assert!(matches!(
            field.status(),
            LoadState::Information(message) if message.contains("Size")
        ));

        let restored = ResponseField::default();
        restored.set_processing_rate(48_000.0);
        nice_plug::params::persist::PersistentField::set(&restored, state);
        assert_eq!(
            restored.snapshot().preparation.time_percent,
            100,
            "an accepted restore must immediately persist its applied maximum"
        );
        assert!(matches!(
            restored.status(),
            LoadState::Information(message) if message.contains("Size")
        ));
    }

    #[test]
    fn high_rate_fixed_budget_remains_a_failure_atomic_refusal() {
        let mut state = ResponseState::default();
        state.preparation.time_percent = 400;
        assert!(matches!(
            normalize_budget(state, 384_000.0, Some(BudgetAxis::Size)),
            Err(StateError::Deadline)
        ));
    }

    #[test]
    fn provenance_name_does_not_change_the_audio_fingerprint() {
        let state = ResponseState::default();
        let mut renamed = state.clone();
        renamed.name = "Renamed".to_owned();
        assert_eq!(fingerprint(&state), fingerprint(&renamed));
        renamed.preparation.reverse = true;
        assert_ne!(fingerprint(&state), fingerprint(&renamed));
    }
}
