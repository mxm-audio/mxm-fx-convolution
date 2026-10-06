//! Live parameter definitions for `mxm-fx-convolution`.
//!
//! Mix and Pre-delay are the first shipped ids. Wet gain, the third, was deleted on 2026-09-28: every
//! response is normalised to one loudness (`response::loudness_scale`), so Mix is the one level
//! (the owner: *there are no old projects — just delete the id*). Wet post-convolution controls add
//! new automatable ids. Size, Decay and Damping remain persisted response-model state because
//! rebuilding FIR spectra cannot be sample-accurate automation.

use crate::response::ResponseField;
use mxm_preset::PresetIdentity;
use nice_plug::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

pub const MAX_PRE_DELAY_SECONDS: f32 = 0.5;
pub const MIN_HIGH_CUT_HZ: f32 = 0.0;
/// Real maximum cutoff, displayed as Open at the conventional top of the High cut control.
pub const HIGH_CUT_OPEN_HZ: f32 = 20_000.0;
/// Pre-listening product ceiling for the response-relative Feedback coordinate. The focused R7
/// listening gate may lower this before release, but it must remain above the contraction mark.
pub const MAX_FEEDBACK: f32 = 1.25;
const PARAMETER_COUNT: usize = 9;
const OBSERVED_BITS_MARKER: u64 = 1 << 63;

struct ParameterEditRevision {
    observed_bits: AtomicU64,
    revision: AtomicU64,
}

pub(crate) struct ParameterEditRevisions {
    values: [ParameterEditRevision; PARAMETER_COUNT],
}

impl ParameterEditRevisions {
    fn new() -> Self {
        Self {
            values: std::array::from_fn(|_| ParameterEditRevision {
                observed_bits: AtomicU64::new(0),
                revision: AtomicU64::new(0),
            }),
        }
    }

    fn sample(&self, index: usize, normalised: f32) -> u64 {
        let value = &self.values[index];
        let encoded = OBSERVED_BITS_MARKER | u64::from(normalised.to_bits());
        let previous = value.observed_bits.swap(encoded, Ordering::AcqRel);
        if previous != 0 && previous != encoded {
            value.revision.fetch_add(1, Ordering::AcqRel);
        }
        value.revision.load(Ordering::Acquire)
    }
}

type ValueToString = Arc<dyn Fn(f32) -> String + Send + Sync>;
type StringToValue = Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>;

fn percent_to_string() -> ValueToString {
    Arc::new(|value| format!("{:.0} %", value * 100.0))
}

fn string_to_percent() -> StringToValue {
    Arc::new(|text| {
        text.trim()
            .trim_end_matches('%')
            .trim()
            .parse::<f32>()
            .ok()
            .map(|value| value / 100.0)
    })
}

fn time_to_string() -> ValueToString {
    Arc::new(|seconds| format!("{:.0} ms", seconds * 1_000.0))
}

fn string_to_time() -> StringToValue {
    Arc::new(|text| {
        text.trim()
            .to_ascii_lowercase()
            .trim_end_matches("ms")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|milliseconds| milliseconds / 1_000.0)
    })
}

fn frequency_to_string(open_at_zero: bool) -> ValueToString {
    Arc::new(move |value| {
        if open_at_zero && value == 0.0 {
            "Open".to_owned()
        } else if value >= 1_000.0 {
            format!("{:.1} kHz", value / 1_000.0)
        } else {
            format!("{value} Hz")
        }
    })
}

fn high_cut_range() -> FloatRange {
    FloatRange::Skewed {
        min: MIN_HIGH_CUT_HZ,
        max: HIGH_CUT_OPEN_HZ,
        factor: FloatRange::skew_factor(-1.4),
    }
}

fn high_cut_to_string() -> ValueToString {
    Arc::new(|value| {
        if value >= HIGH_CUT_OPEN_HZ {
            "Open".to_owned()
        } else if value >= 1_000.0 {
            format!("{:.1} kHz", value / 1_000.0)
        } else {
            format!("{value:.0} Hz")
        }
    })
}

fn string_to_plain() -> StringToValue {
    Arc::new(|text| {
        text.trim()
            .trim_end_matches("dB")
            .trim()
            .parse::<f32>()
            .ok()
    })
}

