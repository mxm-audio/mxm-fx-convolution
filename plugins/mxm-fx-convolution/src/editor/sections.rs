//! The Response, Time, Shape and Output cards in preparation and signal order.
//!
//! Each card body is a `mxm_ui::tree` (plans/plan-layout-tree.md): [`card`] describes it once from
//! the parameters, the response view, the telemetry snapshot and the editor's own preparation
//! values, the paging renderer measures that one description for the card's floor and height, and
//! [`paint`] draws it leaf by leaf — shared knobs through the bindings, and the response model's own values
//! (the preparation sliders, Reverse, the interpretation and the tail shape) on the same shared
//! controls, held locally while dragged and submitted on release. Browse and Acknowledge fault are
//! the collection's button. Nothing is typed and nothing is drawn to learn a size.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::Ui;
use mxm_ui::control::{ParamView, Size, Steps, Wave, Wheel};
use mxm_ui::space::SPACE_2;
use mxm_ui::theme::Tokens;
use mxm_ui::tree::{Flow, Font, Height, Kind, Node, leaf, pad_all, reserve, stack};
use mxm_ui::visual::{
    AXIS_STROKE, CANVAS_RADIUS, EMPHASIS_STROKE, INNER_GUTTER, TALL_PLOT_HEIGHT, TRACE_STROKE,
};
use nice_plug::prelude::ParamSetter;

use super::binding::Bound;
use crate::params::MxmFxConvolutionParams;
use crate::response::{
    DISPLAY_BINS, DISPLAY_FLOOR_DB, Interpretation, LoadState, MAX_SOURCE_SECONDS, ResponseModel,
    TailShape,
};
use crate::telemetry::ResponseRejection;

const KNOB: Size = Size::Standard;
/// The cards' titles, in paging order.
pub const TITLES: [&str; 4] = ["Response", "Time", "Shape", "Output"];
/// The response plot's stated size: it fills its card's width and is never narrower than this.
pub const PLOT_MIN: egui::Vec2 = egui::vec2(200.0, TALL_PLOT_HEIGHT);
/// The linear response-shape strip's height; it fills its card's width.
pub const STRIP_HEIGHT: f32 = 60.0;
const RESPONSE_TIME_GROUP: &[mxm_ui::paging::Key] =
    &[mxm_ui::paging::Key(0), mxm_ui::paging::Key(1)];
const SHAPE_GROUP: &[mxm_ui::paging::Key] = &[mxm_ui::paging::Key(2)];
const OUTPUT_GROUP: &[mxm_ui::paging::Key] = &[mxm_ui::paging::Key(3)];
const GROUPS: &[&[mxm_ui::paging::Key]] = &[RESPONSE_TIME_GROUP, SHAPE_GROUP, OUTPUT_GROUP];

const BROWSE: &str = "Browse…";
const ACKNOWLEDGE: &str = "Acknowledge fault";
const WET_EQ: &str = "Wet EQ";
const FAULT: &str = "Numeric fault · invalid input was replaced with silence";

#[derive(Clone, Debug)]
pub struct ResponseView {
    pub name: String,
    pub sample_rate: u32,
    pub frames: u32,
    pub channel_count: usize,
    pub model: ResponseModel,
    pub energy: [[f32; 2]; DISPLAY_BINS],
    pub status: LoadState,
}

impl ResponseView {
    pub fn read(params: &MxmFxConvolutionParams) -> Self {
        let display = params.response.display();
        Self {
            name: display.name,
            sample_rate: display.sample_rate,
            frames: display.frames,
            channel_count: display.channel_count,
            model: display.model,
            energy: display.energy,
            status: params.response.status(),
        }
    }

    fn duration_seconds(&self) -> f32 {
        self.frames as f32 / self.sample_rate.max(1) as f32
    }

    fn prepared_extent_frames(&self) -> u32 {
        let available = self.frames.saturating_sub(self.model.preparation.onset);
        if self.model.preparation.extent == 0 {
            available
        } else {
            self.model.preparation.extent.min(available)
        }
    }

    fn prepared_duration_seconds(&self) -> f32 {
        self.prepared_extent_frames() as f32 / self.sample_rate.max(1) as f32
            * self.model.preparation.time_percent as f32
            / 100.0
    }

    fn interpretation_label(&self) -> &'static str {
        interpretation_label(self.model.interpretation)
    }

    /// The metadata line under the response's name.
    fn metadata(&self) -> String {
        format!(
            "{} · {:.1} ms · {} Hz",
            self.interpretation_label(),
            self.duration_seconds() * 1_000.0,
            self.sample_rate
        )
    }

    /// The Time card's closing line. [`PREPARED_WIDEST`] is its widest.
    fn prepared(&self) -> String {
        format!(
            "Prepared response {:.1} ms · source {:.1} ms",
            self.prepared_duration_seconds() * 1_000.0,
            self.duration_seconds() * 1_000.0
        )
    }
}

/// [`ResponseView::prepared`] at its widest: both lengths at the ten seconds a source may hold, and
/// a Size that stretches the prepared one fourfold.
const PREPARED_WIDEST: &str = "Prepared response 40000.0 ms · source 10000.0 ms";

fn interpretation_label(interpretation: Interpretation) -> &'static str {
    match interpretation {
        Interpretation::Mono => "Mono",
        Interpretation::MonoToStereo => "Mono to stereo",
        Interpretation::DiagonalStereo => "Diagonal stereo",
    }
}

fn tail_shape_label(shape: TailShape) -> &'static str {
    match shape {
        TailShape::Natural => "Natural",
        TailShape::Fade => "Fade",
        TailShape::Swell => "Swell",
        TailShape::Gate => "Gate",
    }
}

/// A stereo response's two readings, in the switch's order.
const INTERPRETATIONS: [Interpretation; 2] =
    [Interpretation::MonoToStereo, Interpretation::DiagonalStereo];
const INTERPRETATION: &str = "Interpretation";
/// What each interpretation does, in [`INTERPRETATIONS`]' order (design system §7.3; the owner,
/// 2026-09-27: the cells of a row do not share one sentence).
const INTERPRETATION_DETAILS: [&str; 2] = [
    "Hears the file as one response, spread across both sides.",
    "Plays each input side through its own side of the file.",
];

/// The tail laws, in the switch's order, each drawn as the envelope it multiplies the kept response
/// by (design system §7.3: the shape the DSP makes, `response.rs`'s `shape`): one throughout,
/// `(1 − x)²`, `x²`, and one until three quarters, then nothing.
const TAIL_SHAPES: [(TailShape, Wave); 4] = [
    (TailShape::Natural, Wave::Level),
    (TailShape::Fade, Wave::Fade),
    (TailShape::Swell, Wave::Rise),
    (TailShape::Gate, Wave::Gated),
];
const TAIL_SHAPE: &str = "Tail shape";
/// What each tail shape does, in [`TAIL_SHAPES`]' order.
const TAIL_SHAPE_DETAILS: [&str; 4] = [
    "The response as recorded.",
    "Fades the tail away to silence by its end.",
    "Swells the tail up toward its end.",
    "Holds the tail, then stops it dead three quarters of the way through.",
];

