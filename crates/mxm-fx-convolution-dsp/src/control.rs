//! Bounded sample-offset live controls and Mix-zero park/wake transitions.
//!
//! Response preparation edits are intentionally absent: they rebuild immutable coefficients off the
//! audio thread and are durable model state, not automatable controls. This queue is only for the
//! linear run-time controls whose final plugin ranges are fixed at the listening gate.
//!
//! Revision 5's shipped controls remain in [`LiveControls`], less its wet gain, deleted on 2026-09-28:
//! the plugin normalises every response, so Mix is the one level. Revision 6's neutral-by-default
//! post-convolution controls live in [`WetPostControlState`], so extending the engine does not
//! change the existing response snapshot or constructor contract.

/// The quantities currently known to be live DSP controls, in DSP-native units.
///
/// This is not a permanent plugin parameter inventory. The C4 listening gate owns parameter ids,
/// useful ranges, and whether additional LTI wet-tone controls earn a place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveControls {
    /// Linear dry/wet crossfade. Exactly zero is Off.
    pub mix: f32,
    /// Pre-delay in samples after conversion at the active processing rate.
    pub pre_delay_samples: u64,
}

impl Default for LiveControls {
    fn default() -> Self {
        Self {
            mix: 0.5,
            pre_delay_samples: 0,
        }
    }
}

impl LiveControls {
    #[inline]
    pub fn apply(&mut self, change: ControlChange) {
        match change {
            ControlChange::Mix(value) => {
                if value.is_finite() {
                    self.mix = value.clamp(0.0, 1.0);
                }
            }
            ControlChange::PreDelaySamples(value) => self.pre_delay_samples = value,
            ControlChange::LowCutHz(_)
            | ControlChange::HighCutHz(_)
            | ControlChange::ToneDb(_)
            | ControlChange::Modulation(_)
            | ControlChange::Width(_)
            | ControlChange::Feedback(_) => {}
        }
    }
}

/// Internal exact-bypass sentinel retained for the Revision 6 compatibility path. Public plugin
/// controls never expose it: their High cut is a real 0–20 kHz frequency.
pub(crate) const HIGH_CUT_BYPASS_HZ: f32 = -1.0;

/// Linear Wet EQ controls. Zero low cut, the private negative high-cut sentinel and zero tone are
/// an exact bypass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WetEqControls {
    pub low_cut_hz: f32,
    /// Hertz; zero is the explicit open/bypass sentinel.
    pub high_cut_hz: f32,
    pub tone_db: f32,
}

impl Default for WetEqControls {
    fn default() -> Self {
        Self {
            low_cut_hz: 0.0,
            high_cut_hz: HIGH_CUT_BYPASS_HZ,
            tone_db: 0.0,
        }
    }
}

impl WetEqControls {
    pub fn is_neutral(self) -> bool {
        self == Self::default()
    }

    fn sanitised(self, fallback: Self) -> Self {
        Self {
            low_cut_hz: finite_nonnegative(self.low_cut_hz, fallback.low_cut_hz),
            high_cut_hz: {
                let value = finite(self.high_cut_hz, fallback.high_cut_hz);
                if value < 0.0 {
                    HIGH_CUT_BYPASS_HZ
                } else {
                    value
                }
            },
            tone_db: finite(self.tone_db, fallback.tone_db),
        }
    }
}

/// Revision 6's live post-convolution targets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WetPostControls {
    pub wet_eq: WetEqControls,
    /// Normalized one-knob movement amount. Zero is an exact bypass.
    pub modulation: f32,
    /// Mid/side side gain. One reconstructs the unchanged left/right pair.
    pub width: f32,
}

impl Default for WetPostControls {
    fn default() -> Self {
        Self {
            wet_eq: WetEqControls::default(),
            modulation: 0.0,
            width: 1.0,
        }
    }
}

impl WetPostControls {
    pub fn is_neutral(self) -> bool {
        self == Self::default()
    }

    fn sanitised(self, fallback: Self) -> Self {
        Self {
            wet_eq: self.wet_eq.sanitised(fallback.wet_eq),
            modulation: finite(self.modulation, fallback.modulation).clamp(0.0, 1.0),
            width: finite_nonnegative(self.width, fallback.width),
        }
    }
}

/// Per-sample values consumed by Revision 6's later audio-path checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WetPostControlFrame {
    pub wet_eq: WetEqControls,
    pub modulation: f32,
    pub width: f32,
}