fn parse_frequency(text: &str, open_value: f32) -> Option<f32> {
    let text = text.trim().to_ascii_lowercase();
    if text == "open" || text == "off" {
        return Some(open_value);
    }
    if let Some(value) = text.strip_suffix("khz") {
        return value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|value| value * 1_000.0);
    }
    text.trim_end_matches("hz").trim().parse::<f32>().ok()
}

fn string_to_frequency() -> StringToValue {
    Arc::new(|text| parse_frequency(text, 0.0))
}

fn string_to_high_cut() -> StringToValue {
    Arc::new(|text| parse_frequency(text, HIGH_CUT_OPEN_HZ))
}

/// Revision 6 stored High cut over a linear 0–20 kHz range with Open at normalized zero. Preserve
/// a loaded preset identity while moving Open to the top and making the active range perceptual.
pub(crate) fn migrate_legacy_high_cut_normalized(value: f32) -> f32 {
    let old_plain = value.clamp(0.0, 1.0) * HIGH_CUT_OPEN_HZ;
    if old_plain == 0.0 {
        1.0
    } else {
        high_cut_range().normalize(old_plain)
    }
}

/// **Pre-delay's tempo sync** (`plans/plan-tempo-sync-controls.md`): 1/64 to a quarter note, the
/// slice of the ladder the pre-delay's 0 – 500 ms holds at 120 bpm, the top the longest.
pub const PRE_DELAY_SYNC: mxm_tempo::Ladder = mxm_tempo::Ladder::new(
    mxm_tempo::Span::new(
        mxm_tempo::Division::SixtyFourth,
        mxm_tempo::Division::Quarter,
    ),
    mxm_tempo::Direction::Time,
);

#[derive(Params)]
pub struct MxmFxConvolutionParams {
    /// Linear dry/wet crossfade. Exactly zero is Off after the smoothing fade closes.
    #[id = "mix"]
    pub mix: FloatParam,
    /// Delay before the convolved leg. The DSP moves between taps without rebuilding the response.
    #[id = "predelay"]
    pub pre_delay: FloatParam,
    /// Linear high-pass stage on the convolved leg. Zero is an exact bypass.
    #[id = "lowcut"]
    pub low_cut: FloatParam,
    /// Linear low-pass stage on the convolved leg. Zero is fully closed; the real 20 kHz maximum
    /// is displayed as Open.
    #[id = "highcut"]
    pub high_cut: FloatParam,
    /// Small linear low/high tilt on the convolved leg.
    #[id = "tone"]
    pub tone: FloatParam,
    /// Mid/side gain over the convolved stereo output pair. One is exact neutral.
    #[id = "width"]
    pub width: FloatParam,
    /// Amount for the bounded post-convolution modulated delay. Zero is exact bypass.
    #[id = "modulation"]
    pub modulation: FloatParam,
    /// Response-relative raw-convolution return. Zero is an exact structural bypass.
    #[id = "feedback"]
    pub feedback: FloatParam,
    /// Pre-delay's tempo sync: its position picks a division of the host's tempo. Revision 8.
    #[id = "predelaysync"]
    pub pre_delay_sync: BoolParam,

    // Ignored by `Params`: sampled unmodulated bits advance these without treating host modulation
    // as a base edit. Process blocks, editor frames and transaction begin/publication are samples.
    pub(crate) edit_revisions: ParameterEditRevisions,

    /// Embedded canonical source coefficients. Paths and prepared FFT spectra are never state.
    #[persist = "response"]
    pub response: ResponseField,
    #[persist = "preset"]
    pub preset: RwLock<PresetIdentity>,
}