const REVERSE: &str = "Reverse";
const REVERSE_ABOUT: &str = "Plays the reverb backwards, so it swells up into the sound.";

/// The response model's own navigation ids, beside the parameters' — what the keyboard cursor
/// reaches on the Time and Shape cards, and on Response for a stereo response.
pub const PREPARATION_IDS: [&str; 8] = [
    "response-interpretation",
    "response-onset",
    "response-extent",
    "response-reverse",
    "response-size",
    "response-decay",
    "response-tailshape",
    "response-damping",
];

/// How a preparation value reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    /// Milliseconds, to a tenth.
    Milliseconds,
    /// Whole percent.
    Percent,
}

/// One preparation value — model state, not a host parameter — as the collection's slider
/// (`control::slider`) draws it: its name, range, reading and the value a double-click returns to.
#[derive(Clone, Copy)]
struct Preparing {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    low: f64,
    high: f64,
    default: f64,
    unit: Unit,
}

impl Preparing {
    fn format(&self, value: f64) -> String {
        match self.unit {
            Unit::Milliseconds => format!("{value:.1} ms"),
            Unit::Percent => format!("{value:.0} %"),
        }
    }

    /// Typed text, with or without its unit, clamped to the range; anything else is refused.
    fn parse(&self, text: &str) -> Option<f64> {
        let number = text
            .trim()
            .trim_end_matches("ms")
            .trim_end_matches('%')
            .trim()
            .parse::<f64>()
            .ok()?;
        number
            .is_finite()
            .then(|| self.snap(number.clamp(self.low, self.high)))
    }

    fn snap(&self, value: f64) -> f64 {
        match self.unit {
            Unit::Milliseconds => value,
            Unit::Percent => value.round(),
        }
    }

    fn normalised(&self, value: f64) -> f64 {
        ((value - self.low) / (self.high - self.low).max(f64::EPSILON)).clamp(0.0, 1.0)
    }

    fn value(&self, normalised: f64) -> f64 {
        self.snap(self.low + normalised.clamp(0.0, 1.0) * (self.high - self.low))
    }

    fn widest(&self) -> String {
        mxm_ui::control::widest_value(|n| self.format(self.value(n)))
    }

    /// A percent steps by one and ten; a time by the shared hundredth and tenth of its range.
    fn steps(&self) -> Steps {
        match self.unit {
            Unit::Milliseconds => Steps::DEFAULT,
            Unit::Percent => {
                let one = 1.0 / (self.high - self.low);
                Steps {
                    fine_up: one,
                    fine_down: one,
                    coarse_up: 10.0 * one,
                    coarse_down: 10.0 * one,
                }
            }
        }
    }
}

/// The preparation value `leaf` draws, over the response it prepares: Onset and Extent run across
/// the source's own length.
fn preparing(leaf: Leaf, response: &ResponseView) -> Preparing {
    let source = f64::from((response.duration_seconds() * 1_000.0).max(0.1));
    match leaf {
        Leaf::Onset => Preparing {
            id: "response-onset",
            name: "Onset",
            description: "Where the response starts in the file; anything before is skipped.",
            low: 0.0,
            high: source,
            default: 0.0,
            unit: Unit::Milliseconds,
        },
        Leaf::Extent => Preparing {
            id: "response-extent",
            name: "Extent",
            description: "How much of the response is kept.",
            low: 0.1,
            high: source,
            default: source,
            unit: Unit::Milliseconds,
        },
        Leaf::Size => Preparing {
            id: "response-size",
            name: "Size",
            description: "Makes the space bigger or smaller; 100 % is as recorded.",
            low: 25.0,
            high: 400.0,
            default: 100.0,
            unit: Unit::Percent,
        },
        Leaf::Decay => Preparing {
            id: "response-decay",
            name: "Decay",
            description: "Shortens or lengthens the tail; 100 % is as recorded.",
            low: 25.0,
            high: 400.0,
            default: 100.0,
            unit: Unit::Percent,
        },
        Leaf::Damping => Preparing {
            id: "response-damping",
            name: "Damping",
            description: "Darkens the tail as it goes on; zero leaves it as recorded.",
            low: 0.0,
            high: 100.0,
            default: 0.0,
            unit: Unit::Percent,
        },
        other => unreachable!("{other:?} is not a preparation value"),
    }
}

/// A preparation value's leaf: the collection's slider, holding its widest reading.
fn preparing_slider(key: Leaf, response: &ResponseView) -> Node<Leaf> {
    let preparing = preparing(key, response);
    leaf(
        key,
        Kind::Slider {
            label: preparing.name.to_owned(),
            quiet: false,
            widest: preparing.widest(),
        },
    )
}

#[derive(Clone, Copy, Debug)]
pub struct MeterView {
    pub peak: f32,
    pub tail_samples: u64,
    pub numeric_fault: bool,
    pub response_rejection: Option<ResponseRejection>,
    /// The host tempo in force: a synced Pre-delay reads its division with one.
    pub tempo: Option<f64>,
}

pub struct ResponseActions<'a> {
    pub load: &'a mut dyn FnMut(PathBuf),
    pub edit: &'a mut dyn FnMut(ResponseModel),
    /// The collection's impulses folder, resolved once by the editor; Browse opens there when it
    /// is installed (`crate::impulses`).
    pub browse_from: Option<&'a Path>,
}

pub fn all_parameters(params: &MxmFxConvolutionParams) -> [Bound<'_>; 9] {
    [
        Bound::new(
            "mix",
            &params.mix,
            "The balance of dry sound and reverb: at half they are equally loud, at zero the effect is off.",
        ),
        Bound::new(
            "predelay",
            &params.pre_delay,
            "Delays the start of the reverb; the dry sound stays on time.",
        ),
        Bound::new(
            "lowcut",
            &params.low_cut,
            "Removes low end from the reverb; zero leaves it.",
        )
        .law(mxm_preset::StepLaw::Hertz),
        Bound::new(
            "highcut",
            &params.high_cut,
            "Removes high end from the reverb; Open leaves it.",
        )
        .law(mxm_preset::StepLaw::Hertz),
        Bound::new(
            "tone",
            &params.tone,
            "Tilts the reverb darker or brighter; zero is flat.",
        ),
        Bound::new(
            "width",
            &params.width,
            "Narrows or widens the reverb's stereo image.",
        ),
        Bound::new(
            "modulation",
            &params.modulation,
            "Adds gentle movement to the reverb; zero is off.",
        ),
        Bound::new(
            "feedback",
            &params.feedback,
            "Feeds the reverb back into itself for a longer tail; zero is off.",
        ),
        Bound::new(
            "predelaysync",
            &params.pre_delay_sync,
            super::binding::SYNC_DESCRIPTION,
        ),
    ]
}