/// Conservative history owned by the later post-convolution stages. Width is instantaneous and has
/// no entry. Values are configured from measured audio-stage constants, not guessed here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WetPostTailConfig {
    pub wet_eq_settle_samples: u64,
    pub modulation_history_samples: u64,
}

#[derive(Debug, Clone, Copy)]
struct LinearRamp {
    current: f32,
    target: f32,
    step: f32,
    remaining: u64,
    ramp_samples: u64,
}

impl LinearRamp {
    fn new(value: f32, ramp_samples: u64) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
            ramp_samples: ramp_samples.max(1),
        }
    }

    fn set_target(&mut self, target: f32) {
        if target == self.target {
            return;
        }
        self.target = target;
        self.remaining = self.ramp_samples;
        self.step = (target - self.current) / self.remaining as f32;
    }

    #[inline]
    fn next(&mut self) -> f32 {
        if self.remaining != 0 {
            self.current += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
                self.step = 0.0;
            }
        }
        self.current
    }

    fn settle(&mut self) {
        self.current = self.target;
        self.step = 0.0;
        self.remaining = 0;
    }
}

/// One sample of the Revision 7 loop control. `q` is the response-relative small-signal spectral
/// coordinate; the audio core converts it to its response-specific return coefficient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeedbackControlFrame {
    pub q: f32,
}

/// Bounded, allocation-free smoothing for Revision 7 Feedback.
///
/// The zero state is represented structurally by [`None`] from [`Self::next_frame`]. The later
/// recursive core can therefore bypass its return routing, multiply, saturator, and delay read
/// without relying on multiplication by zero.
#[derive(Debug, Clone, Copy)]
pub struct FeedbackControlState {
    target: f32,
    q: LinearRamp,
}

impl FeedbackControlState {
    pub fn new(ramp_samples: u64) -> Self {
        Self {
            target: 0.0,
            q: LinearRamp::new(0.0, ramp_samples),
        }
    }

    pub const fn target(&self) -> f32 {
        self.target
    }

    /// Samples remaining in the current coefficient ramp. The audio activity calculation composes
    /// this bounded remainder with recursive and response-transition horizons.
    pub const fn ramp_remaining(&self) -> u64 {
        self.q.remaining
    }

    /// Whether automation is still lowering the response-relative coefficient. Response
    /// crossfades pause for this interval so the moving bridge is reached before its bounds mix.
    pub const fn is_ramping_down(&self) -> bool {
        self.q.remaining != 0 && self.q.target < self.q.current
    }

    pub fn apply(&mut self, change: ControlChange) {
        let ControlChange::Feedback(value) = change else {
            return;
        };
        if value.is_finite() {
            self.target = value.max(0.0);
            self.q.set_target(self.target);
        }
    }

    pub fn is_exact_bypass(&self) -> bool {
        self.target == 0.0 && self.q.current == 0.0
    }

    /// Return `None` only for the exact zero branch. A ramp which has just reached zero still
    /// returns one final zero frame so the audio core can invalidate recursive history at that
    /// precise sample; the following sample is bypassed structurally.
    #[inline]
    pub fn next_frame(&mut self) -> Option<FeedbackControlFrame> {
        if self.is_exact_bypass() {
            None
        } else {
            Some(FeedbackControlFrame { q: self.q.next() })
        }
    }

    /// Reset, park, and wake retain the selected parameter but discard stale in-flight smoothing.
    /// Recursive history itself belongs to the audio core and is cleared separately.
    pub fn reset(&mut self) {
        self.q.settle();
    }

    pub fn park(&mut self) {
        self.reset();
    }
}

/// Bounded, allocation-free smoothing for the Revision 6 live controls.
#[derive(Debug, Clone, Copy)]
pub struct WetPostControlState {
    targets: WetPostControls,
    low_cut: LinearRamp,
    high_cut: LinearRamp,
    tone: LinearRamp,
    modulation: LinearRamp,
    width: LinearRamp,
}

impl WetPostControlState {
    pub fn new(ramp_samples: u64) -> Self {
        let neutral = WetPostControls::default();
        Self {
            targets: neutral,
            low_cut: LinearRamp::new(neutral.wet_eq.low_cut_hz, ramp_samples),
            high_cut: LinearRamp::new(neutral.wet_eq.high_cut_hz, ramp_samples),
            tone: LinearRamp::new(neutral.wet_eq.tone_db, ramp_samples),
            modulation: LinearRamp::new(neutral.modulation, ramp_samples),
            width: LinearRamp::new(neutral.width, ramp_samples),
        }
    }

