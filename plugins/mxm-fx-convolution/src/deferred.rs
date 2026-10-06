//! Production integration of the shared deferred-preset transaction.
//!
//! Response decoding, resampling, display construction and engine allocation finish on the plugin's
//! background executor. The editor then emits eligible host gestures and commits one prebuilt
//! response while the wrapper holds processing at a state boundary. The first following process
//! callback acknowledges that publication; only its background acknowledgement installs preset
//! identity and the dirty baseline.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use mxm_preset::{
    DeferredContent, DeferredPresetTransaction, DeferredPresetUiRequest, DeferredRequestId,
    DeferredStatus,
};
use nice_plug::prelude::ParamSetter;

use crate::params::MxmFxConvolutionParams;
use crate::response::{PreparationOverlay, ResponseModel, ResponseState};

/// The label a preset's response loads under, which its status line names in words.
pub(crate) const PRESET_EDIT: &str = "preset";

#[derive(Clone, Debug)]
pub struct DeferredPresetWork {
    pub transaction: DeferredRequestId,
    pub response_request: u64,
    pub content: DeferredContent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrackedResponseRequest {
    transaction: DeferredRequestId,
    response_request: u64,
    host_restore_generation: u64,
}

struct TransactionState {
    transaction: DeferredPresetTransaction,
    response_request: Option<TrackedResponseRequest>,
    awaiting_response: Option<TrackedResponseRequest>,
}

impl Default for TransactionState {
    fn default() -> Self {
        Self {
            transaction: DeferredPresetTransaction::new(),
            response_request: None,
            awaiting_response: None,
        }
    }
}

/// One transaction owner shared by the plugin instance and every reconstruction of its editor.
#[derive(Default)]
pub struct DeferredPresetController {
    /// The collection's impulses folder, where a room from the factory list is read from —
    /// injected, so a test never reads the folder of whoever runs it (`crate::impulses`).
    impulses: Option<PathBuf>,
    state: Mutex<TransactionState>,
    publication_id: AtomicU64,
    publication_ready: AtomicBool,
    acknowledgement_id: AtomicU64,
    acknowledgement_ready: AtomicBool,
}

impl DeferredPresetController {
    fn state(&self) -> MutexGuard<'_, TransactionState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Capture a complete preset or source-preserving Init without changing parameters, response or
    /// identity. Beginning another request invalidates the older response worker first.
    pub fn begin(
        &self,
        params: &MxmFxConvolutionParams,
        request: DeferredPresetUiRequest,
    ) -> Result<(DeferredPresetWork, Vec<String>), String> {
        let mut state = self.state();
        let status = state.transaction.status();
        if let Some(tracked) = state.response_request.take() {
            if matches!(
                status,
                DeferredStatus::AwaitingProcessCommit(id) if id == tracked.transaction
            ) {
                debug_assert!(state.awaiting_response.is_none());
                state.awaiting_response = Some(tracked);
            } else {
                params.response.cancel_request(tracked.response_request);
            }
        }

        let (request, problems) = match request {
            DeferredPresetUiRequest::Load {
                name,
                origin,
                preset,
            } => {
                let content = match preset.state.as_ref() {
                    Some(value) => DeferredContent::Complete(value.clone()),
                    None => {
                        let committed = params.response.snapshot();
                        let overlay = PreparationOverlay::from_model(ResponseModel {
                            interpretation: committed.interpretation,
                            preparation: committed.preparation,
                        });
                        DeferredContent::MergeWithCommittedSource(
                            serde_json::to_value(overlay).map_err(|error| {
                                format!("response overlay encode failed: {error}")
                            })?,
                        )
                    }
                };
                state
                    .transaction
                    .begin_preset(params, &preset, &name, origin, content)
                    .map_err(|_| "deferred preset request ids are exhausted".to_owned())?
            }
            DeferredPresetUiRequest::Init => {
                let overlay = serde_json::to_value(PreparationOverlay::init())
                    .map_err(|error| format!("Init overlay encode failed: {error}"))?;
                let request = state
                    .transaction
                    .begin_init(params, overlay)
                    .map_err(|_| "deferred preset request ids are exhausted".to_owned())?;
                (request, Vec::new())
            }
        };
        let response_request = params.response.begin_edit(PRESET_EDIT);
        state.response_request = Some(TrackedResponseRequest {
            transaction: request.id,
            response_request,
            host_restore_generation: params.response.host_restore_generation(),
        });
        Ok((
            DeferredPresetWork {
                transaction: request.id,
                response_request,
                content: request.content,
            },
            problems,
        ))
    }