fn bound<'a>(id: &str, params: &'a MxmFxConvolutionParams) -> Bound<'a> {
    all_parameters(params)
        .into_iter()
        .find(|binding| binding.id == id)
        .expect("every drawn parameter is bound")
}

/// What a card's tree is built from: the parameters, the committed response, the telemetry
/// snapshot taken once before the frame, and whether acquisition and preparation are offered.
/// The editor's in-progress preparation values are read from the `Ui` the tree is built in.
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub params: &'a MxmFxConvolutionParams,
    pub response: &'a ResponseView,
    pub meters: MeterView,
    pub actions: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn cards(
    ui: &mut Ui,
    tokens: &Tokens,
    params: &MxmFxConvolutionParams,
    setter: &ParamSetter<'_>,
    text: &mut HashMap<&'static str, Option<String>>,
    response: &ResponseView,
    meters: MeterView,
    clear_fault: &mut bool,
    actions: Option<&mut ResponseActions<'_>>,
) -> f32 {
    clear_finished_preparation_values(ui, response);
    let inputs = Inputs {
        params,
        response,
        meters,
        actions: actions.is_some(),
    };
    let items = page_items(ui, inputs);
    let text_editing = text.values().any(Option::is_some);
    let mut live = Live {
        inputs,
        setter,
        text,
        clear_fault,
        actions,
    };
    let report = mxm_ui::paging::editor::show(
        ui,
        tokens,
        &items,
        GROUPS,
        text_editing,
        &mut |ui, index| card(ui, index, inputs),
        &mut |ui, _, leaf, rect| paint(ui, tokens, leaf, rect, &mut live),
    );
    report
        .visible
        .iter()
        .map(|(_, rect)| rect.bottom())
        .fold(ui.min_rect().bottom(), f32::max)
}

/// Every paging item, each floor computed from its card's tree in `ui`'s fonts: the tree's
/// narrowest and the card's chrome. There is no usability minimum to add, and each card is as wide
/// as its floor (`plans/plan-editor-standard.md` A1).
pub fn page_items(ui: &Ui, inputs: Inputs<'_>) -> Vec<mxm_ui::paging::Item<'static>> {
    use mxm_ui::paging::{Category, Item, Key};
    TITLES
        .iter()
        .enumerate()
        .map(|(index, title)| Item {
            key: Key(index as u64),
            card: {
                let floor = mxm_ui::tree::card_floor(ui, title, &card(ui, index, inputs));
                mxm_ui::flow::Card::new(title, floor).capped(floor)
            },
            category: Category::Effects,
            kind: match index {
                0 | 1 => "Response preparation",
                2 => "Response shape",
                _ => "Output",
            },
        })
        .collect()
}

/// The paging items as the production panel computes them for a fresh instance — the starter
/// response, idle telemetry, acquisition offered — from a context set up as an editor's is, three
/// passes in so the weighted font cuts are bound: for tests, which have no editor `Ui` to hand.
#[cfg(test)]
pub(crate) fn test_items() -> Vec<mxm_ui::paging::Item<'static>> {
    let ctx = egui::Context::default();
    mxm_ui::typography::apply(&ctx);
    mxm_ui::theme::apply(&ctx);
    let params = MxmFxConvolutionParams::default();
    let response = ResponseView::read(&params);
    let inputs = Inputs {
        params: &params,
        response: &response,
        meters: MeterView {
            peak: 0.0,
            tail_samples: 0,
            numeric_fault: false,
            response_rejection: None,
            tempo: None,
        },
        actions: true,
    };
    let mut items = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            items = page_items(ui, inputs);
        });
        output.textures_delta.clear();
    }
    items
}

/// The cards' floors in paging order, as [`test_items`] computes them.
#[cfg(test)]
pub(crate) fn test_floors() -> Vec<f32> {
    test_items().iter().map(|item| item.card.floor).collect()
}

/// The widest computed floor, as [`test_items`] computes them: the one-card minimum's content.
#[cfg(test)]
pub(crate) fn minimum_card_width() -> f32 {
    let cards: Vec<_> = test_items().iter().map(|item| item.card).collect();
    mxm_ui::flow::minimum_width(&cards)
}

/// Where the Browse dialog's pick waits for the editor frame that collects it.
pub fn browse_picker() -> egui::Id {
    egui::Id::new("mxm-fx-convolution-browse")
}

