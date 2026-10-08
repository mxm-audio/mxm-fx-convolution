//! Automated proof for the production editor: responsive geometry, accessibility, private state,
//! keyboard coverage and balanced parameter gestures. Native-window quality remains a manual gate.

#![allow(dead_code)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use egui::{Rect, ThemePreference, vec2};
use kittest::{NodeT, Queryable};
use nice_plug::{
    params::internals::ParamPtr,
    prelude::{ParamSetter, PluginApi, PluginState},
};

use super::*;

#[derive(Default)]
struct ApplyingHost(Mutex<Vec<(&'static str, String)>>);

impl ApplyingHost {
    fn record(&self, action: &'static str, param: ParamPtr) {
        self.0
            .lock()
            .unwrap()
            .push((action, unsafe { param.name() }.to_owned()));
    }

    fn assert_gesture(&self, name: &str) {
        assert_eq!(
            *self.0.lock().unwrap(),
            ["begin", "set", "end"].map(|action| (action, name.to_owned()))
        );
        self.0.lock().unwrap().clear();
    }
}

impl nice_plug::context::gui::GuiContextInner for ApplyingHost {
    // A test double has no host to ask for a restart (nice-plug 0.4).
    fn request_restart(&self) {}
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        self.record("begin", param);
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, value: f32) {
        self.record("set", param);
        unsafe {
            let _ = param._internal_set_normalized_value(value);
        }
    }

    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        self.record("end", param);
    }

    fn get_state(&self) -> PluginState {
        PluginState {
            version: String::new(),
            params: Default::default(),
            fields: Default::default(),
        }
    }

    fn set_state(&self, _state: PluginState) {}
}

use mxm_plugin_test::keyboard_checks;
use mxm_plugin_test::keyboard_checks::{OUT, VALUE, key_of};
use mxm_plugin_test::paging_checks;

fn panel_state() -> (
    MxmFxConvolutionParams,
    Telemetry,
    ApplyingHost,
    HashMap<&'static str, Option<String>>,
    PresetUi,
    mxm_ui::navigation::State,
) {
    let params = MxmFxConvolutionParams::default();
    let presets = PresetUi::at(mxm_preset::Library::at(None), &params);
    (
        params,
        Telemetry::default(),
        ApplyingHost::default(),
        HashMap::new(),
        presets,
        mxm_ui::navigation::State::default(),
    )
}

fn render_layout(width: f32) -> Vec<Rect> {
    let (params, telemetry, host, mut entries, mut presets, mut nav) = panel_state();
    let setter = ParamSetter::new(&host);
    let ctx = egui::Context::default();
    mxm_ui::typography::apply(&ctx);
    mxm_ui::theme::apply(&ctx);
    ctx.set_theme(ThemePreference::Light);
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 2_000.0))),
        ..Default::default()
    };
    for _ in 0..4 {
        let mut output = ctx.run_ui(input.clone(), |ui| {
            panel(
                ui,
                &params,
                &telemetry,
                &setter,
                &mut entries,
                &mut presets,
                &mut nav,
            );
        });
        output.textures_delta.clear();
    }
    assert!(host.0.lock().unwrap().is_empty());
    paging_checks::all_rects(&ctx, sections::test_items().len())
}

#[test]
fn repeated_ten_second_stereo_paints_reuse_the_bounded_display_cache() {
    let (params, telemetry, host, mut entries, mut presets, mut nav) = panel_state();
    nice_plug::params::persist::PersistentField::set(
        &params.response,
        crate::response::maximum_display_state_for_test(),
    );
    assert!(!params.response.take_preparation_rejected());
    let builds = params.response.display_builds_for_test();
    let setter = ParamSetter::new(&host);
    let ctx = egui::Context::default();
    mxm_ui::typography::apply(&ctx);
    mxm_ui::theme::apply(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1_736.0, 398.0))),
        ..Default::default()
    };
    for _ in 0..20 {
        let mut output = ctx.run_ui(input.clone(), |ui| {
            panel(
                ui,
                &params,
                &telemetry,
                &setter,
                &mut entries,
                &mut presets,
                &mut nav,
            );
        });
        output.textures_delta.clear();
    }
    assert_eq!(params.response.display_builds_for_test(), builds);
    assert!(host.0.lock().unwrap().is_empty());
}