    pub const fn targets(&self) -> WetPostControls {
        self.targets
    }

    pub fn apply(&mut self, change: ControlChange) {
        let mut target = self.targets;
        match change {
            ControlChange::LowCutHz(value) => target.wet_eq.low_cut_hz = value,
            ControlChange::HighCutHz(value) => target.wet_eq.high_cut_hz = value,
            ControlChange::ToneDb(value) => target.wet_eq.tone_db = value,
            ControlChange::Modulation(value) => target.modulation = value,
            ControlChange::Width(value) => target.width = value,
            ControlChange::Mix(_)
            | ControlChange::PreDelaySamples(_)
            | ControlChange::Feedback(_) => return,
        }
        target = target.sanitised(self.targets);
        self.targets = target;
        self.low_cut.set_target(target.wet_eq.low_cut_hz);
        self.high_cut.set_target(target.wet_eq.high_cut_hz);
        self.tone.set_target(target.wet_eq.tone_db);
        self.modulation.set_target(target.modulation);
        self.width.set_target(target.width);
    }

    #[inline]
    pub fn next_frame(&mut self) -> WetPostControlFrame {
        WetPostControlFrame {
            wet_eq: WetEqControls {
                low_cut_hz: self.low_cut.next(),
                high_cut_hz: self.high_cut.next(),
                tone_db: self.tone.next(),
            },
            modulation: self.modulation.next(),
            width: self.width.next(),
        }
    }

    /// Reset/activation starts from selected targets, never stale in-flight coefficients.
    pub fn reset(&mut self) {
        self.low_cut.settle();
        self.high_cut.settle();
        self.tone.settle();
        self.modulation.settle();
        self.width.settle();
    }