    /// Begin an ordinary WAV load in the same ordering domain as deferred preset work.
    pub fn begin_wav_load(&self, params: &MxmFxConvolutionParams, path: &Path) -> u64 {
        self.cancel_unpublished(params);
        params.response.set_loading(path)
    }

    /// Begin an ordinary response-model edit in the same ordering domain as deferred preset work.
    pub fn begin_response_edit(&self, params: &MxmFxConvolutionParams, name: &str) -> u64 {
        self.cancel_unpublished(params);
        params.response.begin_edit(name)
    }

    /// A controller that reads the factory list's rooms from `impulses`.
    pub fn with_impulses(impulses: Option<PathBuf>) -> Self {
        Self {
            impulses,
            ..Self::default()
        }
    }

    /// Execute the expensive half on nice-plug's background worker. Stale completion is inert even
    /// when workers finish out of order. **A room from the impulses folder is read here**, off the
    /// editor and the audio thread, exactly as Browse reads a file; a file that is gone or will not
    /// read fails the load, and the response playing keeps playing.
    pub fn prepare(&self, params: &MxmFxConvolutionParams, work: DeferredPresetWork) {
        let result = match &work.content {
            DeferredContent::Complete(value) => match crate::impulses::reference(value) {
                Some(relative) => crate::impulses::load(self.impulses.as_deref(), relative)
                    .and_then(|response| {
                        params
                            .response
                            .prepare_deferred_state(work.response_request, response)
                    }),
                None => serde_json::from_value::<ResponseState>(value.clone())
                    .map_err(|error| format!("invalid embedded response: {error}"))
                    .and_then(|response| {
                        params
                            .response
                            .prepare_deferred_state(work.response_request, response)
                    }),
            },
            DeferredContent::MergeWithCommittedSource(value)
            | DeferredContent::InitMergeWithCommittedSource(value) => {
                serde_json::from_value::<PreparationOverlay>(value.clone())
                    .map_err(|error| format!("invalid response overlay: {error}"))
                    .and_then(|overlay| {
                        params
                            .response
                            .prepare_deferred_overlay(work.response_request, &overlay)
                    })
            }
        };

        let mut state = self.state();
        let current = state.response_request.is_some_and(|tracked| {
            tracked.transaction == work.transaction
                && tracked.response_request == work.response_request
        });
        if !current {
            return;
        }
        match result {
            Ok(true) => {
                state.transaction.ready(work.transaction);
            }
            Ok(false) => {
                // A non-controller response request superseded this worker. It already owns the
                // response field's latest id, so retire the matching transaction instead of leaving
                // the editor permanently Pending.
                state.response_request = None;
                state.transaction.cancel(work.transaction);
            }
            Err(error) => {
                params
                    .response
                    .fail_request(work.response_request, error.clone());
                state.transaction.reject(work.transaction, error);
            }
        }
    }

    /// Finish any callback acknowledgement, then publish a newly Ready transaction. This runs from
    /// the editor frame; all expensive response work has already completed.
    ///
    /// **Returns whether it published**, so the editor can wake the audio side: publication is
    /// acknowledged only by the next process callback (`acknowledge_process_boundary`), and a host
    /// that sleeps an idle effect — the MXM player does — may already have spent the wake the
    /// parameter gestures asked for before publication was marked ready. Without a wake after it,
    /// the load sits unacknowledged and the bar keeps saying *No preset* (the owner, 2026-09-28).
    pub fn service(&self, params: &MxmFxConvolutionParams, setter: &ParamSetter<'_>) -> bool {
        // One editor-frame observation point. Reading unmodulated values keeps host modulation out
        // of base-edit ordering while still detecting an away-and-back edit across frames.
        params.observe_parameter_edits();
        self.adopt_acknowledgement(params);

        let mut state = self.state();
        if let Some(awaiting) = state.awaiting_response
            && params.response.host_restore_generation() != awaiting.host_restore_generation
        {
            state.transaction.cancel(awaiting.transaction);
            state.awaiting_response = None;
            self.publication_ready.store(false, Ordering::Release);
            self.acknowledgement_ready.store(false, Ordering::Release);
        }

        let status = state.transaction.status();
        if let Some(tracked) = state.response_request {
            let invalidated = match &status {
                DeferredStatus::AwaitingProcessCommit(transaction) => {
                    tracked.transaction != *transaction
                        || params.response.host_restore_generation()
                            != tracked.host_restore_generation
                }
                _ => !params.response.request_is_current(tracked.response_request),
            };
            if invalidated {
                state.transaction.cancel(tracked.transaction);
                state.response_request = None;
                if matches!(status, DeferredStatus::AwaitingProcessCommit(_)) {
                    self.publication_ready.store(false, Ordering::Release);
                    self.acknowledgement_ready.store(false, Ordering::Release);
                }
                return false;
            }
        }
        if !matches!(status, DeferredStatus::Ready(_)) {
            return false;
        }
        let mut committed = false;
        let published = state.transaction.publish_ready(params, setter, |id| {
            committed = matches!(params.response.commit(), Ok(true));
            if committed {
                self.publication_id.store(id.get(), Ordering::Relaxed);
                self.publication_ready.store(true, Ordering::Release);
            }
        });
        if published && !committed {
            let id = match state.transaction.status() {
                DeferredStatus::AwaitingProcessCommit(id) => id,
                _ => return false,
            };
            state.transaction.cancel(id);
            state.response_request = None;
            params.response.fail(
                "prepared preset could not be published; the previous response and identity were kept",
            );
        }
        published && committed
    }

