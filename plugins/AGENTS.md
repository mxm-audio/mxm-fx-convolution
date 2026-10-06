# AGENTS.md — plugins/

Parent: [`../AGENTS.md`](../AGENTS.md)

# Purpose

This repository's CLAP plugin: a thin nice-plug shell over the framework-free DSP crate —
parameters, host contract, event handling and an editor. It keeps the conventions every MXM
plugin keeps. The full text, with the reasons, measurements and history, is mxm-kit's
[`docs/plugin-conventions.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md);
this file is their contract, one line per rule, each linking to its section. What is true of this
machine only lives in its own `AGENTS.md` below.

# Ownership

- Owns each `plugins/<plugin>/`: `Cargo.toml`, `README.md`, `control-map.json`, `presets/`, `src/`
  and `host-tests/`. Not the DSP (`crates/<plugin>-dsp`) and not styling (mxm-kit's `mxm-ui`).
- `host-tests/` is the plugin heard through MXM Player, a separate package so `cargo test -p
  <plugin>` (the fast tier) never builds the player. The slow tier: `cargo xtask bundle <plugin>
  --release`, then `cargo test -p <plugin>-host-tests`. [More](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#ownership)
- The shared checks are `mxm-plugin-test` (a dev-dependency): `keyboard_checks`, `paging_checks`,
  `opening_size`, `time_text_checks`, `tree_checks`, `routing_checks`, `hover_text`. Keep them whole.
- `crate-type = ["cdylib", "lib"]`. Never enable nice-plug's `standalone` feature: it puts cpal and
  JACK into the `.clap`.

# Local Contracts

- **Init** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#every-instrument-has-an-init-patch-and-init-returns-to-it)): every amount
  starts at zero, every configuration somewhere useful; an effect starts engaged (mxm-fx-curve
  excepted). The init patch is the CLAP defaults; Init writes persisted parameters only, each as a
  full begin/set/end gesture, and has no preset file. A deviation is recorded in the plugin's own
  `AGENTS.md`.
- **Presets** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#a-preset-is-parameter-values-and-every-instrument-stores-them-the-same-way)):
  through `mxm-preset` (`impl mxm_preset::Instrument`). Every instrument ships fifty factory sounds,
  an effect what its brief justifies once it has a control beyond level. Factory presets are
  compiled in; `v` is authoritative and `text` never read; applying is bracketed parameter writes
  with the target set resolved first. User presets live under the injected config root, collection
  content under `data_local_dir`. Favourites are per person and sort first; dirty is a comparison.
- **Modulation** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#every-instruments-modulation-is-the-shared-routing)): the
  shared routing (`mxm-modulation`, `mxm-modulation-params`), with a source chosen on the card it
  moves. A target is named for what it moves, one per parameter unless the law differs; the
  standard reach for added performance paths; a standard *Amplitude* target; prove it with
  `mxm_modulation::conformance` and `routing_checks`.
- **Smoothing** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#smooth-signals-not-coefficients)): smooth what is added to or
  multiplies the audio (5–20 ms), never a coefficient: envelope and glide times stay unsmoothed.
- **Permanent identifiers** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#permanent-identifiers--never-change-these)): the
  `CLAP_ID` is `dk.mxm.<plugin>`, assembled from `plugin_name!`; a parameter `#[id]` is never
  changed or reused. Both are public interface.
- **nice-plug, not nih-plug** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#this-is-nice-plug-not-nih-plug)): `activate`, an
  associated `type Editor`, `nice_export_clap!`; declare `CLAP_FEATURES` and `AUDIO_IO_LAYOUTS`.
- **Parameters** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#parameters)): gain stores linear gain; skewed ranges for frequency,
  gain and wide pitch spans; a unit switch chosen from the rounded reading; sentence-case names; no
  help text on the panel (tooltips written for the player, held by `speaks_to_the_player`); a
  syncable control shows its reading; each button of a row says what it does; a range is buttons.
- **Tempo sync** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#tempo-sync-is-one-button-changing-one-rate-or-time-control)): one
  *<Control> sync* button changing one Rate or Time control, on `mxm-tempo`'s ladder; an old preset
  or project loads it off; a cached sync does not outlive an activation or a reset.
- **Editable models** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#editable-models-what-is-a-parameter-and-what-is-state)): a
  user-authored model's controls are versioned state, not parameters; bound them in the DSP.
- **Control map** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#a-plugin-ships-its-own-control-map)): `control-map.json` beside
  the plugin names only roles `MXM_CONTROL_MAP.md` declares, by permanent `#[id]`; leave out a role
  the plugin does not have. `cargo xtask bundle` checks it against the standard.
- **Effects** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#an-instrument-ships-the-effects-its-original-had-and-no-others),
  [§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#every-effect-built-into-an-instrument-is-also-promoted-to-a-standalone-effect)): an
  instrument ships the effects its original had and no others (mxm-mono-00 exempt by ruling); a
  built-in effect is also promoted to a standalone that depends on the instrument's DSP crate in
  place and never changes the instrument's render.
- **Developer channel** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#a-developer-channel-in-every-editor)): with `MXM_DEV_CC` set,
  CC 119 selects a category or the Parameters view (127), 118 the expander, 117 the preset browser
  and 116 the theme. Off otherwise; an effect has no note port and carries none of it.
- **Per-note expression** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#per-note-pitch-expression-is-accepted-nothing-else-per-note-is)):
  `PolyTuning` only, routed by note, unsmoothed; a non-finite tuning is dropped.
- **Realtime** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#realtime-rules-for-process)): no allocation, locks, logging or
  I/O in `process()`; preallocate in `activate()`, clear in `reset()`; report `Tail` while releasing;
  doing nothing costs nothing. `assert_process_allocs` fires in debug builds only.
- **Editor** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#editor-contract)): a panel, never a window, and transient — DSP never
  reads editor state. Parameter edits are bracketed gestures; telemetry is atomics. Widgets, theme,
  zoom and paging come from `mxm-ui`; every card passes the tree checks; the window is at least one
  card and the app bar wide; it opens at the quarter-4K budget, hugged. Test what the editor paints.
- **Keyboard cursor** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#the-keyboard-cursor-runs-in-every-editor-and-each-one-owes-it-three-things)):
  `panel` takes a `navigation::State`, calls `navigation::paged` before the cards, and wraps each
  control in `navigation::at`; prove it with `keyboard_checks::the_cursor_reaches_and_operates`.
- **Modulation display** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#a-knob-shows-the-parameter-modulation-is-drawn-over-it)): the
  knob draws the unmodulated value and an arc to the modulated one; a drag draws its own delta.
- **Licensing** ([§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#licensing)): GPL-3.0-or-later, the repository's `LICENSE`; decide
  before copying anyone else's code.

# Work Guidance

- A fact about one machine belongs in `plugins/<plugin>/AGENTS.md`; a rule for every plugin belongs
  in the kit's conventions. Adding a plugin: [§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#adding-a-new-plugin).

# Verification

```bash
cargo test -p mxm-fx-convolution
cargo clippy -p mxm-fx-convolution --all-targets
cargo xtask bundle mxm-fx-convolution --release          # -> target/bundled/mxm-fx-convolution.clap
clap-validator validate "target/bundled/mxm-fx-convolution.clap"
```

- A change that only moves or sizes things on screen is checked with the layout tests, not the
  full suites. A debug build run matters most: allocations only abort there. [§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#verification)
- `clap-validator` is installed separately (`cargo install --git
  https://github.com/free-audio/clap-validator.git --locked`). A crash reading `0xc0000409` is a
  panic. Rebundle before looking at the plugin in a host. [§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#reading-a-validator-crash)
- Installing a bundle in a DAW: [§](https://github.com/mxm-audio/mxm-kit/blob/main/docs/plugin-conventions.md#installing-a-bundle-in-a-daw).

# Child DOX Index

| Doc | Scope |
|---|---|
| [`mxm-fx-convolution/AGENTS.md`](mxm-fx-convolution/AGENTS.md) | Identity, embedded response state, the collection's impulses folder, presets, activity and editor |
