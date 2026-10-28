//! The shared preset-system seam.
//!
//! No factory set is compiled in: the factory presets are the impulse files in the collection's
//! folder, found when the editor opens (`crate::impulses`), each carrying only where its file is.
//! User presets carry all nine live parameters plus the complete immutable source and reversible
//! model through the shared transaction seams, so they load anywhere.

use crate::params::MxmFxConvolutionParams;
use mxm_preset::{Instrument, PresetIdentity};
use std::sync::RwLock;

pub use mxm_preset::{Category, INIT_NAME, Preset, Value, factory};

impl Instrument for MxmFxConvolutionParams {
    fn clap_id(&self) -> &'static str {
        crate::CLAP_ID
    }

    /// The plugin's version, its `Plugin::VERSION`: its factory presets are installed for it.
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn parameters(&self) -> Vec<(&'static str, &dyn mxm_params::ErasedParam)> {
        vec![
            ("mix", &self.mix),
            ("predelay", &self.pre_delay),
            ("lowcut", &self.low_cut),
            ("highcut", &self.high_cut),
            ("tone", &self.tone),
            ("width", &self.width),
            ("modulation", &self.modulation),
            ("feedback", &self.feedback),
            ("predelaysync", &self.pre_delay_sync),
        ]
    }

    fn parameter_edit_revision(&self, id: &str) -> Option<u64> {
        MxmFxConvolutionParams::parameter_edit_revision(self, id)
    }

    fn default_missing_legacy_parameter(&self, id: &str) -> bool {
        id == "feedback" || id == "predelaysync"
    }

    fn identity(&self) -> &RwLock<PresetIdentity> {
        &self.preset
    }

    fn capture_preset_state(&self) -> Option<serde_json::Value> {
        Some(
            serde_json::to_value(self.response.snapshot())
                .expect("ResponseState serialization is infallible"),
        )
    }

    fn preset_state_fingerprint(&self) -> Option<u64> {
        Some(self.response.map(crate::response::fingerprint))
    }

    fn validate_preset_state(&self, state: Option<&serde_json::Value>) -> Result<(), String> {
        // A room from the impulses folder names its file; it is read when it is applied.
        if let Some(state) = state
            && crate::impulses::reference(state).is_some()
        {
            return Ok(());
        }
        if let Some(state) = state {
            let candidate: crate::response::ResponseState =
                serde_json::from_value(state.clone())
                    .map_err(|error| format!("invalid embedded response: {error}"))?;
            crate::response::validate(&candidate)
                .map_err(|error| format!("invalid embedded response: {error:?}"))
        } else {
            Ok(())
        }
    }

    fn apply_preset_state(&self, state: Option<&serde_json::Value>) -> Result<(), String> {
        if let Some(state) = state {
            // The synchronous path is the host's own and runs in production only, so it reads the
            // installed folder; the editor's path goes through `DeferredPresetController`, whose
            // folder is injected.
            let candidate: crate::response::ResponseState = match crate::impulses::reference(state)
            {
                Some(relative) => {
                    crate::impulses::load(crate::impulses::root().as_deref(), relative)?
                }
                None => serde_json::from_value(state.clone())
                    .map_err(|error| format!("invalid embedded response: {error}"))?,
            };
            self.response.stage(candidate)
        } else {
            Ok(())
        }
    }

    fn commit_preset_state(&self) {
        if let Err(error) = self.response.commit() {
            self.response.fail(error);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mxm_preset::{DeferredContent, DeferredPresetTransaction, DeferredStatus, Origin};
    use nice_plug::params::InternalParamMut;

    use nice_plug::prelude::{Param, ParamSetter};

    use mxm_plugin_test::NoHost;

    /// **Every stored `text` is this plugin's own formatting of the `v` beside it**
    /// (`mxm_preset::stale_factory_texts`).
    #[test]
    fn every_factory_text_is_this_plugins_formatting_of_its_value() {
        let stale = mxm_preset::stale_factory_texts(&MxmFxConvolutionParams::default());
        assert!(
            stale.is_empty(),
            "{} stale texts — regenerate the files:\n{}",
            stale.len(),
            stale.join("\n")
        );
    }

    #[test]
    fn user_preset_state_round_trips_the_complete_response_through_stage_then_commit() {
        let source = MxmFxConvolutionParams::default();
        let mut expected = source.response.snapshot();
        expected.name = "User response".to_owned();
        expected.preparation.reverse = true;
        expected.preparation.time_percent = 75;
        nice_plug::params::persist::PersistentField::set(&source.response, expected.clone());
        let state = source
            .capture_preset_state()
            .expect("source-bearing preset state");
        let fingerprint = source.preset_state_fingerprint();

        let restored = MxmFxConvolutionParams::default();
        restored.validate_preset_state(Some(&state)).unwrap();
        restored.apply_preset_state(Some(&state)).unwrap();
        assert_ne!(
            restored.response.snapshot(),
            expected,
            "prepare published early"
        );
        restored.commit_preset_state();
        assert_eq!(restored.response.snapshot(), expected);
        assert_eq!(restored.preset_state_fingerprint(), fingerprint);
    }

    #[test]
    fn feedback_round_trips_in_new_user_presets() {
        let params = MxmFxConvolutionParams::default();
        let expected = params.feedback.preview_normalized(1.1);
        unsafe {
            params.feedback._internal_set_normalized_value(expected);
        }
        let captured = Preset::capture("Recursive", Category::Fx, &params);
        let parsed = Preset::parse(&captured.to_json(), crate::CLAP_ID).unwrap();
        assert_eq!(parsed.params.len(), 9);
        assert_eq!(parsed.params["feedback"].v, expected);
        assert!(parsed.resolve(&params).1.is_empty());
    }

    #[test]
    fn deferred_init_overlay_preserves_source_and_identity_until_commit_acknowledgement() {
        let params = MxmFxConvolutionParams::default();
        let before = params.response.snapshot();
        mxm_preset::mark_loaded(&params, "Before", Origin::Factory);
        let mut transaction = DeferredPresetTransaction::new();
        let request = transaction
            .begin_init(
                &params,
                serde_json::to_value(crate::response::PreparationOverlay::init()).unwrap(),
            )
            .unwrap();
        let overlay = match &request.content {
            DeferredContent::InitMergeWithCommittedSource(value) => {
                serde_json::from_value::<crate::response::PreparationOverlay>(value.clone())
                    .unwrap()
            }
            other => panic!("wrong deferred operation: {other:?}"),
        };
        params.response.stage_overlay(&overlay).unwrap();
        assert_eq!(
            params.response.snapshot(),
            before,
            "preparation published early"
        );
        assert!(transaction.ready(request.id));
        let host = NoHost;
        let setter = ParamSetter::new(&host);
        let mut queued = None;
        assert!(transaction.publish_ready(&params, &setter, |id| queued = Some(id)));
        assert_eq!(queued, Some(request.id));
        assert!(matches!(
            transaction.status(),
            DeferredStatus::AwaitingProcessCommit(id) if id == request.id
        ));
        assert_eq!(mxm_preset::loaded(&params).name(), Some("Before"));
        params.response.commit().unwrap();
        assert!(transaction.acknowledge(request.id, &params));
        let after = params.response.snapshot();
        assert_eq!(after.name, before.name);
        assert_eq!(after.interpretation, before.interpretation);
        assert_eq!(after.channels, before.channels);
        assert_eq!(after.preparation, crate::response::Preparation::default());
        assert_eq!(mxm_preset::loaded(&params), mxm_preset::Loaded::None);
    }

    #[test]
    fn init_is_generated_from_all_parameter_defaults() {
        let params = MxmFxConvolutionParams::default();
        let init = Preset::init(&params);
        assert_eq!(init.name, INIT_NAME);
        assert_eq!(init.params.len(), 9);
        for (id, parameter) in params.parameters() {
            assert_eq!(init.params[id].v, parameter.default_normalised(), "{id}");
        }
    }

    /// **Without the impulses folder the factory list is Init alone**: nothing is compiled in, and
    /// the rooms come from the folder (`crate::impulses::scan`).
    #[test]
    fn the_compiled_factory_set_is_empty_and_init_stands_alone() {
        let params = MxmFxConvolutionParams::default();
        // No preset files ship: the factory presets are the impulse files in the collection's
        // folder, found when the editor opens (`crate::impulses::scan`, the owner, 2026-09-28).
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
        assert!(!folder.exists());
        let all = factory(&params);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, INIT_NAME);
    }

    #[test]
    fn control_map_uses_the_permanent_identity_and_one_existing_role() {
        let map: serde_json::Value =
            serde_json::from_str(include_str!("../control-map.json")).expect("control map JSON");
        assert_eq!(map["schema_version"], 1);
        let instruments = map["instruments"].as_array().expect("instrument list");
        assert_eq!(instruments.len(), 1);
        assert_eq!(instruments[0]["clap_id"], crate::CLAP_ID);
        assert_eq!(instruments[0]["name"], crate::NAME);
        assert_eq!(instruments[0]["params"]["fx.reverb"], "mix");
        assert_eq!(
            instruments[0]["params"]
                .as_object()
                .expect("role map")
                .len(),
            1
        );
    }
}
