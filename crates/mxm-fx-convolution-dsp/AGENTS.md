# AGENTS.md — mxm-fx-convolution-dsp

Parent: [`../../AGENTS.md`](../../AGENTS.md)

# Purpose

Framework-free, zero-dependency DSP for the original `mxm-fx-convolution` effect: explicit response
matrix routing, a zero-delay two-tier partitioned FIR, linear pre-delay, bounded response replacement,
the wet post-convolution EQ/modulated-delay/Width path, Mix, Revision 7's bounded
raw-response Feedback loop and finite/infinite activity, and deterministic reset.

The method and its evidence are `research:effects/convolution-reverb.md`; product architecture is
`../../plans/plan-mxm-fx-convolution.md` (`plans/plan-mxm-fx-convolution.md` in the private archive). No existing
convolver implementation or third-party impulse response was opened or copied.

# Ownership

- `src/convolver.rs`, `src/fft.rs`: finite response preparation, checked wet/composed-loop induced
  bounds, direct onset, distributed two-tier FFT tail and exact finite support.
- `src/routing.rs`: the complete mono/stereo host × response interpretation matrix.
- `src/control.rs`, `src/request.rs`: bounded sample-offset live controls, wet post-convolution EQ /
  Modulation / Width target smoothing, Feedback's exact-zero branch and bounded coefficient ramp,
  conservative post-stage tail allowances, plus monotonic asynchronous response-request eligibility.
- `src/engine.rs`, `src/feedback.rs`, `src/delay.rs`, `src/post.rs`, `src/activity.rs`: the explicit
  one-sample response-relative Feedback return, its three-phase response replacement, pre-delay,
  response crossfade, the single post-crossfade Wet EQ/modulated-delay/Width chain, retirement
  handoff, Mix-zero parking, telemetry and finite/infinite tail bounds.
- `tests/proof.rs`: public-seam matrix, host-block, sample-rate, extreme-value and idle proofs.
- `tests/revision6_compat.rs`: bit-exact Revision 5 DSP lock captured from unchanged `fda4f94`
  before Revision 6 behavior changes; re-derived at `fda4f94` at wet gain one when the wet gain was
  deleted (2026-09-28), which multiplying by one makes exact.
- `examples/fx_convolution_measure_engine.rs`: non-portable callback deadline probe.
- `examples/fx_convolution_render_demo.rs`: project-authored generated-response audition render;
  output belongs under ignored `target/` and is never committed.

# Local Contracts

- **The response interpretation is explicit state.** Mono uses one coefficient stream;
  mono-to-stereo and diagonal stereo use two. Width never chooses the meaning of a two-channel WAV,
  four-channel matrices are absent, and path gains are never independently normalised. The plugin
  normalises a whole response's level once, with one gain for every channel, before it reaches this
  crate (`plugins/mxm-fx-convolution/AGENTS.md`).
- **Zero delay is structural.** The first 64 coefficients are a direct head. Coefficients through
  sample 4,095 use 64-sample/128-point early partitions. The late tier starts at sample 4,096 and
  uses 2,048-sample/4,096-point partitions; its one-block lookahead distributes frequency products
  over thirty-two early ticks before output is due. Both transform widths follow linear-convolution
  padding. An impulse's sample zero is output at sample zero.
- **The ordered wet path is fixed:** prepared FIR → per-engine Pre-delay → old/new response
  crossfade → one Wet EQ → one bounded modulated delay → mid/side Width → Mix. Wet EQ and
  Width are project-derived linear operations. Engaged Modulation is linear time-varying and never
  changes prepared coefficients. Coefficient gain, deterministic IR noise/clipping shape and
  direct-path timing remain audible. There is no limiter, saturator, normaliser, content-aware gain
  law, and since 2026-09-28 no wet gain: the plugin normalises every response, so Mix is the one
  level (the owner). Every stage before the Mix cleans its own output and a blend of finite values
  cannot overflow, so the Mix's non-finite guard is defence rather than a path. Revision 7 adds one bounded odd soft saturator in the raw-response Feedback return only; no
  saturator exists in the dry path, wet output or post stages. Final-output clip telemetry observes
  without changing samples.
- **Response replacement is bounded to two complete engines.** Old/new complete wet outputs use
  convex weights. `begin_engine_transition` accepts an already constructed engine so a plugin can
  transfer off-audio preparation without rebuilding spectra/history. A third request waits until
  retired storage is collected off audio. This
  deliberately attenuates and reshapes the old tail; it does not claim exact old-tail preservation.
  Off publishes a new empty primary without waiting for a callback fade.