    /// Called once at the start of the production process callback. No locks, allocation or drops.
    pub fn acknowledge_process_boundary(&self) -> Option<u64> {
        if !self.publication_ready.swap(false, Ordering::AcqRel) {
            return None;
        }
        let id = self.publication_id.load(Ordering::Relaxed);
        self.acknowledgement_id.store(id, Ordering::Relaxed);
        self.acknowledgement_ready.store(true, Ordering::Release);
        Some(id)
    }

    /// Background/main-thread half of callback acknowledgement. A successor may already be pending,
    /// but a stale id cannot rename it and a host restore still invalidates the published predecessor.
    pub fn acknowledge(&self, id: u64, params: &MxmFxConvolutionParams) -> bool {
        let mut state = self.state();
        let (tracked, predecessor) = if let Some(tracked) = state
            .awaiting_response
            .filter(|tracked| tracked.transaction.get() == id)
        {
            (tracked, true)
        } else if let Some(tracked) = state
            .response_request
            .filter(|tracked| tracked.transaction.get() == id)
        {
            (tracked, false)
        } else {
            return false;
        };

        if params.response.host_restore_generation() != tracked.host_restore_generation {
            state.transaction.cancel(tracked.transaction);
            if predecessor {
                state.awaiting_response = None;
            } else {
                state.response_request = None;
            }
            self.acknowledgement_ready.store(false, Ordering::Release);
            return false;
        }

        let acknowledged = state.transaction.acknowledge(tracked.transaction, params);
        if acknowledged {
            if predecessor {
                state.awaiting_response = None;
            } else {
                state.response_request = None;
            }
            self.acknowledgement_ready.store(false, Ordering::Release);
        }
        acknowledged
    }

    fn adopt_acknowledgement(&self, params: &MxmFxConvolutionParams) {
        if self.acknowledgement_ready.swap(false, Ordering::AcqRel) {
            let id = self.acknowledgement_id.load(Ordering::Relaxed);
            let _ = self.acknowledge(id, params);
        }
    }

    /// Cancel only work that has not crossed the publication boundary. An Awaiting transaction is
    /// already active and must retain its callback acknowledgement even if its editor closes.
    pub fn cancel_unpublished(&self, params: &MxmFxConvolutionParams) {
        let mut state = self.state();
        let id = match state.transaction.status() {
            DeferredStatus::Pending(id)
            | DeferredStatus::Ready(id)
            | DeferredStatus::Rejected(id, _) => id,
            DeferredStatus::Idle | DeferredStatus::AwaitingProcessCommit(_) => return,
        };
        if let Some(tracked) = state.response_request.take()
            && tracked.transaction == id
        {
            params.response.cancel_request(tracked.response_request);
        }
        state.transaction.cancel(id);
    }

    pub fn status(&self) -> DeferredStatus {
        self.state().transaction.status()
    }

