# AGENTS.md — plugins/mxm-fx-convolution/

Parent: [`../AGENTS.md`](../AGENTS.md) · The full rules, their history, measurements and past
verification runs: [`NOTES.md`](NOTES.md) (status: [§ Status](NOTES.md#status)).

# Purpose

The `mxm-fx-convolution` CLAP effect around `crates/mxm-fx-convolution-dsp`: permanent identity and
legacy, wet-post and Feedback live parameters, explicit host layouts, embedded response/model state,
bounded realtime processing, activity/tails, atomic telemetry, Init, factory content, control-map
integration, a dynamically paged four-card editor and focused release/debug real-host proof.

Product plan: `plans/plan-mxm-fx-convolution.md` in the private archive. UI brief:
[`../../docs/briefs/mxm-fx-convolution.md`](../../docs/briefs/mxm-fx-convolution.md). The effect is
an original implementation of public convolution practice, not a hardware or product copy. No
existing convolver implementation or third-party response was opened or copied.

# Ownership

- `src/lib.rs` — CLAP export, layouts, activation, bounded processing, activity, state preflight.
- `src/params.rs` — the host ids and the generated-response default patch.
- `src/response.rs` — canonical response state, the starter, WAV decoding, reversible modelling,
  resampling, deadline validation, staging of complete prepared engines, display cache, capture.
- `src/telemetry.rs` — lock-free telemetry; `src/deferred.rs` — the deferred-preset owner;
  `src/impulses.rs` — the impulses folder; `src/editor.rs`, `src/editor/` — the editor;
  `src/preset.rs` — the preset seams and generated Init (no factory set is compiled in).
- `control-map.json` — the existing `fx.reverb` role assigned to Mix; no invented page or role.
- Each file in detail: [NOTES.md § Files](NOTES.md#files).

# Local Contracts

## Identity and live controls

- Product name: `mxm-fx-convolution`; permanent CLAP id: `dk.mxm.mxm-fx-convolution`.
  `plugin_name!` is the sole crate-local name literal and `bundler.toml` is the one external
  duplicate, pinned by a test. Package version **0.1.1** is the High cut state-migration boundary:
  empty/0.1.0 state uses the rejected bottom-Open layout; 0.1.1 and later state may persist a real
  closed zero and must never remigrate it.
- **Every response is normalised, and Mix is the one level**: `response::loudness_scale` in
  `prepare()`, straight after decoding, on every path; no stored state or fingerprint changes.
  `every_response_is_prepared_at_one_loudness` holds it. **Wet gain is deleted, id and all**
  ([NOTES.md § Normalisation](NOTES.md#normalisation-mix-and-the-live-ids)).
- Legacy ids `mix` (linear) and `predelay` (moves a DSP tap; no second smoother). New ids `lowcut`,
  `highcut`, `tone`, `width`, `modulation`, `feedback`, `predelaysync`. Low cut, Tone, Width,
  Modulation and Feedback default to exact DSP-neutral values. High cut is a real 0–20 kHz
  low-pass, closed at the bottom, its real maximum shown as Open. Size, Decay and Damping are
  non-automatable model state with no host ids.
- **`predelaysync`** (Revision 8), the collection's one tempo sync on `params::PRE_DELAY_SYNC`,
  defaults off; `filter_state`, `migrate_legacy_identity` and the legacy resolver all supply it off
  ([NOTES.md § Pre-delay sync](NOTES.md#pre-delay-sync)).
- Mix zero is Off after the DSP-owned 12 ms fade: histories clear and park, mono dry duplicates to
  stereo, and stereo dry remains channel-for-channel. An inserted default is engaged.
- Feedback zero is an exact structural bypass. Displayed 1.00 is the linear self-oscillation edge;
  the ceiling 1.25 is past it, labelled self-oscillation-capable, not Unity. The one soft saturator
  is in the return only; all else stays linear. No Freeze, no printed threshold
  ([NOTES.md § Feedback](NOTES.md#feedback)).
- Non-neutral wet-post/Feedback ranges and the Decay/Damping laws are pre-listening values awaiting
  owner approval; neutral states and meanings are fixed, and a later tuning never changes an id
  ([NOTES.md § Pre-listening ranges](NOTES.md#pre-listening-ranges-and-the-high-cut-correction)).

## Layout, processing and activity

- Two explicit layouts: mono input to stereo output first, then stereo input to stereo output. There
  is no note port, MIDI input, sidechain or surround layout, and therefore no developer CC channel.
- nice-plug sample-accurate automation splits host callbacks at parameter events. Internal spans are
  capped at 64 samples; Mix, Pre-delay, the five wet-post controls and Feedback start DSP-owned
  bounded ramps at the host boundary.
- `process()` allocates, locks, logs and performs I/O never. WAV decode, modelling, resampling and
  complete fallible engine construction run on the background/state thread; activation consumes the
  exact staged engine without preparing it a second time. Fixed scratch arrays adapt host buffers to
  the DSP. Reset/park/wake invalidates large history in constant time rather than clearing response-
  or delay-sized storage.
- Finite feed-forward or certified recursive work reports `Tail(n)` after input stops. Uncertified
  nonzero Feedback history reports `KeepAlive`, including while input is present; exact idle and
  parked Off report `Normal`. Reset clears every history and telemetry tail. Vendored nice-plug
  stores each new process status before calling `clap_host_tail.changed` on finite/infinite edges.

## Presets and control map

- Init is generated from CLAP defaults. In the app bar it is a deferred model overlay whose identity
  clears only after the next callback acknowledges publication; the shared synchronous Init entry
  point stays unchanged for every non-opt-in consumer ([NOTES.md § Init](NOTES.md#init)).
- **The factory set is the impulses folder**; nothing is compiled in. Default Mix is 30 %. The
  legacy resolver supplies Feedback zero and Pre-delay sync off for older preset files.
- Selection goes through the opt-in `DeferredPresetTransaction`: rejected, cancelled and superseded
  work emits nothing; Ready writes only bases whose unmodulated bits and base revision are
  unchanged; identity follows callback acknowledgement, and edits before it stay Modified; WAV loads
  and model edits cancel unpublished preset work; a restore advances a restore-only generation;
  Pending suppresses Save/Save As/Rename; `state = null` stays preserve-only
  ([NOTES.md § The deferred preset transaction](NOTES.md#the-deferred-preset-transaction)).
- Mix alone reuses `fx.reverb`. Pre-delay, Low cut, High cut, Tone, Width, Modulation and Feedback
  remain automatable but unmapped; no permanent Convolution control-map page is added.

## Durable response state

- Host state field `response`, schema 1, embeds little-endian finite `f32` coefficients as base64,
  their source sample rate, frame count, name and explicit Mono/Mono-to-stereo/Diagonal-stereo
  interpretation. A path and prepared FFT spectra are never state.
- A malformed field makes the whole incoming state a no-op. Exact clean older identities migrate and
  stay Clean; a mismatched fingerprint stays Modified, never laundered
  ([NOTES.md § State migration](NOTES.md#state-migration)).
- Canonical source is 10.0 s (owner ruling, 2026-09-14); longer WAVs keep the first 10.0 s with a
  fade. Source and wet rates are 8–384 kHz; any other finite positive host rate activates with an
  inert wet and exact dry ([NOTES.md § The ten-second source](NOTES.md#the-ten-second-source)).
- 10.0 prepared seconds through 96 kHz; outside the budget an off-audio normalizer fits Size, then
  Extent, then Onset, except above 96 kHz, where it is a visible failure-atomic refusal. Canonical
  coefficients are never truncated ([NOTES.md § The preparation budget](NOTES.md#the-preparation-budget)).
- Model operations derive from the immutable source and never accumulate. A rejected restore rolls
  back byte-identically without `reset()`; a changed response crossfades 20 ms from the old engine
  ([NOTES.md § Activation and restore](NOTES.md#activation-and-restore)). Requests are latest-wins;
  a failure keeps the old response active and shows the error ([§ Acquisition](NOTES.md#acquisition)).
- The 7 ms starter response is project-authored code; its constants and the Wet EQ/Modulation
  constants await owner approval ([NOTES.md § The starter response](NOTES.md#the-starter-response)).

## The collection's impulse responses

- Installed per user at `mxm/impulses/` under the platform's local data directory, never inside the
  plugin; nothing creates it, and projects never depend on it. Browse opens there (`impulses::root`).
- Its contents are the factory set: `impulses::scan` lists it each time the editor opens, and no
  file is named in code. A preset carries only `{"impulse": <relative path>}`, read by
  `impulses::load` on the deferred worker. Banks leave found presets out; a path leaving the folder
  is refused; a missing file fails the load and keeps the current response.
- A published preset wakes the audio side (`EditorTask::Wake`). Full rules:
  [NOTES.md § The collection's impulse responses](NOTES.md#the-collections-impulse-responses).

## Editor

- Four stable cards in signal order: Response, Time, Shape, Output; Response and Time the sole
  preferred group; every card indivisible and as wide as its floor.
- Every card body is a `mxm_ui::tree` (`editor/sections.rs`); floors are computed by
  `tree::card_floor`, never typed; model values join the keyboard cursor; still text (E1); no help
  text on a card. `every_card_passes_the_tree_checks_in_every_state` holds it. `REFERENCE` and
  `MINIMUM` are held by tests.
- `editor/binding.rs` re-exports `mxm_preset::binding`. Size, Decay and Damping sliders submit only
  on release and emit no host gestures. Nothing is drawn to measure a card.
- States are named in text as well as colour. The plot is energy, not a waveform; paint never
  decodes or clones embedded payload. Browse's dialog runs through `mxm_ui::offthread`. DSP reads no
  editor state. Full rules: [NOTES.md § Editor](NOTES.md#editor).

## Telemetry

- Atomics only. Peak max-combines and resets on read; clip and numeric faults latch until
  acknowledged; remaining tail is the latest callback value. Response rejection publishes its kind
  and host rate during activation, survives callback telemetry, and clears only after supported wet
  preparation succeeds.
- Telemetry observes only. It never clips, limits, saturates or otherwise changes the approved pure
  linear output; the one level change is preparation's normalisation, above. The status line speaks
  to the player ([NOTES.md § The response status line](NOTES.md#the-response-status-line)).

# Work Guidance

- File decoding, model edits and response replacement preserve this committed-state boundary:
  immutable source plus reversible metadata, complete fallible preparation off audio, latest-request
  publication through one state transaction, and no half-prepared response in `process()`.
- Do not add a limiter, a second normaliser, Feedback around post-processing, Freeze, third-party
  response, MIDI port, new preset axis, control-map role or editor in a core/content repair.
- Active-response replacement uses the DSP's bounded two-engine contract and retires allocations off
  audio; do not put a lock around the engine.

# Verification

```bash
cargo test -p mxm-fx-convolution-dsp -p mxm-fx-convolution
MXM_PICTURES=after cargo test -p mxm-fx-convolution --lib tree_pictures -- --ignored   # target/layout-tree/mxm-fx-convolution/after/
cargo clippy -p mxm-fx-convolution-dsp -p mxm-fx-convolution --all-targets -- -D warnings
cargo fmt --all -- --check
cargo xtask fetch                        # test-bundles.txt; mxm-mono-01 is the Player chain's source

# target/bundled is mutable: validate and test debug before replacing it.
cargo xtask bundle mxm-fx-convolution
clap-validator validate "target/bundled/mxm-fx-convolution.clap"
cargo test -p mxm-fx-convolution-host-tests --test behaviour -- --nocapture
cargo test -p mxm-fx-convolution-host-tests --test behaviour editor_opens_closes_and_reopens_through_the_player -- --ignored --nocapture --test-threads=1
cargo test -p mxm-fx-convolution-host-tests --test robustness -- --nocapture

# Build, validate and test release last; leave this validated release bundle staged.
cargo xtask bundle mxm-fx-convolution --release
clap-validator validate "target/bundled/mxm-fx-convolution.clap"
cargo test -p mxm-fx-convolution-host-tests --test behaviour -- --nocapture
cargo test -p mxm-fx-convolution-host-tests --test behaviour editor_opens_closes_and_reopens_through_the_player -- --ignored --nocapture --test-threads=1
cargo test -p mxm-fx-convolution-host-tests --test robustness -- --nocapture
```

- Host proof: `host-tests/tests/behaviour.rs` and `robustness.rs`; the ignored owner-catalogue rig
  fails on a missed or unbracketed Feedback edge ([NOTES.md § Host proof](NOTES.md#host-proof)). The
  control map uses the current player schema (`instruments` array); the real-host test guards it.
- Each profile passes the complete validator suite before the mutable bundle slot is replaced. No
  skipped validator capability or ignored native-window case counts as a pass.
- Coverage: [NOTES.md § What the focused tests cover](NOTES.md#what-the-focused-tests-cover). Past
  runs and artifact hashes: [NOTES.md § Past verification runs](NOTES.md#past-verification-runs).
- Concurrent active-host restore, listening, native §15 visual QA, real DAW and Linux/macOS remain
  later gates. *Since the split:* Linux and macOS are checked later, together, and by CI when started by hand.

# Child DOX Index

None.
