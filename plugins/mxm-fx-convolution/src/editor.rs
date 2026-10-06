//! Dynamically paged production editor for `mxm-fx-convolution`.
//!
//! Four stable Effects cards follow the response pipeline rather than any hardware face: Response,
//! Time, Shape and Output. Revision 7 adds Feedback to the Revision 6 preparation model and live
//! controls without changing those stable card identities or borrowing a hardware face.

pub mod binding;
pub mod sections;

#[cfg(test)]
mod proof;

use std::collections::HashMap;
use std::sync::Arc;

use egui::Ui;
use mxm_ui::space::SPACE_5;
use mxm_ui::theme::Tokens;
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use nice_plug_egui::{EguiEditorState, NiceEguiApp, create_egui_editor};

use crate::deferred::DeferredPresetController;
use crate::params::MxmFxConvolutionParams;
use crate::response::LoadState;
use crate::telemetry::Telemetry;
use crate::{EditorTask, MxmFxConvolution};

/// The label an edit on the panel loads under, which its status line names in words.
pub(crate) const RESPONSE_EDIT: &str = "response change";

const REFERENCE: (u32, u32) = (951, 555);
const MINIMUM: (u32, u32) = (416, 320);

pub type MxmFxConvolutionEditor = nice_plug_egui::EguiEditor<MxmFxConvolutionApp>;
pub use mxm_preset::PresetUi;

pub fn create(
    params: Arc<MxmFxConvolutionParams>,
    telemetry: Arc<Telemetry>,
    deferred_presets: Arc<DeferredPresetController>,
    executor: AsyncExecutor<MxmFxConvolution>,
) -> Option<MxmFxConvolutionEditor> {
    let state = EguiEditorState::from_size(
        nice_plug::editor::dpi::LogicalSize::new(REFERENCE.0, REFERENCE.1),
        1.0,
    );
    create_egui_editor(
        state,
        nice_plug_egui::RepaintNotifier::new(),
        nice_plug_egui::EguiNiceSettings {
            title: crate::NAME.to_owned(),
            resize_hint: ResizeHint {
                size_constraints: nice_plug::editor::SizeConstraints::min_logical_size(
                    nice_plug::editor::dpi::LogicalSize::new(MINIMUM.0 as f32, MINIMUM.1 as f32),
                ),
                ..ResizeHint::RESIZABLE
            },
            ..Default::default()
        },
        MxmFxConvolutionApp::new(params, telemetry, deferred_presets, executor),
    )
}

pub struct MxmFxConvolutionApp {
    params: Arc<MxmFxConvolutionParams>,
    telemetry: Arc<Telemetry>,
    deferred_presets: Arc<DeferredPresetController>,
    executor: AsyncExecutor<MxmFxConvolution>,
    gui_context: Option<GuiContext>,
    display_revision: u64,
    response_view: sections::ResponseView,
    text_entry: HashMap<&'static str, Option<String>>,
    presets: PresetUi,
    nav: mxm_ui::navigation::State,
    /// The collection's impulses folder, looked up once rather than on every Browse.
    impulses: Option<std::path::PathBuf>,
}

impl MxmFxConvolutionApp {
    pub fn new(
        params: Arc<MxmFxConvolutionParams>,
        telemetry: Arc<Telemetry>,
        deferred_presets: Arc<DeferredPresetController>,
        executor: AsyncExecutor<MxmFxConvolution>,
    ) -> Self {
        // **The impulses folder is the factory set**, scanned each time the editor opens, so a file
        // added to the folder is there the next time (`crate::impulses::scan`).
        let impulses = crate::impulses::root();
        let library = mxm_preset::Library::at_config_dir(crate::CLAP_ID)
            .with_found(crate::impulses::scan(impulses.as_deref(), &params));
        let presets = PresetUi::at(library, params.as_ref());
        let display_revision = params.response.revision();
        let response_view = sections::ResponseView::read(&params);
        Self {
            params,
            telemetry,
            deferred_presets,
            executor,
            gui_context: None,
            display_revision,
            response_view,
            text_entry: HashMap::new(),
            presets,
            nav: mxm_ui::navigation::State::default(),
            impulses,
        }
    }
}