impl MxmFxConvolutionParams {
    /// Pre-delay while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`PRE_DELAY_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_pre_delay(&self, tempo: Option<f64>) -> Option<f32> {
        let param = &self.pre_delay;
        PRE_DELAY_SYNC
            .resolve(
                self.pre_delay_sync.value(),
                tempo,
                param.modulated_normalized_value(),
                f64::from(param.preview_plain(0.0)),
                f64::from(param.preview_plain(1.0)),
            )
            .map(|seconds| seconds as f32)
    }

    pub(crate) fn parameter_edit_revision(&self, id: &str) -> Option<u64> {
        let (index, value) = match id {
            "mix" => (0, self.mix.unmodulated_normalized_value()),
            "predelay" => (1, self.pre_delay.unmodulated_normalized_value()),
            "lowcut" => (2, self.low_cut.unmodulated_normalized_value()),
            "highcut" => (3, self.high_cut.unmodulated_normalized_value()),
            "tone" => (4, self.tone.unmodulated_normalized_value()),
            "width" => (5, self.width.unmodulated_normalized_value()),
            "modulation" => (6, self.modulation.unmodulated_normalized_value()),
            "feedback" => (7, self.feedback.unmodulated_normalized_value()),
            "predelaysync" => (8, self.pre_delay_sync.unmodulated_normalized_value()),
            _ => return None,
        };
        Some(self.edit_revisions.sample(index, value))
    }

    pub(crate) fn observe_parameter_edits(&self) {
        for id in [
            "mix",
            "predelay",
            "lowcut",
            "highcut",
            "tone",
            "width",
            "modulation",
            "feedback",
            "predelaysync",
        ] {
            let _ = self.parameter_edit_revision(id);
        }
    }
}