fn rows(rects: &[Rect]) -> Vec<Vec<(usize, Rect)>> {
    let mut ordered: Vec<_> = rects.iter().copied().enumerate().collect();
    ordered.sort_by(|(_, left), (_, right)| {
        left.top()
            .total_cmp(&right.top())
            .then(left.left().total_cmp(&right.left()))
    });
    let mut rows: Vec<Vec<(usize, Rect)>> = Vec::new();
    for item in ordered {
        match rows.last_mut() {
            Some(row)
                if row
                    .iter()
                    .any(|(_, rect)| rect.bottom() > item.1.top() + 1.0) =>
            {
                row.push(item);
            }
            _ => rows.push(vec![item]),
        }
    }
    rows
}

fn assert_geometry(rects: &[Rect]) {
    let items = sections::test_items();
    assert_eq!(rects.len(), items.len());
    for (rect, item) in rects.iter().zip(&items) {
        assert!(
            rect.width() >= item.card.floor - 0.75,
            "{} below its floor: {rect:?}",
            item.card.title
        );
        assert!(
            rect.width() <= item.card.ceiling.unwrap() + 0.75,
            "{} exceeded its ceiling: {rect:?}",
            item.card.title
        );
    }
    for (index, first) in rects.iter().enumerate() {
        for second in rects.iter().skip(index + 1) {
            let overlap = first.intersect(*second);
            assert!(
                overlap.width() <= 0.75 || overlap.height() <= 0.75,
                "cards overlap: {first:?}, {second:?}"
            );
        }
    }
    for row in rows(rects) {
        if row.len() > 1 {
            let low = row
                .iter()
                .map(|(_, rect)| rect.bottom())
                .fold(f32::INFINITY, f32::min);
            let high = row
                .iter()
                .map(|(_, rect)| rect.bottom())
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(high - low < 1.0, "row bottoms differ: {row:?}");
        }
    }
    assert_eq!(
        rows(rects)
            .into_iter()
            .flatten()
            .map(|(index, _)| index)
            .collect::<Vec<_>>(),
        (0..items.len()).collect::<Vec<_>>(),
        "reflow changed Response → Time → Shape → Output"
    );
}

#[test]
fn narrow_default_and_wide_reflow_keep_floors_order_alignment_and_no_overlap() {
    for width in [MINIMUM.0 as f32, REFERENCE.0 as f32, 2_200.0] {
        assert_geometry(&render_layout(width));
    }
}

#[test]
fn preferred_response_and_time_group_stays_together_when_it_fits() {
    let rects = render_layout(REFERENCE.0 as f32);
    assert!(
        (rects[0].top() - rects[1].top()).abs() < 1.0,
        "Response and Time split despite fitting: {:?}, {:?}",
        rects[0],
        rects[1]
    );
}

#[test]
fn lone_cards_never_stretch_and_the_narrow_flow_stays_inside_the_workspace() {
    let rects = render_layout(MINIMUM.0 as f32);
    let items = sections::test_items();
    for row in rows(&rects) {
        if row.len() == 1 {
            let (index, rect) = row[0];
            assert!(
                rect.width() <= items[index].card.ceiling.unwrap() + 0.75,
                "a lone card stretched: {row:?}"
            );
        }
    }
    for rect in rects {
        assert!(rect.left() >= SPACE_5 - 0.75, "card left the workspace");
        assert!(
            rect.right() <= MINIMUM.0 as f32 - SPACE_5 + 0.75,
            "card clipped at the narrow edge: {rect:?}"
        );
    }
}

#[test]
fn every_dynamic_page_is_reachable_at_narrow_default_wide_and_two_x() {
    let (params, telemetry, host, mut entries, mut presets, mut nav) = panel_state();
    paging_checks::verify(
        &sections::test_items(),
        &[
            vec2(MINIMUM.0 as f32, MINIMUM.1 as f32),
            vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            vec2(1_880.0, 1_040.0),
        ],
        |ui| {
            panel(
                ui,
                &params,
                &telemetry,
                &ParamSetter::new(&host),
                &mut entries,
                &mut presets,
                &mut nav,
            );
        },
    );
    assert!(host.0.lock().unwrap().is_empty());
}