    pub fn tail_allowance(&self, config: WetPostTailConfig) -> u64 {
        let eq_active = self.targets.wet_eq != WetEqControls::default()
            || self.low_cut.current != 0.0
            || self.high_cut.current != HIGH_CUT_BYPASS_HZ
            || self.tone.current != 0.0;
        let modulation_active = self.targets.modulation != 0.0 || self.modulation.current != 0.0;
        let eq = if eq_active {
            config.wet_eq_settle_samples.saturating_add(
                self.low_cut
                    .remaining
                    .max(self.high_cut.remaining)
                    .max(self.tone.remaining),
            )
        } else {
            0
        };
        let modulation = if modulation_active {
            config
                .modulation_history_samples
                .saturating_add(self.modulation.remaining)
        } else {
            0
        };
        // The stages are serial: the modulation line may delay the EQ's latest settled sample.
        eq.saturating_add(modulation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlChange {
    Mix(f32),
    PreDelaySamples(u64),
    LowCutHz(f32),
    HighCutHz(f32),
    ToneDb(f32),
    Modulation(f32),
    Width(f32),
    /// Response-relative small-signal spectral coordinate. Zero is exact bypass.
    Feedback(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedControl {
    pub sample_offset: usize,
    pub change: ControlChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlQueueError {
    Full,
    OffsetOutsideBlock,
    ProcessingStarted,
}

/// A fixed-capacity block timeline.
///
/// Events are ordered by sample offset. Equal-offset events retain insertion order, so a later host
/// event at the same sample deterministically wins when changes are applied in iteration order.
/// Storage is caller-selected and never allocates.
pub struct ControlBlock<const N: usize> {
    block_len: usize,
    events: [Option<TimedControl>; N],
    len: usize,
    cursor: usize,
}

impl<const N: usize> ControlBlock<N> {
    pub const fn new(block_len: usize) -> Self {
        Self {
            block_len,
            events: [None; N],
            len: 0,
            cursor: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn block_len(&self) -> usize {
        self.block_len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, event: TimedControl) -> Result<(), ControlQueueError> {
        if self.cursor != 0 {
            return Err(ControlQueueError::ProcessingStarted);
        }
        if event.sample_offset >= self.block_len {
            return Err(ControlQueueError::OffsetOutsideBlock);
        }
        if self.len == N {
            return Err(ControlQueueError::Full);
        }

        // Strict `>` preserves insertion order for equal offsets.
        let mut insertion = self.len;
        while insertion > 0 {
            let previous =
                self.events[insertion - 1].expect("the initialized prefix contains only events");
            if previous.sample_offset <= event.sample_offset {
                break;
            }
            self.events[insertion] = Some(previous);
            insertion -= 1;
        }
        self.events[insertion] = Some(event);
        self.len += 1;
        Ok(())
    }
}

impl<const N: usize> Iterator for ControlBlock<N> {
    type Item = TimedControl;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor == self.len {
            return None;
        }
        let event = self.events[self.cursor];
        self.cursor += 1;
        event
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.cursor;
        (remaining, Some(remaining))
    }
}

impl<const N: usize> ExactSizeIterator for ControlBlock<N> {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateAction {
    None,
    /// Clear histories before processing the first re-engaged sample.
    ResetAndWake,
    /// Clear histories after this zero-gain sample and stop running the wet engine.
    Park,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateTick {
    pub mix: f32,
    pub action: GateAction,
}

/// Linear Mix smoothing with the collection's exact-zero Off lifecycle.
///
/// The caller supplies the measured fade length; this checkpoint does not invent one. A wake from
/// Park requests one reset before any wet processing. A fade to zero requests one park exactly when
/// the effective Mix reaches zero. Reversing a fade before zero keeps valid history and does not
/// reset it.
#[derive(Debug, Clone, Copy)]
pub struct MixGate {
    current: f32,
    target: f32,
    step: f32,
    remaining: u64,
    ramp_samples: u64,
    parked: bool,
}

impl MixGate {
    pub const fn parked(ramp_samples: u64) -> Self {
        Self {
            current: 0.0,
            target: 0.0,
            step: 0.0,
            remaining: 0,
            ramp_samples: if ramp_samples == 0 { 1 } else { ramp_samples },
            parked: true,
        }
    }

    pub fn active(mix: f32, ramp_samples: u64) -> Self {
        let mix = finite_mix(mix, 0.0);
        if mix == 0.0 {
            return Self::parked(ramp_samples);
        }
        Self {
            current: mix,
            target: mix,
            step: 0.0,
            remaining: 0,
            ramp_samples: ramp_samples.max(1),
            parked: false,
        }
    }

    pub const fn is_parked(&self) -> bool {
        self.parked
    }

    pub const fn current_mix(&self) -> f32 {
        self.current
    }

    pub fn set_target(&mut self, mix: f32) -> GateAction {
        let mix = finite_mix(mix, self.target);
        if self.parked && mix == 0.0 {
            return GateAction::None;
        }

        let action = if self.parked {
            self.parked = false;
            self.current = 0.0;
            GateAction::ResetAndWake
        } else {
            GateAction::None
        };
        self.target = mix;
        self.remaining = self.ramp_samples;
        self.step = (self.target - self.current) / self.remaining as f32;
        action
    }

    #[inline]
    pub fn next_sample(&mut self) -> GateTick {
        let mut action = GateAction::None;
        if self.remaining != 0 {
            self.current += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
                self.step = 0.0;
                if self.target == 0.0 && !self.parked {
                    self.parked = true;
                    action = GateAction::Park;
                }
            }
        }
        GateTick {
            mix: self.current,
            action,
        }
    }

    /// Panic/reset clears any pending transition and leaves exact Off silence.
    pub fn reset(&mut self) {
        self.current = 0.0;
        self.target = 0.0;
        self.step = 0.0;
        self.remaining = 0;
        self.parked = true;
    }
}

#[inline]
fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[inline]
fn finite_nonnegative(value: f32, fallback: f32) -> f32 {
    finite(value, fallback).max(0.0)
}

#[inline]
fn finite_mix(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_orders_offsets_and_preserves_equal_offset_order() {
        let mut block = ControlBlock::<5>::new(16);
        block
            .push(TimedControl {
                sample_offset: 9,
                change: ControlChange::PreDelaySamples(2),
            })
            .unwrap();
        block
            .push(TimedControl {
                sample_offset: 3,
                change: ControlChange::Mix(0.25),
            })
            .unwrap();
        block
            .push(TimedControl {
                sample_offset: 3,
                change: ControlChange::Mix(0.75),
            })
            .unwrap();

        let ordered: Vec<_> = block.collect();
        assert_eq!(ordered[0].sample_offset, 3);
        assert_eq!(ordered[1].sample_offset, 3);
        assert_eq!(ordered[2].sample_offset, 9);
        assert_eq!(ordered[0].change, ControlChange::Mix(0.25));
        assert_eq!(ordered[1].change, ControlChange::Mix(0.75));
    }

    #[test]
    fn block_is_bounded_and_cannot_change_after_processing_starts() {
        let mut block = ControlBlock::<1>::new(8);
        assert_eq!(
            block.push(TimedControl {
                sample_offset: 8,
                change: ControlChange::Mix(1.0),
            }),
            Err(ControlQueueError::OffsetOutsideBlock)
        );
        block
            .push(TimedControl {
                sample_offset: 0,
                change: ControlChange::Mix(1.0),
            })
            .unwrap();
        assert_eq!(
            block.push(TimedControl {
                sample_offset: 1,
                change: ControlChange::Mix(0.0),
            }),
            Err(ControlQueueError::Full)
        );
        assert!(block.next().is_some());
        assert_eq!(
            block.push(TimedControl {
                sample_offset: 1,
                change: ControlChange::Mix(0.0),
            }),
            Err(ControlQueueError::ProcessingStarted)
        );
    }

    #[test]
    fn same_sample_changes_apply_in_order_and_non_finite_values_retain_state() {
        let mut controls = LiveControls::default();
        for change in [
            ControlChange::Mix(0.25),
            ControlChange::Mix(0.75),
            ControlChange::Mix(f32::NAN),
        ] {
            controls.apply(change);
        }
        assert_eq!(controls.mix, 0.75);
    }

    #[test]
    fn feedback_zero_is_structural_bypass_and_nonzero_changes_are_bounded_ramps() {
        let mut state = FeedbackControlState::new(4);
        assert!(state.is_exact_bypass());
        assert_eq!(state.next_frame(), None);

        state.apply(ControlChange::Feedback(2.0));
        assert_eq!(state.target(), 2.0);
        assert_eq!(state.next_frame(), Some(FeedbackControlFrame { q: 0.5 }));
        assert_eq!(state.next_frame(), Some(FeedbackControlFrame { q: 1.0 }));
        assert_eq!(state.next_frame(), Some(FeedbackControlFrame { q: 1.5 }));
        assert_eq!(state.next_frame(), Some(FeedbackControlFrame { q: 2.0 }));

        state.apply(ControlChange::Feedback(0.0));
        for expected in [1.5, 1.0, 0.5, 0.0] {
            assert_eq!(
                state.next_frame(),
                Some(FeedbackControlFrame { q: expected })
            );
        }
        assert!(state.is_exact_bypass());
        assert_eq!(state.next_frame(), None);
    }

    #[test]
    fn feedback_numeric_reset_and_park_contracts_are_deterministic() {
        let mut state = FeedbackControlState::new(8);
        state.apply(ControlChange::Feedback(1.25));
        state.apply(ControlChange::Feedback(f32::NAN));
        state.apply(ControlChange::Feedback(f32::INFINITY));
        assert_eq!(state.target(), 1.25);
        assert_eq!(
            state.next_frame(),
            Some(FeedbackControlFrame { q: 0.15625 })
        );

        state.park();
        assert_eq!(state.next_frame(), Some(FeedbackControlFrame { q: 1.25 }));
        state.apply(ControlChange::Feedback(-4.0));
        state.reset();
        assert_eq!(state.target(), 0.0);
        assert_eq!(state.next_frame(), None);
    }

    #[test]
    fn wet_post_defaults_are_exact_neutral() {
        let controls = WetPostControls::default();
        assert!(controls.is_neutral());
        assert!(controls.wet_eq.is_neutral());
        assert_eq!(controls.modulation, 0.0);
        assert_eq!(controls.width, 1.0);
        assert_eq!(
            WetPostControlState::new(32).next_frame(),
            WetPostControlFrame {
                wet_eq: WetEqControls::default(),
                modulation: 0.0,
                width: 1.0,
            }
        );
    }

    #[test]
    fn wet_post_changes_smooth_and_same_sample_last_write_wins() {
        let mut block = ControlBlock::<3>::new(1);
        for change in [
            ControlChange::Width(0.25),
            ControlChange::Width(1.75),
            ControlChange::Modulation(0.8),
        ] {
            block
                .push(TimedControl {
                    sample_offset: 0,
                    change,
                })
                .unwrap();
        }
        let mut state = WetPostControlState::new(4);
        for event in block {
            state.apply(event.change);
        }
        assert_eq!(state.targets().width, 1.75);
        assert_eq!(state.targets().modulation, 0.8);
        let first = state.next_frame();
        assert_eq!(first.width, 1.1875);
        assert!((first.modulation - 0.2).abs() < 1.0e-6);
        for _ in 0..3 {
            state.next_frame();
        }
        assert_eq!(state.next_frame().width, 1.75);
        assert_eq!(state.next_frame().modulation, 0.8);
    }

    #[test]
    fn wet_post_numeric_seam_retains_finite_targets_and_clamps_domains() {
        let mut state = WetPostControlState::new(2);
        for change in [
            ControlChange::LowCutHz(80.0),
            ControlChange::LowCutHz(f32::NAN),
            ControlChange::HighCutHz(-1.0),
            ControlChange::ToneDb(-3.0),
            ControlChange::ToneDb(f32::INFINITY),
            ControlChange::Modulation(2.0),
            ControlChange::Width(-4.0),
        ] {
            state.apply(change);
        }
        assert_eq!(state.targets().wet_eq.low_cut_hz, 80.0);
        assert_eq!(state.targets().wet_eq.high_cut_hz, HIGH_CUT_BYPASS_HZ);
        assert_eq!(state.targets().wet_eq.tone_db, -3.0);
        assert_eq!(state.targets().modulation, 1.0);
        assert_eq!(state.targets().width, 0.0);
    }

    #[test]
    fn wet_post_tail_allowance_covers_only_stateful_stages() {
        let config = WetPostTailConfig {
            wet_eq_settle_samples: 400,
            modulation_history_samples: 128,
        };
        let mut state = WetPostControlState::new(20);
        assert_eq!(state.tail_allowance(config), 0);
        state.apply(ControlChange::Width(2.0));
        assert_eq!(state.tail_allowance(config), 0);
        state.apply(ControlChange::LowCutHz(100.0));
        assert_eq!(state.tail_allowance(config), 420);
        state.apply(ControlChange::Modulation(1.0));
        assert_eq!(state.tail_allowance(config), 568);
        for _ in 0..20 {
            state.next_frame();
        }
        assert_eq!(state.tail_allowance(config), 528);
    }

    #[test]
    fn wet_post_reset_settles_selected_values_deterministically() {
        let mut state = WetPostControlState::new(100);
        state.apply(ControlChange::HighCutHz(9_000.0));
        state.apply(ControlChange::ToneDb(2.5));
        state.apply(ControlChange::Modulation(0.6));
        state.apply(ControlChange::Width(1.4));
        state.next_frame();
        state.reset();
        assert_eq!(
            state.next_frame(),
            WetPostControlFrame {
                wet_eq: WetEqControls {
                    low_cut_hz: 0.0,
                    high_cut_hz: 9_000.0,
                    tone_db: 2.5,
                },
                modulation: 0.6,
                width: 1.4,
            }
        );
    }

    #[test]
    fn mix_zero_parks_once_and_reengagement_resets_once() {
        let mut gate = MixGate::active(0.8, 4);
        assert_eq!(gate.set_target(0.0), GateAction::None);
        for expected in [0.6, 0.4, 0.2] {
            let tick = gate.next_sample();
            assert!((tick.mix - expected).abs() < 1.0e-6);
            assert_eq!(tick.action, GateAction::None);
        }
        let final_tick = gate.next_sample();
        assert_eq!(final_tick.mix, 0.0);
        assert_eq!(final_tick.action, GateAction::Park);
        assert_eq!(gate.next_sample().action, GateAction::None);
        assert!(gate.is_parked());

        assert_eq!(gate.set_target(0.5), GateAction::ResetAndWake);
        assert_eq!(gate.set_target(0.75), GateAction::None);
        for _ in 0..4 {
            assert_ne!(gate.next_sample().action, GateAction::Park);
        }
        assert_eq!(gate.current_mix(), 0.75);
    }

    #[test]
    fn reversing_a_fade_before_zero_preserves_live_history() {
        let mut gate = MixGate::active(1.0, 4);
        gate.set_target(0.0);
        assert_eq!(gate.next_sample().mix, 0.75);
        assert_eq!(gate.set_target(0.5), GateAction::None);
        assert!(!gate.is_parked());
    }

    #[test]
    fn reset_is_deterministic_exact_off() {
        let mut gate = MixGate::active(0.7, 10);
        gate.set_target(0.2);
        gate.next_sample();
        gate.reset();
        assert!(gate.is_parked());
        assert_eq!(gate.current_mix(), 0.0);
        assert_eq!(
            gate.next_sample(),
            GateTick {
                mix: 0.0,
                action: GateAction::None
            }
        );
    }
}