impl Default for MxmFxConvolutionParams {
    fn default() -> Self {
        Self {
            // An inserted effect demonstrates itself. These values and the generated response are
            // provisional voicing for the listening gate; the stable ids are not provisional.
            // The DSP's MixGate owns this fade and its exact-zero park transition. A second
            // parameter smoother would put two ramps in series.
            // 30 %: the old 38 % at a wet gain of -3 dB, folded into Mix against a normalised
            // response (`preset.rs`, `folded_mix`).
            mix: FloatParam::new("Mix", 0.30, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(percent_to_string())
                .with_string_to_value(string_to_percent()),
            // Predelay configures a moving read tap rather than multiplying audio. The DSP owns its
            // bounded tap crossfade, so a second parameter smoother would be two transitions.
            pre_delay: FloatParam::new(
                "Pre-delay",
                0.012,
                FloatRange::Skewed {
                    min: 0.0,
                    max: MAX_PRE_DELAY_SECONDS,
                    factor: FloatRange::skew_factor(-1.4),
                },
            )
            .with_value_to_string(time_to_string())
            .with_string_to_value(string_to_time()),
            // These five non-neutral ranges are pre-listening engineering values awaiting owner
            // approval. Their ids, meanings and exact neutral defaults are the compatibility lock.
            low_cut: FloatParam::new(
                "Low cut",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 4_000.0,
                },
            )
            .with_value_to_string(frequency_to_string(true))
            .with_string_to_value(string_to_frequency()),
            high_cut: FloatParam::new("High cut", HIGH_CUT_OPEN_HZ, high_cut_range())
                .with_value_to_string(high_cut_to_string())
                .with_string_to_value(string_to_high_cut()),
            tone: FloatParam::new(
                "Tone",
                0.0,
                FloatRange::Linear {
                    min: -6.0,
                    max: 6.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1))
            .with_string_to_value(string_to_plain()),
            width: FloatParam::new("Width", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 })
                .with_value_to_string(percent_to_string())
                .with_string_to_value(string_to_percent()),
            modulation: FloatParam::new(
                "Modulation",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(percent_to_string())
            .with_string_to_value(string_to_percent()),
            feedback: FloatParam::new(
                "Feedback",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: MAX_FEEDBACK,
                },
            )
            .with_value_to_string(Arc::new(|value| format!("{value:.2}")))
            .with_string_to_value(string_to_plain()),
            pre_delay_sync: BoolParam::new("Pre-delay sync", false),
            edit_revisions: ParameterEditRevisions::new(),
            response: ResponseField::default(),
            preset: RwLock::new(PresetIdentity::none()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Pre-delay's sync picks a division and is inert without a tempo**
    /// (`plans/plan-tempo-sync-controls.md`): off, or with no tempo, the knob's own time stands; on
    /// at 120 bpm the ends are the ladder's ends that the range can hold, the top the longest.
    #[test]
    fn pre_delay_sync_picks_a_division_and_is_inert_without_a_tempo() {
        use nice_plug::params::InternalParamMut;
        fn set<P: InternalParamMut>(param: &P, normalized: f32) {
            unsafe {
                let _ = param._internal_set_normalized_value(normalized);
            }
        }
        let p = MxmFxConvolutionParams::default();
        set(&p.pre_delay, 1.0);
        assert_eq!(
            p.synced_pre_delay(Some(120.0)),
            None,
            "off is the free time"
        );
        set(&p.pre_delay_sync, 1.0);
        assert_eq!(p.synced_pre_delay(None), None, "no tempo is the free time");

        let top = p.synced_pre_delay(Some(120.0)).expect("synced at a tempo");
        set(&p.pre_delay, 0.0);
        let bottom = p.synced_pre_delay(Some(120.0)).expect("synced at a tempo");
        let (lo, hi) = (
            f64::from(p.pre_delay.preview_plain(0.0)),
            f64::from(p.pre_delay.preview_plain(1.0)),
        );
        assert!(
            top > bottom,
            "the top of a time is the longest: {bottom} to {top}"
        );
        let reach = PRE_DELAY_SYNC.reachable(120.0, lo, hi).divisions();
        let shortest = reach[0].seconds(120.0) as f32;
        let longest = reach[reach.len() - 1].seconds(120.0) as f32;
        assert!(
            (bottom - shortest).abs() < 1e-5,
            "{bottom} against {shortest}"
        );
        assert!((top - longest).abs() < 1e-5, "{top} against {longest}");
    }

    use nice_plug::params::Param;

    #[test]
    fn effect_defaults_are_engaged_and_leave_headroom() {
        let params = MxmFxConvolutionParams::default();
        assert!(params.mix.default_plain_value() > 0.0);
        assert!(params.mix.default_plain_value() < 0.5);
        assert!(params.pre_delay.default_plain_value() >= 0.0);
        assert_eq!(params.low_cut.default_plain_value(), 0.0);
        assert_eq!(params.high_cut.default_plain_value(), HIGH_CUT_OPEN_HZ);
        assert_eq!(params.high_cut.default_normalized_value(), 1.0);
        assert_eq!(params.high_cut.preview_normalized(MIN_HIGH_CUT_HZ), 0.0);
        assert_eq!(
            params.high_cut.normalized_value_to_string(0.0, true),
            "0 Hz"
        );
        assert_eq!(
            params.high_cut.normalized_value_to_string(1.0, true),
            "Open"
        );
        assert_eq!(string_to_high_cut()("0 Hz"), Some(0.0));
        assert_eq!(params.tone.default_plain_value(), 0.0);
        assert_eq!(params.width.default_plain_value(), 1.0);
        assert_eq!(params.modulation.default_plain_value(), 0.0);
        assert_eq!(params.feedback.default_plain_value(), 0.0);
        assert!(params.feedback.preview_normalized(1.0) < 1.0);
        assert_eq!(params.feedback.preview_normalized(MAX_FEEDBACK), 1.0);
    }

    #[test]
    fn legacy_parameter_ids_stay_first_and_new_live_ids_are_stable() {
        let params = MxmFxConvolutionParams::default();
        let ids: Vec<_> = params
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
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
    }

    #[test]
    fn text_round_trips_are_idempotent() {
        let params = MxmFxConvolutionParams::default();
        for parameter in [
            &params.mix,
            &params.pre_delay,
            &params.low_cut,
            &params.high_cut,
            &params.tone,
            &params.width,
            &params.modulation,
            &params.feedback,
        ] {
            for step in 0..=20 {
                let normalized = step as f32 / 20.0;
                let text = parameter.normalized_value_to_string(normalized, false);
                let parsed = parameter
                    .string_to_normalized_value(&text)
                    .unwrap_or_else(|| panic!("{text:?} did not parse"));
                assert_eq!(
                    text,
                    parameter.normalized_value_to_string(parsed, false),
                    "{text:?} was not idempotent"
                );
            }
        }
    }

    #[test]
    fn response_is_persisted_but_is_not_a_parameter() {
        let params = MxmFxConvolutionParams::default();
        let mut response = params.response.snapshot();
        response.name = "State round trip".to_owned();
        nice_plug::params::persist::PersistentField::set(&params.response, response);
        let fields = params.serialize_fields();
        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            ["preset", "response"]
        );
        assert_eq!(params.param_map().len(), 9);

        let restored = MxmFxConvolutionParams::default();
        restored.deserialize_fields(&fields);
        assert_eq!(restored.response.snapshot().name, "State round trip");
    }
}