    pub fn pending(&self) -> bool {
        let state = self.state();
        state.awaiting_response.is_some()
            || matches!(
                state.transaction.status(),
                DeferredStatus::Pending(_)
                    | DeferredStatus::Ready(_)
                    | DeferredStatus::AwaitingProcessCommit(_)
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mxm_preset::{Category, Loaded, Origin, Preset, loaded, mark_loaded};
    use nice_plug::params::InternalParamMut;
    use nice_plug::params::internals::ParamPtr;
    use nice_plug::prelude::{Param, PluginApi, PluginState};

    struct ApplyingHost;

    impl nice_plug::context::gui::GuiContextInner for ApplyingHost {
        // A test double has no host to ask for a restart (nice-plug 0.4).
        fn request_restart(&self) {}
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            // SAFETY: every pointer comes from `params`, which outlives this stack host and setter.
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

    fn publish_target_awaiting_callback(
        params: &MxmFxConvolutionParams,
        controller: &DeferredPresetController,
    ) -> ResponseState {
        let mut target_response = params.response.snapshot();
        target_response.preparation.reverse = !target_response.preparation.reverse;
        let mut preset = Preset::capture("Target", Category::Fx, params);
        preset.params.get_mut("mix").unwrap().v = 0.1;
        preset.state = Some(serde_json::to_value(&target_response).unwrap());
        let (work, problems) = controller
            .begin(
                params,
                DeferredPresetUiRequest::Load {
                    name: "Target".to_owned(),
                    origin: Origin::User,
                    preset,
                },
            )
            .unwrap();
        assert!(problems.is_empty());
        controller.prepare(params, work);
        controller.service(params, &ParamSetter::new(&ApplyingHost));
        assert_eq!(params.response.snapshot(), target_response);
        assert!(matches!(
            controller.status(),
            DeferredStatus::AwaitingProcessCommit(_)
        ));
        target_response
    }

    fn one_frame_wav(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mxm-fx-convolution-deferred-{label}-{}-{nonce}.wav",
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
        writer.finalize().unwrap();
        path
    }

    /// A temporary impulses folder holding one short stereo room at `relative`.
    fn impulses_folder(label: &str, relative: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mxm-fx-convolution-impulses-{label}-{}-{nonce}",
            std::process::id()
        ));
        let path = relative
            .split('/')
            .fold(root.clone(), |path, part| path.join(part));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for (left, right) in [(0.5f32, 0.25f32), (-0.25, 0.125), (0.125, -0.0625)] {
            writer.write_sample(left).unwrap();
            writer.write_sample(right).unwrap();
        }
        writer.finalize().unwrap();
        root
    }

    /// **A room from the impulses folder loads its file and reads Clean under its name** (the
    /// owner, 2026-09-28: every impulse file is a preset): the response committed is the file,
    /// read as Browse reads it, and the identity is the room's.
    #[test]
    fn a_found_preset_loads_its_file_and_reads_clean() {
        let params = MxmFxConvolutionParams::default();
        let relative = "halls/small-hall/small-hall.far.stereo.wav";
        let root = impulses_folder("found", relative);
        let controller = DeferredPresetController::with_impulses(Some(root.clone()));
        let found = crate::impulses::scan(Some(&root), &params);
        assert_eq!(found.len(), 1);
        let preset = found[0].preset.clone();
        assert_eq!(preset.name, "Small hall");

        let (work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: preset.name.clone(),
                    origin: Origin::Factory,
                    preset,
                },
            )
            .unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        controller.prepare(&params, work);
        // Publishing is reported once, so the editor wakes the audio side for the acknowledgement
        // exactly then; a later frame with nothing to publish does not.
        assert!(controller.service(&params, &ParamSetter::new(&ApplyingHost)));
        assert!(!controller.service(&params, &ParamSetter::new(&ApplyingHost)));
        let request = controller
            .acknowledge_process_boundary()
            .expect("the callback acknowledges publication");
        assert!(controller.acknowledge(request, &params));

        let file = relative
            .split('/')
            .fold(root.clone(), |path, part| path.join(part));
        assert_eq!(
            params.response.snapshot(),
            crate::response::decode_wav(&file).unwrap()
        );
        assert_eq!(
            loaded(&params),
            Loaded::Clean {
                name: "Small hall".to_owned(),
                origin: Origin::Factory,
            }
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// **A room whose file has gone fails, and the response playing keeps playing**; a reference
    /// that leaves the folder is refused before anything is read.
    #[test]
    fn a_missing_impulse_fails_and_keeps_the_current_response() {
        let params = MxmFxConvolutionParams::default();
        let relative = "halls/hall/hall.wav";
        let root = impulses_folder("missing", relative);
        let controller = DeferredPresetController::with_impulses(Some(root.clone()));
        let preset = crate::impulses::scan(Some(&root), &params)[0]
            .preset
            .clone();
        std::fs::remove_dir_all(&root).unwrap();
        let before = params.response.snapshot();

        let (work, _) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: preset.name.clone(),
                    origin: Origin::Factory,
                    preset,
                },
            )
            .unwrap();
        controller.prepare(&params, work);
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        assert_eq!(params.response.snapshot(), before);
        assert!(matches!(
            params.response.status(),
            crate::response::LoadState::Failed(message) if message.contains(relative)
        ));
        assert!(
            crate::impulses::load(Some(&root), "../elsewhere.wav")
                .unwrap_err()
                .contains("not in the impulses folder")
        );
    }