/// What a leaf of this editor's cards draws. Hashed by what it names, which keeps its widget ids —
/// a menu's, a slider's value box — stable wherever the tree places it.
#[derive(Clone, Copy, Debug, Hash)]
pub enum Leaf {
    /// A control's tempo sync, the quarter note beside it.
    Picture(&'static str),
    Plot,
    Browse,
    Interpretation,
    Name,
    Metadata,
    ResponseStatus,
    Knob(&'static str),
    Onset,
    Extent,
    Reverse,
    Size,
    Decay,
    Prepared,
    TailShape,
    Damping,
    WetEq,
    LinearStrip,
    OutputStatus,
    Fault,
    Acknowledge,
}

/// The collection's knob row (`mxm_ui::tree::knob_row`): equal columns at the one knob column.
fn knobs(ui: &Ui, params: &MxmFxConvolutionParams, ids: &[&'static str]) -> Node<Leaf> {
    mxm_ui::tree::knob_row(
        ui,
        ids.iter()
            .map(|id| {
                let param = bound(id, params).param;
                // A syncable control's column holds its free readings and its divisions.
                let widest = if *id == "predelay" {
                    super::binding::synced_widest(param, crate::params::PRE_DELAY_SYNC.span)
                } else {
                    mxm_ui::control::widest_value(|n| param.format(n as f32))
                };
                let knob = leaf(
                    Leaf::Knob(id),
                    Kind::Knob {
                        name: param.name().to_owned(),
                        widest,
                        size: KNOB,
                        // In the collection's knob row, which sizes the columns.
                        column: 0.0,
                    },
                );
                (KNOB, knob)
            })
            .collect(),
    )
}

/// Body text that wraps within its card, as `ui.label` wraps in a top-down body.
fn text(key: Leaf, text: impl Into<String>) -> Node<Leaf> {
    leaf(
        key,
        Kind::Text {
            text: text.into(),
            font: Font::Body,
            flow: Flow::Wrap,
        },
    )
}

/// Every line [`output_status`] can say, so the Output card holds the longest (E1).
const OUTPUT_STATUSES: [&str; 3] = ["Off · exact dry after the fade", "Tail active", "Ready"];

/// The Output card's status line and whether it is the Off reading.
fn output_status(inputs: Inputs<'_>) -> (&'static str, bool) {
    if inputs.params.mix.value() == 0.0 {
        ("Off · exact dry after the fade", true)
    } else if inputs.meters.tail_samples > 0 {
        ("Tail active", false)
    } else {
        ("Ready", false)
    }
}

/// Card `index`'s body, as a tree, from what it shows now: the response's name, metadata and
/// status re-wrap as they change, the interpretation menu exists only for a stereo response, and
/// the fault button replaces the linear-output note while telemetry reports a numeric fault.
pub fn card(ui: &Ui, index: usize, inputs: Inputs<'_>) -> Node<Leaf> {
    let Inputs {
        params,
        response,
        meters,
        actions,
    } = inputs;
    let (status, _) = response_status_text(response, meters);
    // **The status holds the room of every line it can say about this response** (E1): the fixed
    // lines, and the ones that name it. A decoder's own message is open-ended and still re-wraps.
    let status_leaf = |key: Leaf| {
        reserve(
            text(key, status.clone()),
            response_statuses(response)
                .into_iter()
                .map(|line| text(key, line))
                .collect(),
        )
    };
    match index {
        0 => {
            // The plot keeps `SPACE_2` of its own below it, beyond the body's rhythm; it is the drop
            // target too, and the drop reads the leaf's own rectangle.
            let mut body = vec![pad_all(
                0.0,
                0.0,
                SPACE_2,
                leaf(
                    Leaf::Plot,
                    Kind::Custom {
                        min_width: PLOT_MIN.x,
                        height: Height::Fixed(PLOT_MIN.y),
                        fills: true,
                    },
                ),
            )];
            if actions {
                body.push(leaf(
                    Leaf::Browse,
                    Kind::Button {
                        label: BROWSE.to_owned(),
                        min: egui::Vec2::ZERO,
                        fills: false,
                    },
                ));
                if response.channel_count == 2 {
                    body.push(leaf(
                        Leaf::Interpretation,
                        Kind::Segmented {
                            label: INTERPRETATION.to_owned(),
                            options: INTERPRETATIONS
                                .iter()
                                .map(|&i| interpretation_label(i).to_owned())
                                .collect(),
                            beside: None,
                        },
                    ));
                }
            }
            body.push(leaf(
                Leaf::Name,
                Kind::Text {
                    text: response.name.clone(),
                    font: Font::Body,
                    flow: Flow::Truncate,
                },
            ));
            body.push(text(Leaf::Metadata, response.metadata()));
            body.push(status_leaf(Leaf::ResponseStatus));
            stack(body)
        }
        1 => {
            // Pre-delay with its tempo sync beside it (`plans/plan-tempo-sync-controls.md`).
            let mut body = vec![mxm_ui::tree::row_gap(
                ui.spacing().item_spacing.x,
                vec![
                    knobs(ui, params, &["predelay"]),
                    mxm_ui::tree::switch_beside_knob(
                        KNOB,
                        leaf(Leaf::Picture("predelaysync"), Kind::SyncToggle),
                    ),
                ],
            )];
            if actions {
                body.extend([
                    preparing_slider(Leaf::Onset, response),
                    preparing_slider(Leaf::Extent, response),
                    leaf(
                        Leaf::Reverse,
                        Kind::Toggle {
                            label: REVERSE.to_owned(),
                        },
                    ),
                    preparing_slider(Leaf::Size, response),
                    preparing_slider(Leaf::Decay, response),
                ]);
            }
            // A caption (E4), holding the room of its widest reading (E1): the two lengths move as
            // the response is prepared.
            let prepared = response.prepared();
            body.push(reserve(
                mxm_ui::tree::caption(Leaf::Prepared, &prepared),
                vec![mxm_ui::tree::caption(Leaf::Prepared, PREPARED_WIDEST)],
            ));
            stack(body)
        }
        2 => {
            let mut body = Vec::new();
            if actions {
                body.push(leaf(
                    Leaf::TailShape,
                    Kind::Waves {
                        label: Some(TAIL_SHAPE.to_owned()),
                        count: TAIL_SHAPES.len(),
                        marks: Vec::new(),
                        beside: None,
                    },
                ));
                body.push(preparing_slider(Leaf::Damping, response));
            }
            body.extend([
                knobs(ui, params, &["feedback"]),
                text(Leaf::WetEq, WET_EQ),
                knobs(ui, params, &["lowcut", "highcut", "tone", "modulation"]),
                leaf(
                    Leaf::LinearStrip,
                    Kind::Custom {
                        min_width: 0.0,
                        height: Height::Fixed(STRIP_HEIGHT),
                        fills: true,
                    },
                ),
            ]);
            stack(body)
        }
        _ => {
            // The fault and its button take the linear-output note's place, and the card keeps the
            // room of both, so a fault arriving does not move what is under the pointer (E1).
            let fault = || {
                stack(vec![
                    text(Leaf::Fault, FAULT),
                    leaf(
                        Leaf::Acknowledge,
                        Kind::Button {
                            label: ACKNOWLEDGE.to_owned(),
                            min: egui::Vec2::ZERO,
                            fills: false,
                        },
                    ),
                ])
            };
            // **Nothing when all is well** (the owner, 2026-09-27: no help text on the panel): the
            // fault line's room is kept, so a fault appearing never moves the card.
            let (shown, other) = if meters.numeric_fault {
                (fault(), Node::Space(egui::Vec2::ZERO))
            } else {
                (Node::Space(egui::Vec2::ZERO), fault())
            };
            stack(vec![
                knobs(ui, params, &["width", "mix"]),
                status_leaf(Leaf::ResponseStatus),
                reserve(
                    text(Leaf::OutputStatus, output_status(inputs).0),
                    OUTPUT_STATUSES
                        .iter()
                        .map(|&line| text(Leaf::OutputStatus, line))
                        .collect(),
                ),
                reserve(shown, vec![other]),
            ])
        }
    }
}

/// Everything a leaf draws with: the tree's inputs, the parameters' host, the text-entry buffers,
/// the fault acknowledgement the panel applies after the frame, and the acquisition callbacks.
pub struct Live<'a, 'b, 'c> {
    pub inputs: Inputs<'a>,
    pub setter: &'a ParamSetter<'b>,
    pub text: &'a mut HashMap<&'static str, Option<String>>,
    pub clear_fault: &'a mut bool,
    pub actions: Option<&'a mut ResponseActions<'c>>,
}

/// Draws one leaf, in the `Ui` the tree bounded to `rect`, exactly as the card body drew it before
/// it was a tree: the same bindings, the same egui widgets and ids, the same colours.
pub fn paint(
    ui: &mut Ui,
    tokens: &Tokens,
    leaf: &Leaf,
    rect: egui::Rect,
    live: &mut Live<'_, '_, '_>,
) {
    let Inputs {
        params,
        response,
        meters,
        ..
    } = live.inputs;
    match *leaf {
        Leaf::Plot => {
            response_plot(ui, tokens, response, meters);
            if let Some(actions) = live.actions.as_deref_mut() {
                accept_drop(ui, rect, actions.load);
            }
        }
        Leaf::Browse => {
            let Some(actions) = live.actions.as_deref_mut() else {
                return;
            };
            // The dialog runs off this frame (`mxm_ui::offthread`) and the editor collects the pick
            // next frame: opened here, its modal loop re-enters this window mid-frame and aborts
            // the host.
            let browsing = mxm_ui::offthread::running::<PathBuf>(ui.ctx(), browse_picker());
            if ui
                .add_enabled(!browsing, egui::Button::new(BROWSE))
                .on_hover_text("Choose a mono or stereo WAV response, or drop one on this card.")
                .clicked()
            {
                let folder = actions.browse_from.map(Path::to_path_buf);
                mxm_ui::offthread::start(ui.ctx(), browse_picker(), move || {
                    let dialog = rfd::FileDialog::new().add_filter("Wave audio", &["wav", "wave"]);
                    match crate::impulses::browse_start(folder.as_deref()) {
                        Some(start) => dialog.set_directory(start),
                        None => dialog,
                    }
                    .pick_file()
                });
            }
        }
        Leaf::Interpretation => {
            let Some(actions) = live.actions.as_deref_mut() else {
                return;
            };
            let requested = requested_model(ui, response);
            let options = INTERPRETATIONS.map(interpretation_label);
            let mut selected = INTERPRETATIONS
                .iter()
                .position(|&i| i == requested.interpretation)
                .unwrap_or(0);
            let changed = mxm_ui::navigation::at(ui, "response-interpretation", |ui| {
                mxm_ui::control::segmented(
                    ui,
                    tokens,
                    INTERPRETATION,
                    &options,
                    &mut selected,
                    None,
                    None,
                    &INTERPRETATION_DETAILS,
                )
            });
            if changed {
                let mut model = requested;
                model.interpretation = INTERPRETATIONS[selected];
                (actions.edit)(model);
            }
        }
        Leaf::Name => {
            ui.add(
                egui::Label::new(&response.name)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .sense(egui::Sense::hover()),
            )
            .on_hover_text(format!(
                "{}\nThe embedded response name saved with this project.",
                response.name
            ));
        }
        Leaf::Metadata => {
            ui.label(egui::RichText::new(response.metadata()).color(tokens.text_secondary));
        }
        Leaf::ResponseStatus => response_status(ui, tokens, response, meters),
        // At the knob row's column (`tree::knob_row`). Synced to a tempo, Pre-delay reads its
        // division; the host still reads its time.
        Leaf::Knob(id) => {
            let size = KNOB;
            let bound = bound(id, params);
            let division = {
                use nice_plug::prelude::Param as _;
                let synced: Option<(bool, &nice_plug::prelude::FloatParam, mxm_tempo::Ladder)> =
                    match id {
                        "predelay" => Some((
                            params.pre_delay_sync.value(),
                            &params.pre_delay,
                            crate::params::PRE_DELAY_SYNC,
                        )),
                        _ => None,
                    };
                synced
                    .filter(|(on, _, _)| *on)
                    .and_then(|(_, param, ladder)| {
                        ladder.shown(
                            param.unmodulated_normalized_value(),
                            meters.tempo,
                            f64::from(param.preview_plain(0.0)),
                            f64::from(param.preview_plain(1.0)),
                        )
                    })
            };
            match division {
                Some(division) => bound.knob_with_reading(
                    ui,
                    tokens,
                    live.setter,
                    size,
                    rect.width(),
                    live.text,
                    division.label(),
                ),
                None => bound.knob(ui, tokens, live.setter, size, rect.width(), live.text),
            }
        }
        Leaf::Picture(id) => {
            super::binding::sync_picture(ui, tokens, id, bound(id, params).param, live.setter);
        }
        Leaf::Onset | Leaf::Extent | Leaf::Reverse | Leaf::Size | Leaf::Decay => {
            let Some(actions) = live.actions.as_deref_mut() else {
                return;
            };
            if let Some(model) = preparation_edit(ui, tokens, live.text, *leaf, response) {
                (actions.edit)(model);
            }
        }
        Leaf::Prepared => {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(response.prepared())
                        .color(tokens.text_secondary)
                        .text_style(mxm_ui::typography::caption_style(ui.style())),
                )
                .wrap(),
            );
        }
        Leaf::TailShape => {
            let Some(actions) = live.actions.as_deref_mut() else {
                return;
            };
            let requested = requested_model(ui, response);
            let options = TAIL_SHAPES.map(|(shape, wave)| (wave, tail_shape_label(shape)));
            let mut selected = TAIL_SHAPES
                .iter()
                .position(|&(shape, _)| shape == requested.preparation.tail_shape)
                .unwrap_or(0);
            let changed = mxm_ui::navigation::at(ui, "response-tailshape", |ui| {
                mxm_ui::control::segmented_waves(
                    ui,
                    tokens,
                    TAIL_SHAPE,
                    &options,
                    &mut selected,
                    None,
                    Some(0),
                    None,
                    &TAIL_SHAPE_DETAILS,
                )
            });
            if changed {
                let mut model = requested;
                model.preparation.tail_shape = TAIL_SHAPES[selected].0;
                (actions.edit)(model);
            }
        }
        Leaf::Damping => {
            let Some(actions) = live.actions.as_deref_mut() else {
                return;
            };
            let committed = f64::from(requested_model(ui, response).preparation.damping_percent);
            let preparing = preparing(Leaf::Damping, response);
            if let Some(value) = release_slider(ui, tokens, live.text, &preparing, committed) {
                let mut model = requested_model(ui, response);
                model.preparation.damping_percent = value as u16;
                (actions.edit)(model);
            }
        }
        Leaf::WetEq => {
            ui.label(WET_EQ);
        }
        Leaf::LinearStrip => linear_strip(ui, tokens, response),
        Leaf::OutputStatus => {
            let (status, off) = output_status(live.inputs);
            ui.label(egui::RichText::new(status).color(if off {
                tokens.text_secondary
            } else {
                tokens.success
            }));
        }
        Leaf::Fault => {
            ui.label(egui::RichText::new(FAULT).color(tokens.danger));
        }
        Leaf::Acknowledge => {
            if ui.button(ACKNOWLEDGE).clicked() {
                *live.clear_fault = true;
            }
        }
    }
}