- **Processing allocates and destroys nothing.** Response spectra, histories and pre-delay storage
  are complete before publication through fallible construction. `take_retired` is called off audio.
  Sample processing performs no lock, I/O, planning, logging or allocation. Reset, park and wake are
  constant-time: generation/valid-sample bookkeeping invalidates response- and sample-rate-sized
  history without clearing it, and stale cells remain unreadable until overwritten.
- **Mix zero is Off.** The wet reaches zero, histories clear once and the engine parks; waking resets
  before accepting audio. Dry routing remains live and channel-correct. Loading a response alone
  creates no tail.
- **Finite support becomes exact idle.** Each path stops after the last nonzero input's declared FIR
  support, invalidates FFT roundoff/history and produces exact zero. Activity may be conservative but
  is never shorter than response + pre-delay + an active transition. A pre-delay edit extends the
  bound while any valid nonzero ring history could be exposed; reaching idle before the edit does not
  grant permission to forget that history.
- **Numeric recovery is not character.** Non-finite source coefficients are rejected. Non-finite
  input/intermediate/final values recover to zero at named seams; subnormal coefficients and stored
  state flush to signed zero without noise. Reset clears direct, block, spectrum, overlap, delay,
  transition-activity and telemetry state deterministically.
- **Feedback wraps the complementary raw response only.** Its nonnegative finite target is the
  response-relative coordinate `q`. Preparation separately measures the complete damped loop's
  positive-real Nyquist crossing `B_edge` and H-infinity peak `B_peak`, including `R·H` and the
  explicit one-sample delay. Runtime return is `A*tanh((q/B_edge)*D*R*y/A)` in widened arithmetic:
  `q = 1` is the measured linear self-oscillation edge, while only
  `q*B_peak/B_edge < 1` certifies contraction and a geometric finite horizon. Magnitude never stands
  in for phase again. Pre-delay, response output crossfade, Wet EQ, Modulation, Width and Mix
  remain outside. Zero target and zero current value produce structural `None`, skipping return
  routing, multiplication, saturation and delay reads exactly. A response replacement ramps from
  the old edge divisor to the larger old/new peak, holds that conservative bridge through the convex
  crossfade, then ramps to the new edge divisor. Reset/park clear recursion. An uncertified nonzero
  recursive state—not merely a nonzero target—maps to `Activity::KeepAlive`; Off/reset/wake clear
  either class.
- **Wet post-convolution live controls are a separate layer.** The legacy `LiveControls` snapshot
  and constructor stay unchanged. Low cut 0, Tone 0 dB, Modulation 0 and Width 1 retain exact neutral
  targets. The private High cut `-1` sentinel alone preserves the bit-identical Revision 6 DSP bypass
  locked in `tests/revision6_compat.rs`; it is not a plugin value. The plugin's public High cut is a
  real 0–20 kHz frequency: zero fully closes the low-pass and the real 20 kHz maximum is displayed as
  Open. The owner's correction intentionally changes the plugin's old default render. Their fixed-capacity sample-offset events retain insertion order,
  invalid values are sanitized at the control seam, and reset/wake settles selected targets rather
  than carrying stale ramps. Wet EQ and Modulation advertise serial tail allowances; Width is
  instantaneous and adds none.
- There are deliberately no keyboard/performance sources, envelopes, pulsers, random sources,
  modulation routes or sequencer. Revision 6's only LFO belongs inside the bounded post-convolution
  modulated-delay stage: it never animates prepared coefficients. Feedback has no modulation source
  and Freeze remains absent.

## Measured and chosen facts

- `PARTITION_SAMPLES = 64`, late offset 4,096 and late partition width 2,048 are chosen, not read
  from a product. Direct-form nulls cross every early/late boundary and awkward host blocks.