    #[test]
    fn production_selection_prepares_then_waits_for_the_process_ack_before_identity() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        mark_loaded(&params, "Before", Origin::Factory);
        let before_response = params.response.snapshot();
        let mut preset = Preset::capture("After", Category::Fx, &params);
        preset.params.get_mut("mix").unwrap().v = 0.1;
        let mut after_response = before_response.clone();
        after_response.preparation.reverse = true;
        preset.state = Some(serde_json::to_value(&after_response).unwrap());

        let (work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "After".to_owned(),
                    origin: Origin::User,
                    preset,
                },
            )
            .unwrap();
        assert!(problems.is_empty());
        assert!(controller.pending());
        assert_eq!(params.response.snapshot(), before_response);
        assert_eq!(loaded(&params).name(), Some("Before"));

        controller.prepare(&params, work);
        assert!(matches!(controller.status(), DeferredStatus::Ready(_)));
        assert_eq!(params.response.snapshot(), before_response);
        let host = ApplyingHost;
        controller.service(&params, &ParamSetter::new(&host));
        assert_eq!(params.response.snapshot(), after_response);
        assert!((params.mix.modulated_normalized_value() - 0.1).abs() < 1.0e-6);
        assert_eq!(loaded(&params).name(), Some("Before"));
        assert!(matches!(
            controller.status(),
            DeferredStatus::AwaitingProcessCommit(_)
        ));