/// One of the Time card's preparation controls, drawn and read: the model to submit when it
/// released or changed at once, from the editor's in-progress values.
fn preparation_edit(
    ui: &mut Ui,
    tokens: &Tokens,
    text: &mut HashMap<&'static str, Option<String>>,
    leaf: Leaf,
    response: &ResponseView,
) -> Option<ResponseModel> {
    let sample_rate = f64::from(response.sample_rate.max(1));
    let mut model = requested_model(ui, response);
    if matches!(leaf, Leaf::Reverse) {
        let mut reverse = model.preparation.reverse;
        let changed = mxm_ui::navigation::at(ui, "response-reverse", |ui| {
            mxm_ui::control::toggle(ui, tokens, REVERSE, &mut reverse, false, REVERSE_ABOUT)
        });
        if !changed {
            return None;
        }
        model.preparation.reverse = reverse;
        return Some(model);
    }
    let preparing = preparing(leaf, response);
    let committed = match leaf {
        Leaf::Onset => f64::from(model.preparation.onset) * 1_000.0 / sample_rate,
        Leaf::Extent => {
            let available = response.frames.saturating_sub(model.preparation.onset);
            let extent = if model.preparation.extent == 0 {
                available
            } else {
                model.preparation.extent.min(available)
            };
            f64::from(extent.max(1)) * 1_000.0 / sample_rate
        }
        Leaf::Size => f64::from(model.preparation.time_percent),
        _ => f64::from(model.preparation.decay_percent),
    };
    let value = release_slider(ui, tokens, text, &preparing, committed)?;
    match leaf {
        Leaf::Onset => model.preparation.onset = (value * sample_rate / 1_000.0).round() as u32,
        Leaf::Extent => model.preparation.extent = (value * sample_rate / 1_000.0).round() as u32,
        Leaf::Size => model.preparation.time_percent = value as u16,
        _ => model.preparation.decay_percent = value as u16,
    }
    Some(model)
}