impl NiceEguiApp for MxmFxConvolutionApp {
    fn build(
        &mut self,
        context: egui::Context,
        gui_context: GuiContext,
        _frame: &mut nice_plug_egui::Frame,
    ) -> Result<(), nice_plug_egui::baseview::HandlerError> {
        mxm_ui::theme::apply(&context);
        mxm_ui::typography::apply(&context);
        context.set_theme(mxm_ui::theme::preference());
        self.params
            .response
            .connect_context(Some(gui_context.clone()));
        if self
            .deferred_presets
            .service(&self.params, &gui_context.param_setter())
        {
            self.executor.execute_gui(EditorTask::Wake);
        }
        self.gui_context = Some(gui_context);
        // Adopt a background completion that arrived while no editor existed. Ready is consumed
        // exactly once because a successful commit changes it to Idle synchronously.
        commit_ready(&self.params);
        self.display_revision = self.params.response.revision();
        self.response_view = sections::ResponseView::read(&self.params);
        Ok(())
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut nice_plug_egui::Frame) {
        let Some(context) = self.gui_context.clone() else {
            return;
        };
        // A published preset is acknowledged by the next process callback, so wake the audio
        // side for it: a host may have put an idle effect to sleep (`DeferredPresetController::service`).
        if self
            .deferred_presets
            .service(&self.params, &context.param_setter())
        {
            self.executor.execute_gui(EditorTask::Wake);
        }
        commit_ready(&self.params);
        let revision = self.params.response.revision();
        if revision != self.display_revision {
            self.display_revision = revision;
            self.response_view = sections::ResponseView::read(&self.params);
        }
        let params = Arc::clone(&self.params);
        let deferred_presets = Arc::clone(&self.deferred_presets);
        let loader = self.executor.clone();
        let mut load = move |path: std::path::PathBuf| {
            let request = deferred_presets.begin_wav_load(&params, &path);
            loader.execute_background(EditorTask::LoadWav { request, path });
        };
        // Browse's dialog finishes on its own thread (`sections::browse_picker`); its pick loads here.
        if let Some(path) =
            mxm_ui::offthread::take::<std::path::PathBuf>(ui.ctx(), sections::browse_picker())
        {
            load(path);
        }
        let params = Arc::clone(&self.params);
        let deferred_presets = Arc::clone(&self.deferred_presets);
        let editor = self.executor.clone();
        let mut edit = move |model: crate::response::ResponseModel| {
            let request = deferred_presets.begin_response_edit(&params, RESPONSE_EDIT);
            editor.execute_background(EditorTask::PrepareResponse { request, model });
        };
        let mut actions = sections::ResponseActions {
            load: &mut load,
            edit: &mut edit,
            browse_from: self.impulses.as_deref(),
        };
        let deferred_presets = Arc::clone(&self.deferred_presets);
        let deferred_params = Arc::clone(&self.params);
        let preset_executor = self.executor.clone();
        let mut preset_request = move |request| {
            let (work, problems) = deferred_presets.begin(&deferred_params, request)?;
            preset_executor.execute_background(EditorTask::PreparePreset { work });
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems.join("; "))
            }
        };
        let mut deferred_ui = mxm_preset::DeferredPresetUi {
            pending: self.deferred_presets.pending(),
            request: &mut preset_request,
        };
        panel_with_actions(
            ui,
            &self.params,
            &self.telemetry,
            &context.param_setter(),
            &mut self.text_entry,
            &mut self.presets,
            &mut self.nav,
            &self.response_view,
            Some(&mut actions),
            Some(&mut deferred_ui),
        );
    }

    fn editor_closed(&mut self) {
        self.deferred_presets.cancel_unpublished(&self.params);
        // A response prepared just before close is committed whole rather than stranded. The old
        // response remains selected if preparation failed.
        if matches!(self.params.response.status(), LoadState::Ready(_))
            && let Err(error) = self.params.response.commit()
        {
            self.params.response.fail(error);
        }
        self.params.response.connect_context(None);
        self.gui_context = None;
    }
}

fn commit_ready(params: &MxmFxConvolutionParams) {
    if matches!(params.response.status(), LoadState::Ready(_))
        && let Err(error) = params.response.commit()
    {
        params.response.fail(error);
    }
}