fn editor_harness(
    params: Arc<MxmFxConvolutionParams>,
    telemetry: Arc<Telemetry>,
    host: Arc<ApplyingHost>,
    size: egui::Vec2,
) -> egui_kittest::Harness<'static> {
    let params = Box::leak(Box::new(params));
    let telemetry = Box::leak(Box::new(telemetry));
    let host = Box::leak(Box::new(host));
    let entries = Box::leak(Box::new(HashMap::new()));
    let presets = Box::leak(Box::new(PresetUi::at(
        mxm_preset::Library::at(None),
        params.as_ref(),
    )));
    let nav = Box::leak(Box::new(mxm_ui::navigation::State::default()));
    egui_kittest::Harness::builder()
        .with_size(size)
        .build_ui(move |ui| {
            mxm_ui::typography::apply(ui.ctx());
            mxm_ui::theme::apply(ui.ctx());
            panel(
                ui,
                params.as_ref(),
                telemetry.as_ref(),
                &ParamSetter::new(host.as_ref()),
                entries,
                presets,
                nav,
            );
        })
}

#[test]
fn private_preset_browser_state_never_emits_an_audio_edit() {
    let params = Arc::new(MxmFxConvolutionParams::default());
    let telemetry = Arc::new(Telemetry::default());
    let host = Arc::new(ApplyingHost::default());
    let mut harness = editor_harness(params, telemetry, Arc::clone(&host), vec2(900.0, 600.0));
    harness.run_steps(8);
    assert!(
        harness
            .query_all_by_label_contains("Scale:")
            .next()
            .is_some(),
        "fixed user zoom is absent from accessibility"
    );
    assert!(
        harness
            .query_all_by_label_contains("Theme:")
            .next()
            .is_some(),
        "theme choice is absent from accessibility"
    );
    assert!(harness.query_by_label("Banks").is_none());
    harness.get_by_label("No preset").click();
    harness.run_steps(3);
    for label in ["Banks", "Categories", "Factory", "My presets", "Close"] {
        harness.get_by_label(label);
    }
    harness.get_by_label("Close").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Banks").is_none());
    assert!(host.0.lock().unwrap().is_empty());
}