/// The linear response-shape strip: the source response is the filter, and nothing else is.
fn linear_strip(ui: &mut Ui, tokens: &Tokens, response: &ResponseView) {
    let width = ui.available_width();
    let (rect, semantic) =
        ui.allocate_exact_size(egui::vec2(width, STRIP_HEIGHT), egui::Sense::hover());
    semantic.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            "Linear response shape from source to convolution",
        )
    });
    semantic.on_hover_text("Your response as it will sound.");
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CANVAS_RADIUS, tokens.surface_2);
    painter.rect_stroke(
        rect,
        CANVAS_RADIUS,
        egui::Stroke::new(AXIS_STROKE, tokens.border),
        egui::StrokeKind::Inside,
    );
    let y = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left() + INNER_GUTTER, y),
            egui::pos2(rect.right() - INNER_GUTTER, y),
        ],
        egui::Stroke::new(TRACE_STROKE, tokens.accent),
    );
    painter.circle_filled(
        egui::pos2(
            rect.left() + rect.width() * response.duration_seconds().min(1.0),
            y,
        ),
        3.0,
        tokens.accent,
    );
}

fn response_status(ui: &mut Ui, tokens: &Tokens, response: &ResponseView, meters: MeterView) {
    let (status, colour) = response_status_presentation(tokens, response, meters);
    ui.add(egui::Label::new(egui::RichText::new(status).color(colour)).wrap());
}

/// The ink a response status is named in, as well as in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    Success,
    Warning,
    Secondary,
    Danger,
}

/// **What the response is doing, in the player's words** (the owner, 2026-09-28: the panel said
/// *Committed · embedded source and preparation are the active filter*). While another response
/// is being loaded, the one playing keeps playing, and the line says so.
fn response_status_text(response: &ResponseView, meters: MeterView) -> (String, Ink) {
    if let Some(rejection) = meters.response_rejection {
        return (rejection.message(), Ink::Danger);
    }
    match &response.status {
        LoadState::Idle => (RESPONSE_LOADED.to_owned(), Ink::Success),
        LoadState::Loading(name) if name == crate::editor::RESPONSE_EDIT => {
            (UPDATING.to_owned(), Ink::Warning)
        }
        LoadState::Loading(name) if name == crate::deferred::PRESET_EDIT => {
            (LOADING_PRESET.to_owned(), Ink::Warning)
        }
        LoadState::Loading(name) => (loading(name), Ink::Warning),
        LoadState::Ready(name) => (switching(name), Ink::Warning),
        LoadState::Information(message) => (message.clone(), Ink::Secondary),
        LoadState::Failed(message) => (
            format!("Could not load: {message}. The current response keeps playing."),
            Ink::Danger,
        ),
    }
}

const RESPONSE_LOADED: &str = "Response loaded";
const UPDATING: &str = "Updating the response · the current one keeps playing";
const LOADING_PRESET: &str = "Loading the preset · the current response keeps playing";

fn loading(name: &str) -> String {
    format!("Loading {name} · the current response keeps playing")
}

fn switching(name: &str) -> String {
    format!("Switching to {name}")
}

/// Every line [`response_status_text`] can say about `response` that is known before it is said:
/// the fixed lines and the ones naming it. What the Output and Response cards reserve (E1).
fn response_statuses(response: &ResponseView) -> Vec<String> {
    vec![
        RESPONSE_LOADED.to_owned(),
        UPDATING.to_owned(),
        LOADING_PRESET.to_owned(),
        loading(&response.name),
        switching(&response.name),
    ]
}

fn response_status_presentation(
    tokens: &Tokens,
    response: &ResponseView,
    meters: MeterView,
) -> (String, egui::Color32) {
    let (text, ink) = response_status_text(response, meters);
    let colour = match ink {
        Ink::Success => tokens.success,
        Ink::Warning => tokens.warning,
        Ink::Secondary => tokens.text_secondary,
        Ink::Danger => tokens.danger,
    };
    (text, colour)
}

/// Where a preparation slider marks a gesture still open — a drag, or an arrow held across frames —
/// so its held value outlives the frames between the gesture's start and its end.
fn gesture_open(id: egui::Id) -> egui::Id {
    id.with("gesture")
}