- **Owner ruling, 2026-09-14:** ten full prepared seconds are supported through 96 kHz. Above that,
  only the measured 4,096-sample early tier is accepted; preparation rejects excess with no
  truncation. Windows x86_64 release measurements at 64-frame callbacks, warmed complete old-engine
  history, a fresh maximum-size replacement, the old late tier aligned to its heaviest boundary
  inside the real 20 ms crossfade, explicit seam flushing and FTZ register state not queried are
  recorded as Revision 7 neutral then fully engaged Wet EQ/Modulation/Width plus the deliberately
  uncertified `q = 1.25` Feedback probe, **steady / complete three-phase transition worst**: 8 kHz ×
  80,000 measured 312.8 / 345.9 then 437.4 / 364.7 µs against 8 ms; 48 kHz × 480,000 measured
  405.0 / 387.1 then 521.4 / 424.2 µs against 1.333 ms; 96 kHz × 960,000 measured 528.3 / 417.2
  then 603.1 / 434.4 µs against 666.7 µs; and 384 kHz × 4,096 measured 114.3 / 65.9 then 57.4 /
  76.8 µs against 166.7 µs. The Feedback transition includes its 64-sample pre-ramp, the full
  response crossfade and its 64-sample post-ramp. These
  are the highest observations from the final runs on one machine, not portable proof. The >96 kHz
  cap is measured at the highest supported rate, so lower rates have more callback time for the same
  fixed work.
- The Feedback return scale `A = 0.25` is the chosen in-loop saturation ceiling in normalized sample
  units. `A` cancels at small signal and therefore does not move the Nyquist edge; it is not a safety
  ceiling on output and was not read from another product. The displayed range ends at `q = 1.25`.
  Release-bundle measurement on three deliberately different owner impulses brackets the sustained
  edge at 0.98–1.02 after response-specific preparation; the previous magnitude/RMS variants ranged
  from 0.56 to 2.32 and could falsely certify an unstable setting.
- The pre-listening Wet EQ and Modulation values are a 20 Hz minimum enabled low cut, a 200 ms
  conservative Wet EQ settling bound, ±6 dB Tone ceiling, and a 0.23 Hz deterministic stereo
  modulation whose full-amount read moves from 1 to 7 ms with a quarter-cycle channel offset. They
  are provisional engineering values, not values selected by listening, and await owner approval;
  none was read from another product.
- The generated room render responses are 0.45 s solely to keep the pre-listening demonstration
  bounded. The Feedback audition uses a separate project-authored sparse 0.18 s response at
  `q = 1.12`; its final-second PCM tail remains nonzero (peak 13,976, RMS 6,956 on the generated
  16-bit file), demonstrating reachable self-oscillation for one qualifying response without making
  that claim for every WAV. These are not the starter response and establish no product range.

# Work Guidance

- Preserve complete linear convolution before optimizing. Every scheduling change is first nulled
  against direct form through the complete tail and across awkward response/host boundaries.
- Preparation may allocate; `process_block`, `process_excitation` and per-sample helpers may not.
- Do not add WAV parsing, filesystem paths, async executors, plugin parameters or persisted payloads
  here; those belong to `plugins/mxm-fx-convolution`.
- Do not extract the FFT, delay or routing into shared DSP until another shipped implementation
  demonstrates the API.
- Treat the local deadline figures as warning evidence. Never infer a portable response-duration
  limit from them or hide a miss behind average CPU.

# Verification

```bash
cargo test -p mxm-fx-convolution-dsp
cargo test -p mxm-fx-convolution-dsp --test revision6_compat
cargo clippy -p mxm-fx-convolution-dsp --all-targets -- -D warnings
cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 8000 80000 feedback
cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 48000 480000 feedback
cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 96000 960000 feedback
cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 384000 4096 feedback
# Repeat each corner with `neutral` as the final argument for the paired bypass envelope.
cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_render_demo
```

The tests prove direct-oracle agreement, all six channel rows, neutral legacy bit identity including
Feedback zero, Feedback event/ramp/sanitization/reset/park behavior, separate checked Nyquist-edge
and H-infinity bounds, return-matrix routing, explicit one-sample raw-response recursion,
response-relative gain,
return-only saturation, three-phase replacement, certified decay below one and sustained activity
above it, exact Feedback-zero drain, reset and Mix-Off/wake clearing, 44.1/48 kHz exact idle, and
host-block invariance with Feedback, replacement and the wet post path engaged. They also prove
sample-rate sample placement including both partition tiers, scalar fractional-delay agreement, Wet
EQ attenuation, output-pair Width, response replacement, constant-time invalidation on
Off/reset/wake, pre-delay and post-stage activity, finite hostile output, fallible preparation and
structural subnormal flushing. The timing example is scoped
Windows evidence only. Listening, plugin callback
allocation instrumentation, broader WAV compatibility, host tail/latency,
Linux/macOS and real-DAW behavior are later gates. *Since the split (2026-10-06):* the tests run on
Linux in WSL before a push and on macOS by CI on `v*` release tags or by hand.

# Child DOX Index

No child AGENTS.md files.