#[test]
fn every_control_and_semantic_view_is_accessible_in_both_themes_at_fixed_physical_scales() {
    let fixed_physical = vec2(1_880.0, 1_040.0);
    let checks = [
        (0, "Response energy over ten seconds and active tail energy"),
        (0, "Response loaded"),
        (1, "Pre-delay"),
        (1, "Size"),
        (1, "Decay"),
        (2, "Damping"),
        (2, "Feedback"),
        (2, "Low cut"),
        (2, "High cut"),
        (2, "Tone"),
        (2, "Modulation"),
        (2, "Linear response shape from source to convolution"),
        (3, "Width"),
        (3, "Mix"),
    ];
    for theme in [ThemePreference::Light, ThemePreference::Dark] {
        for scale in [1.0, 1.5, 2.0] {
            let params = Arc::new(MxmFxConvolutionParams::default());
            let telemetry = Arc::new(Telemetry::default());
            let host = Arc::new(ApplyingHost::default());
            let mut harness =
                editor_harness(params, telemetry, Arc::clone(&host), fixed_physical / scale);
            harness.ctx.set_theme(theme);
            harness.ctx.set_pixels_per_point(scale);
            harness.run_steps(12);
            for (card_index, label) in checks {
                mxm_ui::paging::editor::request_card(&harness.ctx, mxm_ui::paging::Key(card_index));
                harness.run_steps(4);
                let report = mxm_ui::paging::editor::report(&harness.ctx).unwrap();
                let card = report
                    .visible
                    .iter()
                    .find(|(key, _)| key.0 == card_index)
                    .expect("requested card visible")
                    .1;
                let node = harness
                    .query_all_by_label_contains(label)
                    .find(|node| card.contains_rect(node.rect()))
                    .unwrap_or_else(|| panic!("{label:?} absent at {theme:?}, {scale}x"));
                assert!(
                    card.contains_rect(node.rect()),
                    "{label:?} clips its card at {theme:?}, {scale}x"
                );
            }
            assert_eq!(harness.ctx.pixels_per_point(), scale);
            assert!(host.0.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn all_eight_live_knobs_emit_exactly_one_balanced_host_gesture() {
    for (card, name) in [
        (1, "Pre-delay"),
        (2, "Low cut"),
        (2, "High cut"),
        (2, "Tone"),
        (2, "Modulation"),
        (2, "Feedback"),
        (3, "Width"),
        (3, "Mix"),
    ] {
        // One harness per card keeps AccessKit focus from naming a node removed by a page change.
        let params = Arc::new(MxmFxConvolutionParams::default());
        let telemetry = Arc::new(Telemetry::default());
        let host = Arc::new(ApplyingHost::default());
        let mut harness = editor_harness(params, telemetry, Arc::clone(&host), vec2(900.0, 600.0));
        mxm_ui::paging::editor::request_card(&harness.ctx, mxm_ui::paging::Key(card));
        harness.run_steps(8);
        let control = harness.get_by_label(name);
        control.focus();
        // VALUE + ↑, kept with OUT.
        for key in [key_of(VALUE), egui::Key::ArrowUp, key_of(OUT)] {
            harness.key_press(key);
        }
        harness.run_steps(2);
        host.assert_gesture(name);
    }
}

#[test]
fn preparation_drag_submits_only_the_released_latest_value_without_host_gestures() {
    let params = Box::leak(Box::new(Arc::new(MxmFxConvolutionParams::default())));
    let telemetry = Box::leak(Box::new(Arc::new(Telemetry::default())));
    let host = Box::leak(Box::new(Arc::new(ApplyingHost::default())));
    let host_assert = Arc::clone(host);
    let edits = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&edits);
    let entries = Box::leak(Box::new(HashMap::new()));
    let presets = Box::leak(Box::new(PresetUi::at(
        mxm_preset::Library::at(None),
        params.as_ref(),
    )));
    let nav = Box::leak(Box::new(mxm_ui::navigation::State::default()));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(vec2(900.0, 600.0))
        .build_ui(move |ui| {
            mxm_ui::typography::apply(ui.ctx());
            mxm_ui::theme::apply(ui.ctx());
            let response = sections::ResponseView::read(params.as_ref());
            let mut load = |_path| {};
            let captured = Arc::clone(&captured);
            let mut edit = move |model| captured.lock().unwrap().push(model);
            let mut actions = sections::ResponseActions {
                load: &mut load,
                edit: &mut edit,
                browse_from: None,
            };
            panel_with_actions(
                ui,
                params.as_ref(),
                telemetry.as_ref(),
                &ParamSetter::new(host.as_ref()),
                entries,
                presets,
                nav,
                &response,
                Some(&mut actions),
                None,
            );
        });
    mxm_ui::paging::editor::request_card(&harness.ctx, mxm_ui::paging::Key(1));
    harness.run_steps(8);
    let size = harness
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Size")
        .rect();
    let start = size.center();
    harness.drag_at(start);
    harness.run_steps(2);
    harness.hover_at(egui::pos2(size.right() - 2.0, start.y));
    harness.run_steps(3);
    assert!(
        edits.lock().unwrap().is_empty(),
        "drag queued preparation work"
    );
    assert!(host_assert.0.lock().unwrap().is_empty());
    harness.drop_at(egui::pos2(size.right() - 2.0, start.y));
    harness.run_steps(3);
    let edits = edits.lock().unwrap();
    assert_eq!(edits.len(), 1, "release did not coalesce to one request");
    assert_ne!(edits[0].preparation.time_percent, 100);
    assert!(host_assert.0.lock().unwrap().is_empty());
}

/// **A held arrow submits where it stopped** (review, 2026-09-25). A key held across frames is one
/// gesture with no pointer down and the response idle, which is exactly when finished values are
/// cleared; clearing its value made the key's release submit the committed one. Key-down and
/// key-up are frames apart here, as they are under a finger.
#[test]
fn a_held_arrow_on_a_preparation_slider_submits_where_it_stopped() {
    let params = Box::leak(Box::new(Arc::new(MxmFxConvolutionParams::default())));
    let committed = Arc::clone(params)
        .response
        .snapshot()
        .preparation
        .time_percent;
    let telemetry = Box::leak(Box::new(Arc::new(Telemetry::default())));
    let host = Box::leak(Box::new(Arc::new(ApplyingHost::default())));
    let edits = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&edits);
    let entries = Box::leak(Box::new(HashMap::new()));
    let presets = Box::leak(Box::new(PresetUi::at(
        mxm_preset::Library::at(None),
        params.as_ref(),
    )));
    let nav = Box::leak(Box::new(mxm_ui::navigation::State::default()));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(vec2(1_200.0, 800.0))
        .build_ui(move |ui| {
            mxm_ui::typography::apply(ui.ctx());
            mxm_ui::theme::apply(ui.ctx());
            let response = sections::ResponseView::read(params.as_ref());
            let mut load = |_path| {};
            let captured = Arc::clone(&captured);
            let mut edit = move |model| captured.lock().unwrap().push(model);
            let mut actions = sections::ResponseActions {
                load: &mut load,
                edit: &mut edit,
                browse_from: None,
            };
            panel_with_actions(
                ui,
                params.as_ref(),
                telemetry.as_ref(),
                &ParamSetter::new(host.as_ref()),
                entries,
                presets,
                nav,
                &response,
                Some(&mut actions),
                None,
            );
        });
    harness.run_steps(8);
    // A press on the slider lands the cursor there, as it does for a person; it may submit a
    // value of its own, which the edits taken below leave out.
    let size = harness
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Size")
        .rect();
    harness.drag_at(size.center());
    harness.run_steps(2);
    harness.drop_at(size.center());
    harness.run_steps(4);
    let before = edits.lock().unwrap().len();

    // A held VALUE + ↑: the edit ends when VALUE is let go.
    harness.key_down(key_of(VALUE));
    harness.key_down(egui::Key::ArrowUp);
    harness.run_steps(6);
    harness.key_up(egui::Key::ArrowUp);
    harness.run_steps(2);
    harness.key_up(key_of(VALUE));
    harness.run_steps(4);

    let edits = edits.lock().unwrap();
    let submitted: Vec<u16> = edits[before..]
        .iter()
        .map(|model| model.preparation.time_percent)
        .collect();
    assert_eq!(
        submitted.len(),
        1,
        "one held arrow is one submission: {submitted:?}"
    );
    assert_ne!(
        submitted[0], committed,
        "the held arrow submitted the committed Size back"
    );
}

#[test]
fn production_app_bar_routes_preset_selection_and_init_through_the_deferred_adapter() {
    let params = Box::leak(Box::new(Arc::new(MxmFxConvolutionParams::default())));
    let telemetry = Box::leak(Box::new(Arc::new(Telemetry::default())));
    let host = Box::leak(Box::new(Arc::new(ApplyingHost::default())));
    let entries = Box::leak(Box::new(HashMap::new()));
    let presets = std::rc::Rc::new(std::cell::RefCell::new(PresetUi::at(
        mxm_preset::Library::at(None),
        params.as_ref(),
    )));
    let harness_presets = std::rc::Rc::clone(&presets);
    let nav = Box::leak(Box::new(mxm_ui::navigation::State::default()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(vec2(900.0, 600.0))
        .build_ui(move |ui| {
            mxm_ui::typography::apply(ui.ctx());
            mxm_ui::theme::apply(ui.ctx());
            let response = sections::ResponseView::read(params.as_ref());
            let mut load = |_path| {};
            let mut edit = |_model| {};
            let mut actions = sections::ResponseActions {
                load: &mut load,
                edit: &mut edit,
                browse_from: None,
            };
            let captured = Arc::clone(&captured);
            let mut request = move |request| {
                captured.lock().unwrap().push(request);
                Ok(())
            };
            let mut deferred = mxm_preset::DeferredPresetUi {
                pending: true,
                request: &mut request,
            };
            let mut presets = harness_presets.borrow_mut();
            panel_with_actions(
                ui,
                params.as_ref(),
                telemetry.as_ref(),
                &ParamSetter::new(host.as_ref()),
                entries,
                &mut presets,
                nav,
                &response,
                Some(&mut actions),
                Some(&mut deferred),
            );
        });
    harness.run_steps(4);

    // "Save preset" since the owner's ruling of 2026-09-22: the app bar's save always asks for a
    // name, so the label says which button it is rather than what half of it used to do.
    harness.get_by_label("Save preset").click();
    harness.run_steps(2);
    assert!(
        !presets.borrow().is_naming(),
        "pending Save preset was not suppressed"
    );
    assert!(requests.lock().unwrap().is_empty());

    harness.get_by_label("›").click();
    harness.run_steps(2);
    assert!(matches!(
        requests.lock().unwrap().as_slice(),
        [mxm_preset::DeferredPresetUiRequest::Load { .. }]
    ));

    harness.get_by_label("…").click();
    harness.run_steps(2);
    harness.get_by_label("Init patch").click();
    harness.run_steps(2);
    assert!(matches!(
        requests.lock().unwrap().as_slice(),
        [
            mxm_preset::DeferredPresetUiRequest::Load { .. },
            mxm_preset::DeferredPresetUiRequest::Init
        ]
    ));
}

#[test]
fn numeric_fault_acknowledgement_is_private_and_does_not_touch_audio_parameters() {
    let params = Arc::new(MxmFxConvolutionParams::default());
    let telemetry = Arc::new(Telemetry::default());
    telemetry.publish(0.5, 48_000, false, true);
    let host = Arc::new(ApplyingHost::default());
    let mut harness = editor_harness(
        params,
        Arc::clone(&telemetry),
        Arc::clone(&host),
        vec2(800.0, 600.0),
    );
    mxm_ui::paging::editor::request_card(&harness.ctx, mxm_ui::paging::Key(3));
    harness.run_steps(8);
    assert!(
        harness
            .query_all_by_label_contains("Numeric fault")
            .next()
            .is_some(),
        "latched numeric fault is absent from accessibility"
    );
    harness.get_by_label("Acknowledge fault").click();
    harness.run_steps(3);
    assert!(!telemetry.numeric_fault());
    assert!(host.0.lock().unwrap().is_empty());
}

#[test]
fn budget_clamp_is_neutral_information_and_size_reads_back_the_committed_maximum() {
    let params = Arc::new(MxmFxConvolutionParams::default());
    params.response.set_processing_rate(48_000.0);
    let mut state = crate::response::maximum_display_state_for_test();
    state.preparation.time_percent = 400;
    nice_plug::params::persist::PersistentField::set(&params.response, state);
    let committed = params.response.snapshot().preparation.time_percent;
    assert!(committed < 400, "the over-budget request was not clamped");
    assert!(matches!(
        params.response.status(),
        LoadState::Information(message)
            if message.starts_with("Too long to run here;") && !message.contains("Could not load")
    ));

    let telemetry = Arc::new(Telemetry::default());
    let host = Arc::new(ApplyingHost::default());
    // Roomy enough that Response — whose status carries the information — shares a page with
    // Time, which carries Size: hugged, a ten-second response's Time card is tall, and a smaller
    // window pages the two apart, which is the pager doing its job rather than what this checks.
    let mut harness = editor_harness(params, telemetry, host, vec2(1_200.0, 800.0));
    mxm_ui::paging::editor::request_card(&harness.ctx, mxm_ui::paging::Key(1));
    harness.run_steps(8);
    assert!(
        harness
            .query_all_by_label_contains("Too long to run here;")
            .next()
            .is_some(),
        "clamp information is absent from accessibility"
    );
    // The collection's slider announces its position normalised over Size's 25–400 %.
    let size = harness.get_by_role_and_label(egui::accesskit::Role::Slider, "Size");
    let position = size
        .accesskit_node()
        .numeric_value()
        .expect("a slider has a value");
    assert!(
        (position - (f64::from(committed) - 25.0) / 375.0).abs() < 1e-9,
        "Size reads {position}, not the committed {committed} %"
    );
}

#[test]
fn response_preparation_rejection_is_named_on_the_response_card() {
    let params = Arc::new(MxmFxConvolutionParams::default());
    let telemetry = Arc::new(Telemetry::default());
    telemetry.reject_response(crate::telemetry::ResponseRejection::UnsupportedRate(
        768_000.0,
    ));
    let host = Arc::new(ApplyingHost::default());
    let mut harness = editor_harness(params, telemetry, host, vec2(800.0, 600.0));
    mxm_ui::paging::editor::request_card(&harness.ctx, mxm_ui::paging::Key(0));
    harness.run_steps(8);
    assert!(
        harness
            .query_all_by_label_contains("the reverb runs from 8 to 384 kHz")
            .next()
            .is_some(),
        "response rejection reason is absent from accessibility"
    );
    assert!(
        harness
            .query_all_by_label_contains("The dry sound passes through.")
            .next()
            .is_some(),
        "the inert fallback is not named"
    );
}

/// Nothing is disclosed; every current sound control is directly visible.
const REVEAL: fn(&egui::Context) = |_| {};

#[test]
fn the_keyboard_cursor_reaches_and_operates_every_parameter() {
    let params = MxmFxConvolutionParams::default();
    let telemetry = Telemetry::default();
    let host = keyboard_checks::Recorder::default();
    let setter = ParamSetter::new(&host);
    // The parameters, and the response model's own controls on the shared widgets — the
    // interpretation only for a stereo response, which is the only kind it is drawn for.
    let stereo = sections::ResponseView::read(&params).channel_count == 2;
    let ids: Vec<&str> = sections::all_parameters(&params)
        .iter()
        .map(|binding| binding.id)
        .chain(
            sections::PREPARATION_IDS
                .into_iter()
                .filter(|&id| stereo || id != "response-interpretation"),
        )
        .collect();
    let mut entries = HashMap::new();
    let mut presets = PresetUi::at(mxm_preset::Library::at(None), &params);
    let mut nav = mxm_ui::navigation::State::default();
    // The opening control is the starter response's Interpretation, which edits the response
    // model rather than a host parameter; the arrow is proved on Pre-delay, the first parameter.
    keyboard_checks::the_cursor_reaches_and_operates_from(
        vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
        &sections::test_items(),
        keyboard_checks::Coverage::Exactly(&ids),
        "predelay",
        &REVEAL,
        &host,
        &mut |ui| {
            panel(
                ui,
                &params,
                &telemetry,
                &setter,
                &mut entries,
                &mut presets,
                &mut nav,
            );
        },
    );
}

use mxm_plugin_test::tree_checks;

/// Every page at the opening size, light and dark, for the owner's review of the layout-tree
/// conversion (plans/plan-layout-tree.md §4.3): `target/layout-tree/mxm-fx-convolution/<tag>/`,
/// where `MXM_PICTURES` names the tag — `before` on the unconverted editor, `after` on the tree.
///
/// `MXM_PICTURES=after cargo test -p mxm-fx-convolution --lib tree_pictures -- --ignored`
#[test]
#[ignore = "renders through wgpu; run by hand"]
fn tree_pictures() {
    let tag = std::env::var("MXM_PICTURES").unwrap_or_else(|_| "after".to_owned());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/layout-tree/mxm-fx-convolution")
        .join(tag);
    let (params, telemetry, host, mut entries, mut presets, mut nav) = panel_state();
    let setter = ParamSetter::new(&host);
    tree_checks::pictures(
        &|_| {},
        vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
        &dir,
        &mut |ui| {
            panel(
                ui,
                &params,
                &telemetry,
                &setter,
                &mut entries,
                &mut presets,
                &mut nav,
            );
        },
    );
}

/// Every card, in every state that changes what it holds, passes the layout tree's checks
/// (plans/plan-layout-tree.md §4.3, `tree_checks::card`): its computed floor holds its content
/// with nothing painted outside the card, the floor is exact, the height its tree states is the
/// height it draws, and every leaf stays in the room it was given.
///
/// The structural-state matrix: the starter response (stereo, so its interpretation menu is
/// shown); a mono response, which has none, while a named file prepares; a ten-second stereo
/// response read as diagonal stereo whose last load failed, with Mix at Off, a tail sounding and a
/// numeric fault latched, so the fault button replaces the linear-output note; a response the host
/// rate rejected, whose status is the longest line either card prints; and the panel without its
/// acquisition callbacks, where Browse, the menus and the preparation sliders are absent.
#[test]
fn every_card_passes_the_tree_checks_in_every_state() {
    use nice_plug::params::Param;
    let idle = sections::MeterView {
        peak: 0.0,
        tail_samples: 0,
        numeric_fault: false,
        response_rejection: None,
        tempo: None,
    };
    for state in [
        "init",
        "mono response preparing a file",
        "ten seconds, failed, Off with a tail and a fault",
        "host rate rejected",
        "no acquisition callbacks",
        "pre-delay synced, no tempo",
        "pre-delay synced to a tempo",
    ] {
        let params = MxmFxConvolutionParams::default();
        let mut response = sections::ResponseView::read(&params);
        let mut meters = idle;
        let mut actions = true;
        match state {
            "mono response preparing a file" => {
                response.channel_count = 1;
                response.model.interpretation = crate::response::Interpretation::Mono;
                response.status =
                    LoadState::Loading("A rather long impulse response file name.wav".to_owned());
            }
            "ten seconds, failed, Off with a tail and a fault" => {
                response.name = "A ten-second stereo response with a long embedded name".to_owned();
                response.sample_rate = 48_000;
                response.frames = 480_000;
                response.model.interpretation = crate::response::Interpretation::DiagonalStereo;
                response.status = LoadState::Failed(
                    "the file is not a mono or stereo PCM or float WAV response".to_owned(),
                );
                // SAFETY: the parameters are this test's own and nothing else reads them.
                unsafe {
                    let _ = params.mix.as_ptr()._internal_set_normalized_value(0.0);
                }
                meters.tail_samples = 48_000;
                meters.numeric_fault = true;
            }
            "host rate rejected" => {
                meters.response_rejection =
                    Some(crate::telemetry::ResponseRejection::Deadline(768_000.0));
            }
            "no acquisition callbacks" => actions = false,
            "pre-delay synced, no tempo" | "pre-delay synced to a tempo" => {
                // SAFETY: the parameters are this test's own and nothing else reads them.
                unsafe {
                    let _ = params
                        .pre_delay_sync
                        .as_ptr()
                        ._internal_set_normalized_value(1.0);
                }
                if state == "pre-delay synced to a tempo" {
                    meters.tempo = Some(120.0);
                }
            }
            _ => {}
        }
        let inputs = sections::Inputs {
            params: &params,
            response: &response,
            meters,
            actions,
        };
        let floors = floors_for(inputs);
        let host = ApplyingHost::default();
        let setter = ParamSetter::new(&host);
        for (index, floor) in floors.iter().enumerate() {
            let mut text = HashMap::new();
            let mut clear_fault = false;
            let mut load = |_path: std::path::PathBuf| {};
            let mut edit = |_model: crate::response::ResponseModel| {};
            let mut callbacks = sections::ResponseActions {
                load: &mut load,
                edit: &mut edit,
                browse_from: None,
            };
            let mut live = sections::Live {
                inputs,
                setter: &setter,
                text: &mut text,
                clear_fault: &mut clear_fault,
                actions: actions.then_some(&mut callbacks),
            };
            tree_checks::card(
                &|_| {},
                state,
                sections::TITLES[index],
                *floor,
                &|ui| sections::card(ui, index, inputs),
                &mut |ui, leaf, rect| sections::paint(ui, &mxm_ui::LIGHT, leaf, rect, &mut live),
            );
            assert!(!clear_fault, "{state}: drawing acknowledged the fault");
        }
        assert!(
            host.0.lock().unwrap().is_empty(),
            "{state}: drawing a card edited a parameter"
        );
    }
}

/// The floors the panel computes for `inputs`, from a context set up as an editor's is.
fn floors_for(inputs: sections::Inputs<'_>) -> Vec<f32> {
    let ctx = tree_checks::context(&|_| {});
    let mut floors = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            floors = sections::page_items(ui, inputs)
                .iter()
                .map(|item| item.card.floor)
                .collect();
        });
        output.textures_delta.clear();
    }
    floors
}