fn clear_finished_preparation_values(ui: &Ui, response: &ResponseView) {
    // Not on the frame a drag is let go either: the slider submits the value it holds on that
    // frame, and the pointer is already up.
    if ui.input(|input| input.pointer.primary_down() || input.pointer.any_released())
        || !matches!(
            response.status,
            LoadState::Idle | LoadState::Information(_) | LoadState::Failed(_)
        )
    {
        return;
    }
    ui.data_mut(|data| {
        for id in [
            "response-onset",
            "response-extent",
            "response-size",
            "response-decay",
            "response-damping",
        ] {
            let id = egui::Id::new(id);
            // **Never under an open gesture.** A held arrow keeps its gesture open with no pointer
            // down and the response idle, and clearing its value then made the key's release
            // submit the old one.
            if data.get_temp::<bool>(gesture_open(id)).is_none() {
                data.remove_temp::<f64>(id);
            }
        }
    });
}

fn requested_model(ui: &Ui, response: &ResponseView) -> ResponseModel {
    let mut model = response.model;
    let sample_rate = f64::from(response.sample_rate.max(1));
    let held = |id: &str| ui.data(|data| data.get_temp::<f64>(egui::Id::new(id)));
    if let Some(value) = held("response-onset") {
        model.preparation.onset = (value * sample_rate / 1_000.0).round() as u32;
    }
    if let Some(value) = held("response-extent") {
        model.preparation.extent = (value * sample_rate / 1_000.0).round() as u32;
    }
    if let Some(value) = held("response-size") {
        model.preparation.time_percent = value as u16;
    }
    if let Some(value) = held("response-decay") {
        model.preparation.decay_percent = value as u16;
    }
    if let Some(value) = held("response-damping") {
        model.preparation.damping_percent = value as u16;
    }
    model
}

/// A preparation value on the collection's slider (`control::slider`), **held locally while it is
/// dragged and submitted when it is let go**: each submission rebuilds the prepared response, so
/// one is sent per gesture rather than one per frame. A typed value, an arrow key or a double-click
/// is a gesture that begins and ends at once, and submits at once. Returns the value to submit.
fn release_slider(
    ui: &mut Ui,
    tokens: &Tokens,
    text: &mut HashMap<&'static str, Option<String>>,
    preparing: &Preparing,
    committed: f64,
) -> Option<f64> {
    let id = egui::Id::new(preparing.id);
    let mut value = ui
        .data(|data| data.get_temp::<f64>(id))
        .unwrap_or(committed);
    let reading = preparing.format(value);
    let widest = preparing.widest();
    let view = ParamView {
        name: preparing.name,
        label: preparing.name,
        text: &reading,
        widest: &widest,
        description: preparing.description,
        default: preparing.normalised(preparing.default),
        bipolar: false,
        read_only: false,
        modulation: 0.0,
        marked: false,
        steps: preparing.steps(),
        next: None,
        quiet_label: false,
    };
    let mut normalised = preparing.normalised(value);
    let entry = text.entry(preparing.id).or_default();
    // `Escape` abandons an open entry, as it does on every bound control (`Bound::entry`).
    if entry.is_some()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        *entry = None;
    }
    let width = ui.available_width();
    let outcome = mxm_ui::navigation::at(ui, preparing.id, |ui| {
        mxm_ui::control::slider(ui, tokens, &view, &mut normalised, width, entry, Wheel::Off)
    });
    if outcome.changed && entry.is_some() {
        // A committed text entry: parsed here, because this editor wrote the reading. Unparseable
        // text is discarded and the value stays.
        let parsed = entry.as_deref().and_then(|typed| preparing.parse(typed));
        *entry = None;
        return parsed;
    }
    if outcome.changed {
        value = preparing.value(normalised);
        ui.data_mut(|data| data.insert_temp(id, value));
    }
    if outcome.gesture_ended {
        ui.data_mut(|data| data.remove_temp::<bool>(gesture_open(id)));
    } else if outcome.gesture_started {
        ui.data_mut(|data| data.insert_temp(gesture_open(id), true));
    }
    outcome.gesture_ended.then_some(value)
}

fn response_plot_baseline(rect: egui::Rect, caption_height: f32) -> f32 {
    rect.bottom() - INNER_GUTTER - caption_height - AXIS_STROKE
}

fn response_plot(ui: &mut Ui, tokens: &Tokens, response: &ResponseView, meters: MeterView) {
    let width = ui.available_width().max(PLOT_MIN.x);
    let (rect, semantic) =
        ui.allocate_exact_size(egui::vec2(width, PLOT_MIN.y), egui::Sense::hover());
    semantic.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            "Response energy over ten seconds and active tail energy",
        )
    });
    semantic.on_hover_text(format!(
        "Energy on a fixed ten-second axis, one bar per 50 ms: pale is the peak and solid the RMS \
         level, from 0 dB at the top to {DISPLAY_FLOOR_DB:.0} dB at the base, with guides every \
         second and every 24 dB. The frame marks the region in use; the live marker shows output \
         energy without feeding it back to audio."
    ));
    let caption_style = mxm_ui::typography::caption_style(ui.style());
    let caption_height = ui.text_style_height(&caption_style);
    let caption = caption_style.resolve(ui.style());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CANVAS_RADIUS, tokens.surface_2);
    painter.rect_stroke(
        rect,
        CANVAS_RADIUS,
        egui::Stroke::new(AXIS_STROKE, tokens.border),
        egui::StrokeKind::Inside,
    );
    let left = rect.left() + INNER_GUTTER;
    let right = rect.right() - INNER_GUTTER;
    let top = rect.top() + INNER_GUTTER;
    let baseline = response_plot_baseline(rect, caption_height);
    let span = right - left;
    let level =
        |db: f32| baseline - (1.0 - db / DISPLAY_FLOOR_DB).clamp(0.0, 1.0) * (baseline - top);
    let guide = egui::Stroke::new(AXIS_STROKE, tokens.border);
    for second in 1..MAX_SOURCE_SECONDS as usize {
        let x = left + span * second as f32 / MAX_SOURCE_SECONDS;
        painter.line_segment([egui::pos2(x, top), egui::pos2(x, baseline)], guide);
    }
    for db in [-24.0, -48.0] {
        let y = level(db);
        painter.line_segment([egui::pos2(left, y), egui::pos2(right, y)], guide);
    }
    let bin_width = span / DISPLAY_BINS as f32;
    for (bin, [peak, rms]) in response.energy.iter().copied().enumerate() {
        if peak <= DISPLAY_FLOOR_DB {
            continue;
        }
        let x0 = left + bin_width * bin as f32;
        let x1 = x0 + bin_width;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, level(peak)), egui::pos2(x1, baseline)),
            0.0,
            tokens.accent.gamma_multiply(0.30),
        );
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, level(rms)), egui::pos2(x1, baseline)),
            0.0,
            tokens.accent,
        );
    }
    painter.line_segment(
        [egui::pos2(left, baseline), egui::pos2(right, baseline)],
        egui::Stroke::new(AXIS_STROKE, tokens.border_strong),
    );
    let axis_frames = response.sample_rate.max(1) as f32 * MAX_SOURCE_SECONDS;
    let onset = response.model.preparation.onset as f32 / axis_frames;
    let extent = response.prepared_extent_frames().max(1) as f32 / axis_frames;
    painter.rect_stroke(
        egui::Rect::from_x_y_ranges(
            (left + span * onset.min(1.0))..=(left + span * (onset + extent).min(1.0)),
            rect.top() + INNER_GUTTER..=baseline,
        ),
        CANVAS_RADIUS,
        egui::Stroke::new(TRACE_STROKE, tokens.accent),
        egui::StrokeKind::Inside,
    );
    painter.text(
        egui::pos2(left, baseline + AXIS_STROKE),
        egui::Align2::LEFT_TOP,
        "0 s",
        caption.clone(),
        tokens.text_secondary,
    );
    painter.text(
        egui::pos2(right, baseline + AXIS_STROKE),
        egui::Align2::RIGHT_TOP,
        format!("{MAX_SOURCE_SECONDS:.0} s"),
        caption,
        tokens.text_secondary,
    );
    let live = meters.peak.clamp(0.0, 1.0);
    painter.line_segment(
        [
            egui::pos2(left, baseline),
            egui::pos2(left, baseline - live * (baseline - top)),
        ],
        egui::Stroke::new(EMPHASIS_STROKE, tokens.success),
    );
}