pub fn panel(
    ui: &mut Ui,
    params: &MxmFxConvolutionParams,
    telemetry: &Telemetry,
    setter: &ParamSetter<'_>,
    text_entry: &mut HashMap<&'static str, Option<String>>,
    presets: &mut PresetUi,
    nav: &mut mxm_ui::navigation::State,
) {
    // Test/lab callers render the complete production surface with inert acquisition callbacks;
    // only `MxmFxConvolutionApp` supplies the background executor.
    let mut load = |_path: std::path::PathBuf| {};
    let mut edit = |_model: crate::response::ResponseModel| {};
    let mut actions = sections::ResponseActions {
        load: &mut load,
        edit: &mut edit,
        browse_from: None,
    };
    let response_view = sections::ResponseView::read(params);
    panel_with_actions(
        ui,
        params,
        telemetry,
        setter,
        text_entry,
        presets,
        nav,
        &response_view,
        Some(&mut actions),
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn panel_with_actions(
    ui: &mut Ui,
    params: &MxmFxConvolutionParams,
    telemetry: &Telemetry,
    setter: &ParamSetter<'_>,
    text_entry: &mut HashMap<&'static str, Option<String>>,
    presets: &mut PresetUi,
    nav: &mut mxm_ui::navigation::State,
    response: &sections::ResponseView,
    actions: Option<&mut sections::ResponseActions<'_>>,
    mut deferred_presets: Option<&mut mxm_preset::DeferredPresetUi<'_>>,
) {
    let tokens = tokens_for(ui);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(50));

    // Destructive telemetry is sampled exactly once, before the paging renderer may perform any
    // hidden measurement passes. Measurement receives this inert copy and cannot consume audio data.
    let meters = sections::MeterView {
        peak: telemetry.take_peak(),
        tail_samples: telemetry.tail_samples(),
        numeric_fault: telemetry.numeric_fault(),
        response_rejection: telemetry.response_rejection(),
        tempo: telemetry.tempo.get(),
    };
    let clipped = telemetry.clipped();
    let busy = presets.holds_the_keyboard() || text_entry.values().any(Option::is_some);
    mxm_ui::paging::editor::hold(ui.ctx(), busy);
    mxm_ui::navigation::paged(ui.ctx(), nav, busy);

    mxm_ui::AppBar::new(crate::NAME).show_with(
        ui,
        &tokens,
        |ui| {
            if let Some(deferred) = deferred_presets.as_deref_mut() {
                mxm_preset::ui::preset_row_deferred(ui, &tokens, params, setter, presets, deferred);
            } else {
                mxm_preset::ui::preset_row(ui, &tokens, params, setter, presets);
            }
        },
        |ui| {
            if mxm_ui::shell::level_meter(ui, &tokens, meters.peak, clipped) {
                telemetry.clear_clip();
            }
            mxm_ui::shell::zoom_control(ui);
            mxm_ui::shell::editor_theme_control(ui);
        },
    );
    if let Some(deferred) = deferred_presets {
        mxm_preset::ui::overlays_deferred(ui, &tokens, params, setter, presets, deferred);
    } else {
        mxm_preset::ui::overlays(ui, &tokens, params, setter, presets);
    }

    let mut clear_fault = false;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(tokens.canvas)
                .inner_margin(egui::Margin::same(SPACE_5 as i8)),
        )
        .show(ui, |ui| {
            sections::cards(
                ui,
                &tokens,
                params,
                setter,
                text_entry,
                response,
                meters,
                &mut clear_fault,
                actions,
            );
        });
    if clear_fault {
        telemetry.clear_numeric_fault();
    }
}

fn tokens_for(ui: &Ui) -> Tokens {
    if ui.visuals().dark_mode {
        mxm_ui::DARK
    } else {
        mxm_ui::LIGHT
    }
}

#[cfg(test)]
mod tests {
    #![allow(dead_code)]

    use super::*;
    use nice_plug::params::internals::ParamPtr;
    use nice_plug::prelude::{PluginApi, PluginState};

    struct NoHost;

    impl nice_plug::context::gui::GuiContextInner for NoHost {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, _param: ParamPtr, _normalized: f32) {}
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

    use mxm_plugin_test::opening_size;

    #[test]
    fn reopening_adopts_ready_work_even_when_revision_baseline_already_matches() {
        let params = MxmFxConvolutionParams::default();
        let mut model = params.response.display().model;
        model.preparation.reverse = true;
        let request = params.response.begin_edit("delayed");
        // The editor closes while Loading; its background worker completes afterward.
        params.response.connect_context(None);
        params.response.prepare_edit(request, model).unwrap();
        let reopened_revision_baseline = params.response.revision();
        assert!(matches!(params.response.status(), LoadState::Ready(_)));

        // A reconstructed app starts with this same baseline. Reconnection adopts Ready directly
        // rather than waiting for a revision edge that can never occur.
        commit_ready(&params);
        assert_eq!(params.response.revision(), reopened_revision_baseline + 1);
        assert_eq!(params.response.snapshot().preparation, model.preparation);
        assert!(matches!(params.response.status(), LoadState::Idle));
    }

    #[test]
    fn the_minimum_holds_the_widest_card() {
        assert!(MINIMUM.0 as f32 >= sections::minimum_card_width() + 2.0 * SPACE_5);
    }

    #[test]
    fn the_opening_frame_is_inside_the_quarter_4k_budget() {
        assert!(REFERENCE.0 <= 1920 && REFERENCE.1 <= 1080);
    }

    #[test]
    fn the_opening_size_is_the_budget_hugged() {
        let params = MxmFxConvolutionParams::default();
        let telemetry = Telemetry::default();
        let host = NoHost;
        let setter = ParamSetter::new(&host);
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(mxm_preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        opening_size::is_the_budget_hugged(
            egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            &|_| {},
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &setter,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                );
            },
        );
    }

    /// **The app bar holds in the narrowest window**: its `…` menu whole and nothing drawn over
    /// anything else, from `MINIMUM` up (`opening_size::bar_holds_from_the_minimum`).
    #[test]
    fn the_app_bar_holds_in_the_minimum_window() {
        let params = MxmFxConvolutionParams::default();
        let telemetry = Telemetry::default();
        let host = NoHost;
        let setter = ParamSetter::new(&host);
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(mxm_preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        opening_size::bar_holds_from_the_minimum(
            egui::vec2(MINIMUM.0 as f32, MINIMUM.1 as f32),
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &setter,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                );
            },
        );
    }
}