        let request = controller
            .acknowledge_process_boundary()
            .expect("first production callback acknowledges publication");
        assert_eq!(loaded(&params).name(), Some("Before"));
        assert!(controller.acknowledge(request, &params));
        assert_eq!(
            loaded(&params),
            Loaded::Clean {
                name: "After".to_owned(),
                origin: Origin::User,
            }
        );
    }

    #[test]
    fn callback_ack_retains_targets_across_pre_and_post_publication_edits() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        let mut preset = Preset::capture("Target", Category::Fx, &params);
        preset.params.get_mut("mix").unwrap().v = 0.1;
        let width_target =
            mxm_preset::ErasedParam::canonical_normalised(&params.width, preset.params["width"].v);
        let mut target_response = params.response.snapshot();
        target_response.preparation.reverse = true;
        preset.state = Some(serde_json::to_value(&target_response).unwrap());
        let (work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "Target".to_owned(),
                    origin: Origin::User,
                    preset,
                },
            )
            .unwrap();
        assert!(problems.is_empty());

        // This edit wins while Pending, so publication skips Width.
        unsafe {
            let _ = params.width._internal_set_normalized_value(0.8);
        }
        controller.prepare(&params, work);
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        // This edit lands after publication but before the process acknowledgement.
        unsafe {
            let _ = params.mix._internal_set_normalized_value(0.7);
        }
        let request = controller.acknowledge_process_boundary().unwrap();
        assert!(controller.acknowledge(request, &params));
        assert_eq!(
            loaded(&params),
            Loaded::Modified {
                name: "Target".to_owned(),
                origin: Origin::User,
            }
        );
        let identity = params.preset.read().unwrap();
        assert!((identity.baseline["mix"] - 0.1).abs() < 1.0e-6);
        assert!((identity.baseline["width"] - width_target).abs() < 1.0e-6);
        assert_eq!(
            identity.state_fingerprint,
            Some(crate::response::fingerprint(&target_response))
        );
    }

    #[test]
    fn host_modulation_during_pending_does_not_mask_the_preset_base_target() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        let mut preset = Preset::capture("Target", Category::Fx, &params);
        preset.params.get_mut("mix").unwrap().v = 0.1;
        let (work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "Target".to_owned(),
                    origin: Origin::User,
                    preset,
                },
            )
            .unwrap();
        assert!(problems.is_empty());
        let revision = params.parameter_edit_revision("mix").unwrap();

        // This is the direct nice-plug seam reached by CLAP_EVENT_PARAM_MOD. It changes audible
        // value but leaves unmodulated_normalized_value(), and therefore the base revision, alone.
        unsafe {
            let _ = params.mix._internal_modulate_value(0.2);
        }
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        assert_eq!(params.parameter_edit_revision("mix"), Some(revision));

        controller.prepare(&params, work);
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        assert!((params.mix.unmodulated_normalized_value() - 0.1).abs() < 1.0e-6);
        let request = controller.acknowledge_process_boundary().unwrap();
        assert!(controller.acknowledge(request, &params));
        assert_eq!(
            loaded(&params),
            Loaded::Clean {
                name: "Target".to_owned(),
                origin: Origin::User,
            }
        );
    }

    #[test]
    fn away_and_back_parameter_edit_advances_revision_and_wins() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        let observed = params.mix.unmodulated_normalized_value();
        let mut preset = Preset::capture("Target", Category::Fx, &params);
        preset.params.get_mut("mix").unwrap().v = 0.1;
        let (work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "Target".to_owned(),
                    origin: Origin::User,
                    preset,
                },
            )
            .unwrap();
        assert!(problems.is_empty());

        // Final value equality is deliberate. Each service call models an editor frame and samples
        // the unmodulated base, so both observable legs advance the monotonic revision.
        unsafe {
            let _ = params.mix._internal_set_normalized_value(0.9);
        }
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        unsafe {
            let _ = params.mix._internal_set_normalized_value(observed);
        }
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        controller.prepare(&params, work);
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        assert_eq!(params.mix.unmodulated_normalized_value(), observed);
        let request = controller.acknowledge_process_boundary().unwrap();
        assert!(controller.acknowledge(request, &params));
        assert_eq!(
            loaded(&params),
            Loaded::Modified {
                name: "Target".to_owned(),
                origin: Origin::User,
            }
        );
    }

    #[test]
    fn published_preset_identity_precedes_an_ordinary_model_successor() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        mark_loaded(&params, "Before", Origin::Factory);
        let target = publish_target_awaiting_callback(&params, &controller);

        let mut successor_model = ResponseModel {
            interpretation: target.interpretation,
            preparation: target.preparation,
        };
        successor_model.preparation.time_percent = 75;
        let successor = controller.begin_response_edit(&params, "successor");
        let request = controller
            .acknowledge_process_boundary()
            .expect("published preset still reaches the callback");
        assert!(controller.acknowledge(request, &params));
        assert_eq!(
            loaded(&params),
            Loaded::Clean {
                name: "Target".to_owned(),
                origin: Origin::User,
            },
            "starting the model successor must not cancel the published preset identity"
        );

        params
            .response
            .prepare_edit(successor, successor_model)
            .unwrap();
        assert_eq!(params.response.commit(), Ok(true));
        assert_eq!(
            loaded(&params),
            Loaded::Modified {
                name: "Target".to_owned(),
                origin: Origin::User,
            },
            "the later model publication makes the acknowledged preset baseline dirty"
        );
    }

    #[test]
    fn published_preset_identity_precedes_an_ordinary_wav_successor() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        mark_loaded(&params, "Before", Origin::Factory);
        let _target = publish_target_awaiting_callback(&params, &controller);
        let path = one_frame_wav("successor");

        let successor = controller.begin_wav_load(&params, &path);
        let request = controller
            .acknowledge_process_boundary()
            .expect("published preset still reaches the callback");
        assert!(controller.acknowledge(request, &params));
        assert_eq!(
            loaded(&params),
            Loaded::Clean {
                name: "Target".to_owned(),
                origin: Origin::User,
            },
            "starting the source successor must not cancel the published preset identity"
        );

        params.response.import_wav(successor, &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(params.response.commit(), Ok(true));
        assert_eq!(
            loaded(&params),
            Loaded::Modified {
                name: "Target".to_owned(),
                origin: Origin::User,
            },
            "the later source publication makes the acknowledged preset baseline dirty"
        );
    }

    #[test]
    fn host_restore_after_publication_cancels_the_unacknowledged_identity() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        mark_loaded(&params, "Before", Origin::Factory);
        let target = publish_target_awaiting_callback(&params, &controller);
        let mut restored = target;
        restored.preparation.reverse = !restored.preparation.reverse;

        nice_plug::params::persist::PersistentField::set(&params.response, restored.clone());
        mark_loaded(&params, "Restored", Origin::User);
        controller.service(&params, &ParamSetter::new(&ApplyingHost));

        assert_eq!(controller.status(), DeferredStatus::Idle);
        assert_eq!(params.response.snapshot(), restored);
        assert_eq!(loaded(&params).name(), Some("Restored"));
        assert!(controller.acknowledge_process_boundary().is_none());
    }

    #[test]
    fn host_restore_during_pending_preparation_supersedes_the_older_worker() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        mark_loaded(&params, "Before", Origin::Factory);
        let mut preset = Preset::capture("Old worker", Category::Fx, &params);
        preset.params.get_mut("mix").unwrap().v = 0.1;
        let mut old_response = params.response.snapshot();
        old_response.preparation.reverse = true;
        preset.state = Some(serde_json::to_value(&old_response).unwrap());
        let (work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "Old worker".to_owned(),
                    origin: Origin::User,
                    preset,
                },
            )
            .unwrap();
        assert!(problems.is_empty());

        // The durable response may compare equal while other host-state fields differ. Restore is
        // still a newer transaction and must invalidate the older response worker.
        let restored = params.response.snapshot();
        nice_plug::params::persist::PersistentField::set(&params.response, restored.clone());
        unsafe {
            let _ = params.mix._internal_set_normalized_value(0.8);
        }
        mark_loaded(&params, "Restored", Origin::User);

        // The old worker completes after the restore. Its response request is stale, which also
        // retires its parameter and identity transaction before either can publish.
        controller.prepare(&params, work);
        controller.service(&params, &ParamSetter::new(&ApplyingHost));
        assert_eq!(controller.status(), DeferredStatus::Idle);
        assert_eq!(params.response.snapshot(), restored);
        assert!((params.mix.unmodulated_normalized_value() - 0.8).abs() < 1.0e-6);
        assert_eq!(loaded(&params).name(), Some("Restored"));
        assert!(controller.acknowledge_process_boundary().is_none());
    }

    #[test]
    fn production_init_preserves_source_and_clears_identity_only_after_callback_ack() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        let mut source = params.response.snapshot();
        source.name = "Kept source".to_owned();
        source.preparation.reverse = true;
        nice_plug::params::persist::PersistentField::set(&params.response, source.clone());
        unsafe {
            let _ = params.mix._internal_set_normalized_value(0.9);
        }
        mark_loaded(&params, "Before", Origin::User);

        let (work, problems) = controller
            .begin(&params, DeferredPresetUiRequest::Init)
            .unwrap();
        assert!(problems.is_empty());
        controller.prepare(&params, work);
        let host = ApplyingHost;
        controller.service(&params, &ParamSetter::new(&host));
        let published = params.response.snapshot();
        assert_eq!(published.name, source.name);
        assert_eq!(published.channels, source.channels);
        assert_eq!(published.interpretation, source.interpretation);
        assert_eq!(
            published.preparation,
            crate::response::Preparation::default()
        );
        assert_eq!(loaded(&params).name(), Some("Before"));

        let request = controller.acknowledge_process_boundary().unwrap();
        assert!(controller.acknowledge(request, &params));
        assert_eq!(loaded(&params), Loaded::None);
        assert_eq!(
            params.mix.modulated_normalized_value(),
            params.mix.default_normalized_value()
        );
    }

    #[test]
    fn cancelling_a_successor_does_not_orphan_the_published_predecessor_identity() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        mark_loaded(&params, "Before", Origin::Factory);
        let published_response = publish_target_awaiting_callback(&params, &controller);
        assert_eq!(loaded(&params).name(), Some("Before"));
        assert!((params.mix.unmodulated_normalized_value() - 0.1).abs() < 1.0e-6);

        let mut successor_response = published_response.clone();
        successor_response.preparation.time_percent = 75;
        let mut successor = Preset::capture("Successor", Category::Fx, &params);
        successor.params.get_mut("mix").unwrap().v = 0.8;
        successor.state = Some(serde_json::to_value(successor_response).unwrap());
        let (successor_work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "Successor".to_owned(),
                    origin: Origin::User,
                    preset: successor,
                },
            )
            .unwrap();
        assert!(problems.is_empty());

        controller.cancel_unpublished(&params);
        controller.prepare(&params, successor_work);
        assert_eq!(params.response.snapshot(), published_response);
        assert!((params.mix.unmodulated_normalized_value() - 0.1).abs() < 1.0e-6);
        assert_eq!(loaded(&params).name(), Some("Before"));

        let predecessor = controller
            .acknowledge_process_boundary()
            .expect("the predecessor publication still reaches the callback");
        assert!(controller.acknowledge(predecessor, &params));
        assert_eq!(params.response.snapshot(), published_response);
        assert!((params.mix.unmodulated_normalized_value() - 0.1).abs() < 1.0e-6);
        assert_eq!(
            loaded(&params),
            Loaded::Clean {
                name: "Target".to_owned(),
                origin: Origin::User,
            }
        );
    }

    #[test]
    fn rejecting_a_successor_does_not_orphan_the_published_predecessor_identity() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        mark_loaded(&params, "Before", Origin::Factory);
        let published_response = publish_target_awaiting_callback(&params, &controller);
        assert_eq!(loaded(&params).name(), Some("Before"));

        let mut successor = Preset::capture("Rejected", Category::Fx, &params);
        successor.params.get_mut("mix").unwrap().v = 0.8;
        successor.state = Some(serde_json::json!({"not": "a response"}));
        let (successor_work, problems) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "Rejected".to_owned(),
                    origin: Origin::User,
                    preset: successor,
                },
            )
            .unwrap();
        assert!(problems.is_empty());
        controller.prepare(&params, successor_work);
        assert!(matches!(
            controller.status(),
            DeferredStatus::Rejected(_, _)
        ));
        assert_eq!(params.response.snapshot(), published_response);
        assert!((params.mix.unmodulated_normalized_value() - 0.1).abs() < 1.0e-6);
        assert_eq!(loaded(&params).name(), Some("Before"));

        let predecessor = controller
            .acknowledge_process_boundary()
            .expect("the predecessor publication still reaches the callback");
        assert!(controller.acknowledge(predecessor, &params));
        assert_eq!(params.response.snapshot(), published_response);
        assert!((params.mix.unmodulated_normalized_value() - 0.1).abs() < 1.0e-6);
        assert_eq!(
            loaded(&params),
            Loaded::Clean {
                name: "Target".to_owned(),
                origin: Origin::User,
            }
        );
    }

    #[test]
    fn newer_work_supersedes_old_completion_and_close_cancels_only_unpublished_work() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        let (old, _) = controller
            .begin(&params, DeferredPresetUiRequest::Init)
            .unwrap();
        let mut preset = Preset::capture("Latest", Category::Fx, &params);
        let mut latest_response = params.response.snapshot();
        latest_response.preparation.time_percent = 75;
        preset.state = Some(serde_json::to_value(&latest_response).unwrap());
        let (latest, _) = controller
            .begin(
                &params,
                DeferredPresetUiRequest::Load {
                    name: "Latest".to_owned(),
                    origin: Origin::Factory,
                    preset,
                },
            )
            .unwrap();

        controller.prepare(&params, old);
        assert!(matches!(controller.status(), DeferredStatus::Pending(_)));
        controller.prepare(&params, latest);
        assert!(matches!(controller.status(), DeferredStatus::Ready(_)));
        controller.cancel_unpublished(&params);
        assert_eq!(controller.status(), DeferredStatus::Idle);
        assert!(!controller.pending());
        assert_ne!(params.response.snapshot(), latest_response);
    }

    #[test]
    fn ordinary_model_and_wav_requests_share_the_deferred_cancellation_domain() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();

        let (model_superseded, _) = controller
            .begin(&params, DeferredPresetUiRequest::Init)
            .unwrap();
        let _model_request = controller.begin_response_edit(&params, "ordinary model");
        assert_eq!(controller.status(), DeferredStatus::Idle);
        controller.prepare(&params, model_superseded);
        assert_eq!(controller.status(), DeferredStatus::Idle);

        let (wav_superseded, _) = controller
            .begin(&params, DeferredPresetUiRequest::Init)
            .unwrap();
        let _wav_request =
            controller.begin_wav_load(&params, std::path::Path::new("replacement.wav"));
        assert_eq!(controller.status(), DeferredStatus::Idle);
        controller.prepare(&params, wav_superseded);
        assert_eq!(controller.status(), DeferredStatus::Idle);
    }

    #[test]
    fn a_response_request_outside_the_controller_cannot_strand_pending() {
        let params = MxmFxConvolutionParams::default();
        let controller = DeferredPresetController::default();
        let (superseded, _) = controller
            .begin(&params, DeferredPresetUiRequest::Init)
            .unwrap();

        let _ordinary_request = params.response.begin_edit("external response edit");
        controller.prepare(&params, superseded);
        assert_eq!(controller.status(), DeferredStatus::Idle);
        assert!(!controller.pending());
    }
}