fn accept_drop(ui: &Ui, rect: egui::Rect, load: &mut dyn FnMut(PathBuf)) {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    if !pointer.is_some_and(|position| rect.contains(position)) {
        return;
    }
    let path = ui.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_path_buf())
            .next()
    });
    if let Some(path) = path {
        load(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug::prelude::Params;

    #[test]
    fn revision_eight_surface_binds_all_nine_live_parameters_once() {
        let params = MxmFxConvolutionParams::default();
        let ids: Vec<_> = all_parameters(&params)
            .iter()
            .map(|binding| binding.id)
            .collect();
        assert_eq!(
            ids,
            [
                "mix",
                "predelay",
                "lowcut",
                "highcut",
                "tone",
                "width",
                "modulation",
                "feedback",
                "predelaysync"
            ]
        );
        assert_eq!(params.param_map().len(), 9);
    }

    /// **An edit on the panel says it is updating the response, and a preset load says so too.**
    /// Before 2026-09-28 every label gained " model" on its way in, so the edit's own line was
    /// never reached and the panel read *Preparing response change model*.
    #[test]
    fn a_loading_response_is_named_in_words() {
        let params = MxmFxConvolutionParams::default();
        let meters = MeterView {
            peak: 0.0,
            tail_samples: 0,
            numeric_fault: false,
            response_rejection: None,
            tempo: None,
        };
        for (label, expected) in [
            (crate::editor::RESPONSE_EDIT, UPDATING.to_owned()),
            (crate::deferred::PRESET_EDIT, LOADING_PRESET.to_owned()),
            ("Hall.wav", loading("Hall.wav")),
        ] {
            let _ = params.response.begin_edit(label);
            let response = ResponseView::read(&params);
            assert_eq!(
                response_status_text(&response, meters).0,
                expected,
                "{label}"
            );
        }
        let _ = params.response.begin_edit("Hall.wav");
        let mut response = ResponseView::read(&params);
        response.status = LoadState::Idle;
        assert_eq!(response_status_text(&response, meters).0, "Response loaded");
    }

    #[test]
    fn clamp_information_is_neutral_while_a_real_refusal_remains_danger() {
        let params = MxmFxConvolutionParams::default();
        let mut response = ResponseView::read(&params);
        response.status = LoadState::Information(
            "Too long to run here; Size set to the longest that fits".to_owned(),
        );
        let meters = MeterView {
            peak: 0.0,
            tail_samples: 0,
            numeric_fault: false,
            response_rejection: None,
            tempo: None,
        };
        let (message, colour) = response_status_presentation(&mxm_ui::LIGHT, &response, meters);
        assert_eq!(
            message,
            "Too long to run here; Size set to the longest that fits"
        );
        assert_eq!(colour, mxm_ui::LIGHT.text_secondary);
        assert_ne!(colour, mxm_ui::LIGHT.danger);

        let refusal = MeterView {
            response_rejection: Some(ResponseRejection::UnsupportedRate(768_000.0)),
            tempo: None,
            ..meters
        };
        let (message, colour) = response_status_presentation(&mxm_ui::LIGHT, &response, refusal);
        assert!(message.starts_with("Off at"));
        assert_eq!(colour, mxm_ui::LIGHT.danger);
    }

    #[test]
    fn painted_response_axis_captions_stay_inside_their_clip_rect() {
        let params = MxmFxConvolutionParams::default();
        let response = ResponseView::read(&params);
        let meters = MeterView {
            peak: 0.0,
            tail_samples: 0,
            numeric_fault: false,
            response_rejection: None,
            tempo: None,
        };
        let ctx = egui::Context::default();
        mxm_ui::typography::apply(&ctx);
        mxm_ui::theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(test_floors()[0], 200.0),
            )),
            ..Default::default()
        };
        let mut output = None;
        for _ in 0..3 {
            let mut frame = ctx.run_ui(input.clone(), |ui| {
                response_plot(ui, &mxm_ui::LIGHT, &response, meters);
            });
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let mut captions = 0;
        for clipped in output.unwrap().shapes {
            if let egui::Shape::Text(text) = &clipped.shape
                && matches!(text.galley.text(), "0 s" | "10 s")
            {
                let bounds = text.galley.rect.translate(text.pos.to_vec2());
                assert!(
                    clipped.clip_rect.contains_rect(bounds),
                    "{} was cropped: {bounds:?} outside {:?}",
                    text.galley.text(),
                    clipped.clip_rect
                );
                captions += 1;
            }
        }
        assert_eq!(captions, 2);
    }

    #[test]
    fn response_axis_reserves_the_complete_caption_inside_the_canvas() {
        let rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(test_floors()[0], TALL_PLOT_HEIGHT),
        );
        let caption_height = 15.0;
        let baseline = response_plot_baseline(rect, caption_height);
        let caption_bottom = baseline + AXIS_STROKE + caption_height;
        assert_eq!(caption_bottom, rect.bottom() - INNER_GUTTER);
        assert!(baseline > rect.top() + INNER_GUTTER);
    }

    #[test]
    fn response_and_time_are_the_only_preferred_pair() {
        assert_eq!(
            GROUPS,
            &[
                &[mxm_ui::paging::Key(0), mxm_ui::paging::Key(1)][..],
                &[mxm_ui::paging::Key(2)][..],
                &[mxm_ui::paging::Key(3)][..],
            ]
        );
        assert_eq!(test_items().len(), 4);
    }
}
