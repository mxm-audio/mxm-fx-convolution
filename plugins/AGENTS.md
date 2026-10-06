# AGENTS.md — plugins/

Parent: [`../AGENTS.md`](../AGENTS.md)

# Purpose

The CLAP plugins the collection ships. Each is a thin nice-plug shell over a framework-free DSP
crate: parameter definitions, host contract, event handling, and an editor.

Twenty plugins — eleven instruments and nine effects:

| | |
|---|---|
| [`mxm-mono-01/`](https://github.com/mxm-audio/mxm-mono-01/blob/main/plugins/mxm-mono-01/AGENTS.md) | Monophonic subtractive synthesizer inspired by the SH-101 |
| [`mxm-mono-03/`](https://github.com/mxm-audio/mxm-mono-03/blob/main/plugins/mxm-mono-03/AGENTS.md) | Monophonic acid bass voice inspired by the TB-303 |
| [`mxm-poly-06/`](https://github.com/mxm-audio/mxm-poly-06/blob/main/plugins/mxm-poly-06/AGENTS.md) | Six-voice polysynth with the source instrument’s built-in chorus |
| [`mxm-mono-00/`](https://github.com/mxm-audio/mxm-mono-00/blob/main/plugins/mxm-mono-00/AGENTS.md) | Monophonic semi-modular with target-local routing and three effects under its explicit exemption |
| [`mxm-mono-02/`](https://github.com/mxm-audio/mxm-mono-02/blob/main/plugins/mxm-mono-02/AGENTS.md) | Monophonic two-oscillator synthesizer with a sub and one envelope shared three ways |
| [`mxm-mono-08/`](https://github.com/mxm-audio/mxm-mono-08/blob/main/plugins/mxm-mono-08/AGENTS.md) | Patchable monophonic voice with dual low-pass gates, a five-step sequencer and mono spring |
| [`mxm-mono-pr1/`](https://github.com/mxm-audio/mxm-mono-pr1/blob/main/plugins/mxm-mono-pr1/AGENTS.md) | Monophonic two-oscillator instrument with additive waves and the collection's any-to-any modulation routing |
| [`mxm-para-07/`](https://github.com/mxm-audio/mxm-para-07/blob/main/plugins/mxm-para-07/AGENTS.md) | Two-pitch paraphonic synthesizer with divider oscillators and one shared articulation path |
| [`mxm-folded-spring/`](https://github.com/mxm-audio/mxm-folded-spring/blob/main/plugins/mxm-folded-spring/AGENTS.md) | Spring reverb promoted from mxm-mono-00, with selectable tank and feedback |
| [`mxm-chorus-06/`](https://github.com/mxm-audio/mxm-chorus-06/blob/main/plugins/mxm-chorus-06/AGENTS.md) | Chorus promoted from mxm-poly-06 and proved equal to its built-in circuit positions |
| [`mxm-bucket-delay/`](https://github.com/mxm-audio/mxm-bucket-delay/blob/main/plugins/mxm-bucket-delay/AGENTS.md) | Bucket-brigade delay based on the MN30xx device family and six-tap constellation |
| [`mxm-grain-fx/`](https://github.com/mxm-audio/mxm-grain-fx/blob/main/plugins/mxm-grain-fx/AGENTS.md) | Granular processor on a live capture buffer with a recycled loop |
| [`mxm-fx-convolution/`](mxm-fx-convolution/AGENTS.md) | Original convolution effect over embedded user response state, with an owner-ruled ten-second source |
| [`mxm-fx-curve/`](https://github.com/mxm-audio/mxm-fx-curve/blob/main/plugins/mxm-fx-curve/AGENTS.md) | Original serial point-curve processor with memoryless and linked-dynamics stages |
| [`mxm-fx-delay/`](https://github.com/mxm-audio/mxm-fx-delay/blob/main/plugins/mxm-fx-delay/AGENTS.md) | General-purpose Clean, Vintage digital and Tape delay with three routing and time-change laws |
| [`mxm-shimmer/`](https://github.com/mxm-audio/mxm-shimmer/blob/main/plugins/mxm-shimmer/AGENTS.md) | Pitch-shifted feedback reverb with outer-tap placement, reverse sweeps and Freeze |
| [`mxm-classic-verb/`](https://github.com/mxm-audio/mxm-classic-verb/blob/main/plugins/mxm-classic-verb/AGENTS.md) | Everyday algorithmic reverb whose spaces are fitted from impulse responses |
| [`mxm-creative-sampler/`](https://github.com/mxm-audio/mxm-creative-sampler/blob/main/plugins/mxm-creative-sampler/AGENTS.md) | Original small-sample instrument with Repitch, Stretch and Grain readers and embedded asset state |
| [`mxm-drum-machine/`](https://github.com/mxm-audio/mxm-drum-machine/blob/main/plugins/mxm-drum-machine/AGENTS.md) | Original sixteen-slot drum instrument over an append-only pool of machine-specific circuits; all 94 admitted models owner listening-approved, Kit or chromatic MIDI per slot, stereo main plus sixteen mono outputs, nine source-family audition presets on one canonical role map |
| [`mxm-model-drums/`](https://github.com/mxm-audio/mxm-model-drums/blob/main/plugins/mxm-model-drums/AGENTS.md) | Sixteen slots of realistic synthesized drums that reach far past the real ones; twenty general controls and four routes a slot (650 parameters); the Ringing kick its first model; built and bundled, clap-validator clean |


**This doc holds shared conventions.** Permanent identifiers, machine-specific behavior and active
product gates live in each plugin’s own `AGENTS.md` or active plan.

# Ownership

Owns each `plugins/<plugin>/` directory: `Cargo.toml`, its own `LICENSE`, `README.md`,
`control-map.json`, `presets/`, `src/`, and `host-tests/`. Instruments normally split `lib.rs`,
`params.rs`, `preset.rs`, `telemetry.rs` and `editor.rs`; a built editor may add an `editor/` subtree.

**`host-tests/` is the plugin heard through MXM Player** — its `behaviour`, `golden_audio`,
`robustness` or `effect_chain` tests and their fixtures, loading the release bundle through the real
host with [`mxm_player_harness`](https://github.com/mxm-audio/mxm-player/blob/main/apps/mxm-player-harness/AGENTS.md). It is a separate package
(`<plugin>-host-tests`) so the plugin's own `cargo test -p <plugin>` — the fast tier — never builds
the player; `cargo xtask bundle <plugin> --release` and then `cargo test -p <plugin>-host-tests` is
the slow tier, which the merge gate runs once, after the bundle.

The checks every plugin's tests share live in **`crates/mxm-plugin-test`**, a `[dev-dependencies]`
entry of each plugin and compiled into no bundle: `mxm_plugin_test::paging_checks` for the dynamic
pager, `mxm_plugin_test::opening_size` for the derived opening size,
`mxm_plugin_test::keyboard_checks` for the keyboard cursor, `mxm_plugin_test::time_text_checks` for a
time reading's host round trip, `mxm_plugin_test::tree_checks` for the layout tree's per-card checks
and the review pictures, and `mxm_plugin_test::routing_checks` for the modulation standard's plugin
half — a route parameter held to its DSP's `mxm_modulation::conformance::Declaration`. They are
shared because a check copied once per consumer is many chances to weaken it; keep them whole rather
than trimming one to what one consumer happens to call. The crate's own contract is
[`crates/mxm-plugin-test/AGENTS.md`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-plugin-test/AGENTS.md).

**`crate-type = ["cdylib", "lib"]`.** The `cdylib` is the shipped artifact; the `lib` exists so
[`apps/mxm-mono-01-standalone`](https://github.com/mxm-audio/mxm-mono-01/blob/main/apps/mxm-mono-01-standalone/AGENTS.md) can link the plugin and run its
editor outside a host. It adds no dependencies and changes nothing about the bundle. **Do not
enable nice-plug's `standalone` feature here** — it pulls in cpal, JACK and six more crates, and
they would end up in the `.clap`.

Owns **permanent identifiers** — see below. Does not own DSP (that is
[`crates/<plugin>-dsp`](https://github.com/mxm-audio/mxm-mono-01/blob/main/crates/mxm-mono-01-dsp/AGENTS.md)) or styling (that is
[`crates/ui`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/ui/AGENTS.md)).

# Local Contracts

## Every instrument has an init patch, and Init returns to it

**Every *amount* starts at zero; every *configuration* starts at a musically useful value.** So the
first control a person turns after Init produces the change that control is named after, and nothing
else.

The line between the two is **what a control does, not what unit it is in** — say that explicitly,
because "time" is the word that makes it ambiguous:

| | | Starts at |
|---|---|---|
| **Amount** | how much of an effect there is | **zero** |
| | modulation depths, resonance, **and glide time** — a time, and still an amount: it is how much portamento there is | |
| **Configuration** | how an effect behaves once you ask for it | **somewhere useful** |
| | LFO rate and shape, pulse width, sub shape, source and priority selectors, bend range | |
| | **and the envelope's own times**, because the envelope is always running — there is no "envelope amount" to be zero | |

**A deviation from any of this is recorded in the plugin's own AGENTS.md**, never here and never left
implicit, so that it reads as a decision rather than an oversight.

**An effect starts engaged** — the owner's ruling, 2026-09-03, for the effects collection. An
inserted effect demonstrates the effect it is named after, so its CLAP defaults are an audible,
useful setting rather than zero: `mxm-chorus-06` starts as the circuit at its first position. The
rule above is the instruments'; this is the effects', and each effect's AGENTS.md records which
setting it starts at. **`mxm-fx-curve` is the specific owner-directed exception:** fresh construction
and Init are one unlinked 1:1 identity stage, so inserting the open-ended authoring surface changes
nothing until the player draws a law or chooses a sound.

That is why "no LFO, but the LFO at a good vibrato rate" is not a contradiction: depth zero makes it
inaudible, and a sensible rate makes it *vibrato* the moment depth is raised rather than a drift or a
buzz.

The rest of the contract:

- **The filter starts effectively open**, and short of a range end that wastes the top of the
  control. The frequency is per instrument — a filter whose range stops at 8 kHz needs a different
  number for the same reason.
- **One plain source sounds**; the rest are silent.
- **Where an instrument has more than one oscillator, they start slightly detuned** rather than in
  unison. mxm-mono-01 has one — `oscillator.rs` holds a single `Phasor`, and saw, pulse and sub all read
  it — so it fills this line by having nothing to detune, and says so.
- **The init patch and the CLAP defaults are one set of values.** They are allowed to be equal and
  must not be two concepts: hosts show `default_value` as a control's detent and use it for "reset
  this parameter", so a divergence means the button and the host disagree about the same sound.
- **Init normally writes persisted parameters and nothing else.** The creative sampler's approved
  synchronous exception preserves its loaded A/B assets plus root and source extents while resetting
  performed processing. Convolution is the second durable-content consumer and the first asynchronous
  one: its explicit `DeferredPresetTransaction` Init merges neutral model state with the committed
  source, emits no default gesture while Pending, and clears identity only after process-boundary
  acknowledgment. **FX Curve is the authored-model exception:** its generated Init carries the
  versioned one-stage unlinked identity curve, prepares it before Mix gestures and publishes it after
  them, so the browser's Init cannot leave an edited stack behind. The opt-in paths change no existing
  consumer. Not sounding notes, not live controller state, not a host's sequencer or its per-step
  data. Every parameter write is a full
  `begin_set_parameter` / `set_parameter_normalized` / `end_set_parameter`; an Init that opens
  gestures and closes none latches every automation lane in the host.
- **A new instrument declares its init patch before its editor is built**, in its §14 brief.
- **Init has no preset file, and must not grow one.** It is the first row of the browser and it is
  *generated from the parameter defaults*. Writing it out as `presets/init.json` beside the others
  would put the same values in two places, and the first person to retune a default would ship a
  preset that no longer matched the button — which is the line above, one level up.
  `the_init_preset_is_the_parameter_defaults` compares them parameter by parameter.

## A preset is parameter values, and every instrument stores them the same way

**Every instrument ships the preset system** — the browser in the app bar's patch slots, a
compiled-in factory set, the generated Init, favourites, and Save / Save As / Rename / Delete for
user presets. An instrument without presets is unfinished, not minimal. `mxm-creative-sampler`'s
asset-aware user save/Init transaction is live and it ships **four recipes rather than a bank** —
`state = null` sounds that treat whatever is loaded four ways, so the listening gate can be held on
the owner's own recordings. The fifty-sound authored bank waits for that gate's verdict.

**An effect ships it when it has a meaningful control beyond level** (the owner's rule,
2026-09-03); a level-only effect does not, and normal CLAP state and host presets still work. The
instruments' floor of fifty sounds is theirs: an effect's factory set is what its brief justifies,
and `mxm-chorus-06` ships ten chorus sounds (the owner, 2026-09-28), its circuit's three positions
being buttons on its panel rather than presets.

**The system is [`crates/mxm-preset`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-preset/AGENTS.md)**, one crate for every
instrument and qualifying effect. A plugin implements `mxm_preset::Instrument` on its `Params` —
its `CLAP_ID`, parameters in declaration order, `#[persist]`ed identity slot and factory files. Its
`preset.rs` owns that implementation, factory sounds and product-specific tests; its editor calls
`mxm_preset::ui::preset_row` in the app bar and `overlays` immediately after the bar. Shared preset
behavior changes once in the crate and is tested against every consumer. The rules that shape it:

**Two numbers per parameter, and only `v` is read.** `v` is the normalised value and is
authoritative; `text` is the plugin's own formatting, kept so the file can be read and diffed, and
never loaded. That is what lets one `set_parameter_normalized` serve a float, an enum, a stepped
parameter and a boolean with no per-type conversion — and the conversions belong to nice-plug, which
this repository's rule about not guessing at its APIs already says to leave alone. There is
deliberately **no mismatch detection**: checking would mean formatting `v` back through one of those
APIs, which is the conversion the format exists to avoid.

**Applying is bracketed parameter writes, never a state blob.** A host sees parameter edits, so
automation lanes and undo work; a blob would need a second path into the DSP that nothing else uses.
The target set is resolved **before** anything is written — an unknown id is reported and skipped, a
parameter the preset does not mention keeps its current value — so a bad file cannot half-change a
patch, and each parameter is written exactly once rather than twice by way of Init. The default and
the creative sampler's durable-content seam remain synchronous. A consumer whose durable work is
asynchronous must opt into `mxm-preset::DeferredPresetTransaction`: Pending/Rejected emits nothing,
Ready emits eligible gestures then queues one prepared process-boundary publication, and loaded/Init
identity changes only after callback acknowledgment. A model overlay declares
`MergeWithCommittedSource`; `state = null` keeps its existing preserve-only meaning.

**Factory presets are compiled in** with `include_str!`. A plugin instance is never handed the path
its bundle was loaded from, and a directory beside a `.clap` is not part of it on every platform:
copy the bundle alone and the sounds would be gone. **The one exception is content the collection
installs per user**: mxm-fx-convolution's factory set is the impulse files in the collection's
impulses folder, scanned when its editor opens and handed over as `mxm_preset::Found` (the owner,
2026-09-28), because the folder is the content and a file added to it is a sound.

**A factory set is content, and fifty sounds is the floor** — every instrument ships fifty; the owner
raised the floor from twenty on 2026-09-04, twenty plus Init being too few to browse. The sounds are designed as overrides-on-defaults in the plugin's own test module and
generated to JSON by an `#[ignore]`d test, so the readable statement of each sound stays in one
place; a runs-by-default test compares the shipped files against the design, which is what catches
a regenerate that was forgotten.

**User presets live under the config directory, and the root is injected.** `dirs::config_dir()` is
called once and held as a field — which lets a test drive the `None` path deliberately and, far more
importantly, stops a test writing into the config directory of whoever ran it. Every preset test here
runs in a temporary directory it removes afterwards. Where there is no config directory at all,
saving **says why**; a Save button that does nothing is worse than one that explains itself.

**Collection content lives under the local data directory, not the config directory.** The impulse
responses `crates/mxm-room-ir` releases are installed at `mxm/impulses/` under
`dirs::data_local_dir()` (owner, 2026-09-16), because hundreds of megabytes must not roam with a
Windows profile. The folder belongs to the collection rather than one plugin id, and a VST3 build of
a plugin reads the same folder. As with presets, the root is resolved once and injected.

**A star has to do something.** Favourites shipped writing a file and lighting up on the bar, and
nothing read them: no list, order or filter. Asked, correctly, *"what is it used for?"* — and the
answer was nothing. Starred presets sort to the **top** of the browser, which is what the arrows then
step through first, and they carry a star in the list. A control that records a preference nothing
acts on is a control that does nothing dressed as one that does something; that is the bug, not the
missing polish.

The ordering is a **stable partition**, not a sort: within each group the library's own order is kept,
so starring one preset moves that one and leaves the rest where they were.

**A sound is saved with its category** (the owner's rule, 2026-09-04): one word from
`mxm_preset::Category`'s fixed, collection-wide list, chosen in the Save As row beside the name (a
factory preset found in a folder is filed under that folder instead, `crates/mxm-preset/AGENTS.md`);
Rename keeps the preset's own and Save keeps the loaded one's. The category is on the preset, not on
a bank, because a person looking for a bass wants every bass they own in one list whichever bank it
came from. A file without the field — every preset written before it — reads as *Uncategorised*, and
so does a word this build does not know; the schema stays at 1 because neither direction is wrong,
only poorer.

**A bank is a directory of presets beside your own, and one file on the wire** — `Origin::Bank`, a
`bank.json` naming it, and `<name>.mxmbank.json` in a `banks/` folder beside `presets/` for sending
and receiving. Import unpacks, refusing whole what is not this plugin's and asking before replacing a
bank of the same name; export packs any list of presets, so *save a category* is *export a bank*. A
folder and *Open folder* rather than a native dialog, because a dialog cannot be driven from the
player's CLI and the developer-channel rule reaches every editor state; `crates/mxm-preset/AGENTS.md`
has the rest.

**The name in the bar opens a three-pane browser** — banks, categories, presets with a search box,
the bank actions along its foot — in place of the dropdown, which a shared bank would have made
useless; the arrows step what it is filtering by, and CC 117 opens it for a script.
`crates/mxm-preset/AGENTS.md` has what it does and `crates/ui/AGENTS.md` how it is drawn.

**Favourites are per-person state, beside the presets rather than inside them.** A `favourite` field
in a preset would arrive on somebody else's machine already starred, and a factory preset could never
be starred at all. The index shares the directory and the `.json` extension, so `Library::list` skips
it by name — without that it appears in the browser as a broken preset, which reads as somebody's
file being corrupt.

**Dirty is a comparison, not a flag.** The identity and the baseline are set together and cleared
together, giving three states — *no preset*, loaded, loaded-and-modified — from one predicate:
`identity.is_some() && values != baseline`. Automation that ran while the editor was closed is caught
because the comparison happens when anybody looks; returning a parameter to its baseline clears the
marker because nothing latched it. **A save reads its baseline from the live parameters; a load
retains the intended targets after canonicalising them through each parameter.** It cannot read live
values immediately after emitting GUI gestures, because CLAP may apply those gestures on a later
process or flush call and that snapshot would describe the patch being replaced. Raw JSON numbers
are never baselines: stepped, enum and boolean parameters canonicalise what they are given.

**Init clears the identity rather than marking it modified.** There is nothing left that it would be
a modification of. So does deleting the loaded preset — and that leaves the sound exactly as it is,
because reverting or loading the next one would throw away what somebody is listening to as a side
effect of tidying up.

**Pin the rule, not the taste.** A test asserting every *amount* is zero survives a retune and fails
the moment somebody makes a depth non-zero because it sounded nice. Resonance and oscillator levels
are **not** modulation depths — resonance is a filter amount, the levels are a mix — so they are
pinned by the instrument's own defaults test, not by that one.

## Every instrument's modulation is the shared routing

**The owner's ruling, 2026-09-15: every instrument uses
[`crates/mxm-modulation`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-modulation/AGENTS.md) and
[`crates/mxm-modulation-params`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-modulation-params/AGENTS.md)** — *it is why it was
made*. A modulation path is a *(target, source)* route with a presence and an amount, not an
instrument-local matrix, bus or depth knob. The machine's own wiring is the init patch's present
routes, at the depths it always had (`mxm-mono-01/AGENTS.md`, *The machine's own modulation is
routing*). **A source is chosen on the card it moves, never at the source** (the owner,
2026-09-22): no card carries a destination switch, and a machine's own is routes into each
destination it reached. Where a conversion keeps a switch that no route can express, the
instrument's own AGENTS.md says why: `mxm-mono-01`'s `vcasource`.

### Declaring the target list — what `mxm-mono-00` had to discover twice

The owner's rulings of 2026-09-22, found on `mxm-mono-00` after its conversion had shipped. Each is
cheap to get right at intake and a re-release of ids to fix afterwards.

- **A target is the thing it moves, not the jack it came in on.** Where the hardware has several
  inputs into one parameter — a VCF's ADSR IN, LFO IN and KYBD CV all moving the cutoff — declare
  **one** target and give each source the reach its own jack gave it, through a per-*(target,
  source)* scale (`crates/mxm-mono-02-dsp/src/routing.rs`'s `FULL_SCALE`, `mxm-mono-00`'s
  `CUTOFF`); a source the machine never had takes the **standard reach**
  (`mxm_modulation::standard::reach` — below). Separate
  targets on one parameter read as *Modulator 1* and *Modulator 2*, and a player cannot tell which
  is which. **Keep a second target only where the law differs** — a gate, a sync, a dip-only
  tremolo, a narrowing pulse width — never for a second reach. Measure the merge against the
  unmerged renders; `mxm-mono-00`'s came out bit-identical.
- **A target is named for what it moves** — *Pitch*, *Cutoff*, *Amplitude*, *Pulse width*, *Rate*,
  *Tremolo* — so a row reads *Cutoff from Envelope 1*, `mxm-mono-01`'s model. **Never *Modulator*,
  *Modulator n*, *Rate CV*, *Input* or a jack's name**: those say that something is patched, not
  what it does, and the name is on every row of the stack.
- **A sound source keeps a mixer level of its own.** A ring modulator, a noise generator, a sub:
  anything the machine let you hear gets a plain level slider beside the oscillators', even where
  the plug-out shared its slider with an input jack. The jack becomes a routing input of its own —
  `mxm-mono-00`'s *Mix* — so the level is where a player looks for it and not only in a row.
- **No card chooses where its own output goes.** A destination switch, a depth knob wired to one
  source, a switch choosing a source: each is a route the conversion has not made yet. Make it a
  route into each destination it reached, present in the init patch where it pointed.
- **Every path the machine had stays reachable, including paths that summed.** Where two inputs
  used to carry the same source to one parameter at once, size the merged target's reach and bound
  for their concurrent sum, and read each input in its consumer's stage.
- **A source's name sets every card's width.** It appears in every target's rows, and the longest
  `<target> from <source>` sets the card's floor: *LFO 1 core reverse saw* cost `mxm-mono-00`
  forty points on eleven cards. Keep source names as short as the longest one already there.
- **A wide pitch route gets a square-law fader** (`FloatRange::SymmetricalSkewed`, factor 0.5,
  about zero) so a vibrato's depth is in the first tenth of the travel — the owner's answer to
  narrowing the reach, which they declined. The *Parameters* rule on pitch spans, applied to a route.
  **Only a machine pitch column wider than ±24 semitones** takes one
  (`mxm_modulation_params::reading::Fader`); every other amount is linear.
- **Every modulation means the same on every instrument** (the owner, 2026-09-26;
  `plans/plan-modulation-standard.md` (`plans/plan-modulation-standard.md` in the private archive)). Publish the
  performance sources through `mxm_modulation::standard` — Key the glided note, **Velocity `v − 1`**,
  Wheel and Pressure 0…1, Bend the lever, Random `2u − 1` — so every route does nothing at its
  source's rest. A path the machine had keeps the machine's reach; **an added path from a
  performance source takes the standard reach**: 12 st on a pitch (12 st/oct from Key), 4 oct on a
  cutoff or rate (1 oct/oct), 45 % on a width, the whole range on a control, ×0…×2 on Amplitude
  (20 %/oct from Key). **A generator pair the conversion added is not yet one rule**: `mxm-para-07`,
  `mxm-mono-pr1` and `mxm-poly-06` gave theirs the standard reach, while `mxm-mono-08` and
  `mxm-mono-00` kept their rows' network reach (mono-08's *Feedback sheen* is built on 2.5 octaves of
  complex-oscillator FM that 12 st could not hold) — the owner's to unify
  (`plans/plan-modulation-standard.md` revision 12).
- **Every instrument has a standard *Amplitude* target**, `level × amplitude_factor(Σ)`. A
  machine's CV-summing amplifier keeps its own law as a target named for it (*VCA level*) and the
  standard Amplitude sits after it; a dB level converts.
- **A pair that can never mean anything is not offered** (`standard::offer`): a gesture into a
  gate or sync, Velocity or Random into a sample-and-hold input, anything performance into an audio
  input. Its ids are retired, never reused. A target that discards a sign offers a performance
  source only its live half; **a machine or generator pair keeps both halves**, its discarded half
  subtracting from another route — except a sync depth, where a negative reset is no reset at all
  (`mxm-mono-00`). Read each amount through `mxm_modulation_params::reading`, and prove the
  instrument with `mxm_modulation::conformance` from its tests: `check_declaration`,
  `check_publishers` and `check_release_silence` in the DSP crate (at the voice's lowest sample rate
  where a tail takes seconds to reach exact zero, as `mxm-mono-08`'s optical gates do), and
  `mxm_plugin_test::routing_checks`'s `amounts` in the plugin, each falsified once.

A new instrument starts on it. An existing one converts under
`plans/plan-modulation-routing.md` (`plans/plan-modulation-routing.md` in the private archive) M5, which names the
conversions still owed. Read [`docs/code-review-notes.md`](https://github.com/mxm-audio/newdawn-workspace/blob/main/docs/code-review-notes.md) §7 before
converting: five of its lessons were found twice, on two conversions.

## Smooth signals, not coefficients

A parameter whose value is added to or multiplied by the audio needs smoothing; a parameter that
configures how something *behaves* does not, and smoothing it produces a control that lies about
what it is doing. Envelope times and glide time set state-machine behaviour and are left unsmoothed
for exactly this reason.

Measured: an unsmoothed continuous parameter stepping under automation puts broadband energy
**28.5 dB** below the signal, falling to −71.6 dB with 5 ms of smoothing — about 14 dB per decade,
with the knee around 1 ms. Useful range is **5–20 ms**; past about 20 ms the control feels
disconnected and fast automation is smeared.

Those figures come from a generic one-pole model in `crates/dsp-lab/examples/mod_spike.rs` §4,
**not** from this crate's shipped `SmoothingStyle` path, which has never been measured. See
[`docs/modulation/03-smoothing-and-events.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/modulation/03-smoothing-and-events.md) for
the argument and the open gap.

## Permanent identifiers — never change these

| What | Rule | mxm-mono-01's value |
|---|---|---|
| `CLAP_ID` | Reverse-DNS, permanent. Changing it breaks every saved project using the plugin. The project owns `mxm.dk`, so the form is `dk.mxm.<plugin>` | `dk.mxm.mxm-mono-01` — assembled from `plugin_name!` in `src/lib.rs`, **not** from `CARGO_PKG_NAME`, so renaming the directory cannot move it |
| Parameter `#[id = "…"]` | Permanent, never reused. Renaming the *field* is fine; renaming the id is not | see `src/params.rs` |

Treat both as public interface.

**A `CLAP_ID` cannot collide, and an earlier version of this rule said otherwise** (the owner's
ruling, 2026-09-05). It asked a new plugin to confirm its name was not taken on the CLAP plugin
lists before the id was fixed. That conflated two different things: the id is reverse-DNS under
`mxm.dk`, a domain the project owns, and every plugin in the collection lives in this repository —
so the namespace is ours alone and a check against other vendors can never find anything. Hosts key
on the id, not the name. What the id needs is care that it is **permanent**, which the paragraph
below covers.

A *display name* that duplicates a well-known plugin is a naming question and can be raised with the
owner, but it is **not a gate** and it never blocks an id.

**`CLAP_ID` has been changed once, and will not be again.** MXM-101 became `mxm-mono-01` while the
project was pre-alpha, undistributed and untagged — the only moment the change cost nothing. It
orphaned every file keyed to the old id: the user preset directory (which `preset::user_root` *names*
after `CLAP_ID`), its `favourites.json`, parked locks in the player's settings, and locks inside any
saved sequence. That loss was accepted deliberately because nothing had been saved. After a release
none of it would be acceptable, which is what the permanence rule above is protecting.

## This is nice-plug, not nih-plug

The community successor to nih-plug (which is in maintenance mode). The API differs in ways that
bite anyone coding from nih-plug memory:

| nih-plug | nice-plug |
|---|---|
| `nih_plug::prelude::*` | `nice_plug::prelude::*` |
| `nih_export_clap!` | `nice_export_clap!` |
| `fn initialize(...)` | **`fn activate(...)`** with `ActivateContext` |
| `editor() -> Option<Box<dyn Editor>>` | **`type Editor` associated type**, `-> Option<Self::Editor>` |

Every plugin implements `Plugin` + `ClapPlugin` and ends with `nice_export_clap!(MyPlugin);`.

- `CLAP_FEATURES` drives host categorisation. For instruments: `Instrument`, `Synthesizer`, plus
  channel counts.
- `AUDIO_IO_LAYOUTS` declares supported configs explicitly. An instrument has no main input; an
  effect has exactly one, and nice-plug cannot declare a stereo-in, mono-out main pair — a mono
  host takes the mono-in layout. A main pair of two different shapes is not an in-place pair
  (vendored fix 5).

## Parameters

One `#[derive(Params)]` struct behind `Arc<…>`.

- Give continuous params a `SmoothingStyle`; unsmoothed ones zipper. But **do not smooth values that
  are coefficients rather than signals** — envelope times, glide time — because it makes
  state-machine timing impossible to reason about.
- **Gain parameters store linear gain, not dB.** `SmoothingStyle::Logarithmic` across a dB range
  spanning zero is mathematically invalid and trips a debug assertion. Use `FloatRange::Skewed`
  between `db_to_gain(min)` and `db_to_gain(max)` and format as dB for display.
- Use `FloatRange::Skewed` / `SymmetricalSkewed` for frequency and gain so controls feel right.
- **And for a continuous pitch span wider than a couple of semitones.** A detune is a few *cents*;
  linear across an octave or more, every musically useful setting sits inside the first pixel or two
  of the collection's full-scale drag (`mxm_ui::control`'s `DRAG_TRAVEL`). Two spread depths have
  met this and been corrected after the owner played them — `mxm-creative-sampler`'s grain spread
  and `mxm-grain-fx`'s Pitch variation. **The reading has to resolve below a semitone as well** —
  cents, or semitones to two decimals — or the curve moves the sound while the panel says nothing
  about where the control is. A control stepped in semitones is exempt and stays linear: every Bend
  range in the collection, and `mxm-shimmer`'s Shift. (Only `mxm-mono-01`'s and the sampler's Bend
  ranges declare `with_step_size(1.0)`; the other six are continuous and read whole semitones, and
  the keyboard's `StepLaw::Semitones` steps them from the semitone shown.) The open case is a
  **bipolar continuous tune**, where `SymmetricalSkewed` about zero is the tool;
  `mxm-creative-sampler`'s Tune is still linear across four octaves and has not been reported.
- **A reading that switches units chooses the unit from the rounded reading**, never from the raw
  value, or the host's text round trip is not idempotent at the switch: a raw `< 1.0` printed
  `1000 ms` for a value that parses to one second and reads back as `1.00 s`, in nine plugins at
  once. `clap-validator`'s fixed grid misses that sliver; only its random values find it. Those
  nine hold every milliseconds-to-seconds control with `mxm_plugin_test::time_text_checks`, used from
  `params.rs`; `mxm-grain-fx` and `mxm-creative-sampler` hold theirs with their own round-trip
  tests, and `mxm-mono-pr1`'s cutoff is the same rule at a kHz boundary.
- User-facing names are sentence case and not cryptic — `Cutoff`, `Filter envelope`, not
  `FREQ` / `F.ENV`. The same name and unit appear in the editor and host automation. A repeated
  module prefix may be omitted inside that module's card/section (§7.1); accessibility and tooltips
  retain the full name. An assignable amount is named for its role, not its default source.
- **No help text on the panel** (the owner, 2026-09-27; design system §7.6): no sentence under a
  control or at a card's foot explaining how something works. Live readings, status and a
  display's legend stay; the explanation is the parameter's tooltip (its `description`) or a
  display's hover text, **written for the player** — what the control does to the sound, never the
  circuit's or the research's vocabulary. Every editor was cleared on 2026-09-27; a new card's
  explanation starts as a tooltip. **Every editor's sentences were rewritten the same day** (the
  owner, on mono-00's *the plug-out's own control*), and each plugin's `speaks_to_the_player` tests
  hold it (`mxm_plugin_test::hover_text`): they read that editor's sentences and fail on the
  machine's, the history's and the code's words. Run them after writing any tooltip, hover or status
  line: `cargo test -p <plugin> --lib speaks_to_the_player`. **The host description is the other
  way round on technique**: `CLAP_DESCRIPTION` is what a plugin browser shows, so it says what the
  plugin is *and what sets it apart* — *bucket-brigade (BBD) chorus*, *drums modelled from their
  circuits*, *diode-ladder filter* are its selling points (the owner, 2026-09-27, after a pass that
  removed them) — but never a maker's or model's name (root *Naming*) or the code's own words
  (checked by the same test).
- **A syncable rate or time shows its reading** (the owner, 2026-09-27; design system §7.1): hertz
  or seconds free, the note when synced, always — Standard at least, never Compact. mxm-mono-00's
  LFOs are the model; mxm-para-07's Rate and Sample time were Compact and were the ones that hid it.
- **Each button of a row says what it does** (the owner, 2026-09-27; design system §7.3): a
  segmented control's cells each carry their own hover sentence, written for the player, never one
  sentence for the row. A plugin keeps them beside the parameter's own sentence — `Bound::details`,
  filled from a `details_of(id)` table (or its own table, as creative-sampler's `Switch::details`
  and the effects' `*_DETAILS` constants) — and the shared control asserts one per option, so every
  test that paints the card checks them. Every row in the collection was written on 2026-09-27.
- **A range is buttons** (the owner, 2026-09-27; design system §7.3): an oscillator's footage or
  octave is a segmented switch in every editor, never a caret selector or a drop-down — six
  positions included (mxm-mono-00's 64'–2').
- Map resonance around the filter's **measured** oscillation threshold, not the theoretical one.

### Tempo sync is one button changing one Rate or Time control

**Owner ruling, 2026-09-20:** all user-adjustable LFOs offer host-tempo sync, and delay/LFO sync
looks the same across the collection as far as each card permits. The interaction is deliberately
small: one adjacent **Sync** on/off button. Off, the existing Rate reads hertz or Time reads seconds;
on, that same control snaps across musical divisions and reads `1/16`, `1/8.`, `1/4T`, and so on.
There is no separate Division control or parameter. A Glide/Snap transition law is a separate
control, not extra Sync states. Missing or invalid tempo falls back to the free value and never
assumes 120 BPM.

**One way, everywhere (the owner, 2026-09-25):** *"add sync to everything that can be tempo synced in
all plugins"*, and *"1 common way it works. If they have different ranges of time we keep to that"*.
Every synced control follows one contract:

- **The ladder is `mxm-tempo`'s** ([`../crates/mxm-tempo/AGENTS.md`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-tempo/AGENTS.md)):
  a `const <CONTROL>_SYNC: Ladder` beside the parameter in `params.rs` — its span of the one
  sixteen-step table (the control's own range at 120 BPM; every LFO `Span::LFO`) and its direction
  (`Time`: the longest division at the top; `Rate`: the fastest), so a synced control moves the way
  it moves free. Never a plugin-local division table.
- **One `BoolParam` per synced control**, default Off, named *<Control> sync*; existing ids are kept.
- **Resolved once per block** from `context.transport().tempo` and the control's modulated position
  (`Ladder::resolve`), into an `Option` the patch applies as `synced.unwrap_or(free)`; the free
  smoother is advanced exactly as it is with sync off, so turning sync off lands where the free
  control would be (where a plugin parks and advances nothing, it advances nothing either way); and
  routes, octave amounts and offsets apply on top exactly as on a free value. The tempo in force is
  published in a `TempoCell`.
- **Drawn as the quarter note** (`binding::sync_picture`, a square button, `Kind::SyncToggle`) beside
  its control on the knob's grid, `tree::switch_gap` from the circle (`tree::switch_beside_knob`); synced, the control reads the division (`Bound::knob_with_reading`,
  `Ladder::shown` against the `TempoCell`) and its free value when there is no tempo; its column holds
  both (`binding::synced_widest`).
- **The knob, its quarter note and what follows are one flat row**, the quarter note straight after
  its knob: the row measures its gap from that knob's circle. A row inside a row keeps the
  width it is offered, so a nested knob row pushes the next control across a wide card; split the knob
  row around the quarter note instead (para-07's Lag was the case).
- **A preset file or a project from before a sync loads it off.** Each plugin lists its sync ids in
  `preset.rs`'s `TEMPO_SYNC_IDS` and opts them into `Instrument::default_missing_legacy_parameter`, so
  an old file neither keeps an instance's sync nor reports a missing control
  (`a_preset_from_before_the_tempo_syncs_loads_them_off`); and its `Plugin::filter_state` calls
  `mxm_preset::add_switches_off` with them, so an old project restores each Off (nice-plug restores
  only what a state names) and a loaded preset's baseline gains it, staying clean
  (`an_older_state_restores_the_tempo_syncs_off`).
- **A cached sync does not outlive an activation or a reset.** An effect that seeds its engine from
  its controls clears its resolved values at `prepare`, and at `reset` — which a host may call with no
  callback between, after a parameter flush — re-resolves them from the parameters and the last tempo
  seen, so a restored or flushed state never starts at the previous division (mxm-shimmer also sets
  its two synced smoothers at their targets there).
- **A division arrives the way a turned knob does.** Where the DSP ramps the control itself (a delay's
  glide, a convolution tap, a reverb engine) the resolved value goes straight in; where the engine
  reads a position every sample with no ramp of its own, it goes through the control's own smoother
  (mxm-shimmer's `follow_tempo`), never at once. A rate or a per-grain draw needs neither.

`../plans/plan-tempo-sync-controls.md` (`plans/plan-tempo-sync-controls.md` in the private archive) owns the rollout
inventory.

### Editable models: what is a parameter and what is state

When a plugin exposes an editable model — a filter voicing, a calibration, anything with a "save as
your own" — the controls inside it are **plugin state, not automatable parameters**.

| | Automatable parameter | Persisted state only |
|---|---|---|
| What the player performs (cutoff, resonance, drive) | yes | |
| The model selector | yes — one stepped parameter | |
| The model's internal controls | | yes |

Exposing a dozen voicing controls as parameters puts a dozen permanent `#[id]`s into the public
interface, clutters every host's automation list, and invites automation of values that are not
signals. Version the state so an added control does not break an old preset.

**This is about editable *models*, not about advanced controls in general**, and the two are easy to
confuse. What makes something a model is that a person authors it: a set they can extend, name and
save as their own, reached through a selector. A **fixed set of extra controls behind a disclosure
is not a model** — it is a known list the plugin ships, it cannot grow at a user's hand, and it
needs no selector or versioning. Those are ordinary automatable parameters, and `mxm-mono-01`'s
disclosed `bendrange` is the precedent.

The line is *who can add to the set*. If the answer is "only a release of the plugin", they are
parameters; if it is "anybody, at runtime", they are state. What carries either way is the rule
below: **a constant is not a signal and must not be smoothed.**

Applying a model edit is a **control-rate** operation: recompute coefficients at a block boundary,
never mid-sample. The performed parameters stay smoothed; the model's constants are not signals and
must not be smoothed.

A user-authored model is user-generated input. **Bound its ranges in the DSP**, not only in the
editor.

Worked example, with measurements:
`research:filters/machines/ssm2040-cem3320-prophet.md` §12.

## A plugin ships its own control map

`plugins/<plugin>/control-map.json` says which of this plugin's parameters fill the collection's
control roles. `cargo xtask bundle` stages it beside the `.clap` as
`<Bundle Name>.control-map.json`, and MXM Player loads it from there.

**It lives with the plugin, not with the player.** Nobody is obliged to install the whole
collection: someone who downloads one plugin must still get a working controller layout, and the
player must not need to have heard of a plugin released after it.

- Roles come from [`docs/MXM_CONTROL_MAP.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/MXM_CONTROL_MAP.md), which is normative. Do
  not invent a role name; add one there first, in a pass that also updates the standard's
  `roles` table.
- Reference parameters by their **permanent string `#[id]`**, never by display name.
- **Leave a role out when the instrument does not have it.** mxm-mono-01 has one oscillator and one
  shared envelope, so it fills no `osc2.*` or `filter_env.*` role. Those slots stay inert, which is
  what lets a two-oscillator instrument fill them without anything moving.
- A file naming a role the standard does not declare is refused, with the typo named.

## An instrument ships the effects its original had, and no others

The binding form of the root's *no effect that was not on the original instrument*
([`../AGENTS.md`](../AGENTS.md)). Two principles meet here and the order between them is settled:

1. **Effects are their own collection.** A distortion, a delay, a reverb is a separate box, so it is
   a separate plugin (the owner's decision).
2. **Except where the machine had one built in**, in which case it is part of the machine and
   belongs in the instrument. This overrides (1), because an instrument missing an effect its
   original had is not a faithful copy of it, and no amount of "you can chain one" makes it one.

**One instrument is exempt by the owner's ruling (2026-09-02): `mxm-mono-00`**, whose feature set
is Roland's SYSTEM-100 plug-out and which therefore takes the plug-out's phaser and delay although
no System-100 module had them. The root's effects bullet holds the exemption; it is specific to that
instrument, and `mxm-mono-00/AGENTS.md` carries the record below.

**A standalone effect may be granted a path its original did not have, by the owner's ruling and
never by default.** `mxm-folded-spring`'s Feedback is the first: the tank's return driving its own
input, which no tank does by itself. The bar is the same as the plug-out exemption above — a ruling,
recorded in the plugin's own doc, with the evidence — and there is one more requirement because
this is an addition rather than an omission: **the added path must start at zero and cost nothing
there, provably**, so the plugin as it loads is still the circuit. `feedback_at_zero_is_the_bare_tank_to_the_bit`
is what that looks like.

**Record the call in the instrument's own AGENTS.md**, whichever way it goes, with the evidence
about the hardware — so a later reader sees a decision about that machine rather than an omission.
`plugins/mxm-mono-03/AGENTS.md`'s *There is no distortion* is the shape, including which arguments
it names as not being arguments: *the clones add one* and *it is most of the recorded sound* are
both true of the 303 and neither is about the TB-303.

**A built-in effect does not need the player to host audio inputs.** Worth stating, because the two
are easy to conflate: an instrument with an internal chorus is still MIDI in, audio out. The player
now hosts *standalone* effects — a plugin with exactly one audio input — in a serial chain after
the source (`apps/mxm-player/AGENTS.md`, *The effect chain*); that is the host's business and
changes nothing about a built-in one.

## Every effect built into an instrument is also promoted to a standalone effect

**The owner's ruling, 2026-09-04.** An effect that exists inside an instrument has already had the
expensive half done: the DSP is written, measured or argued, and tested. Promoting it is a wrapper,
a pair of layouts, an editor and a preset file — *low-hanging fruit*, in the owner's words — and it
is how one implementation reaches both a player of that instrument and a track that needs the
effect on something else.

So the default answer for a built-in effect is **yes, it gets a standalone too**, and a decision
not to promote one is the thing that needs an argument in the instrument's own doc.

Three rules make it cheap and keep it honest, all proved twice now by `mxm-chorus-06` and
`mxm-folded-spring`:

- **Depend on the instrument's DSP crate in place. Never copy it.** One implementation, and the
  instrument's render stays provably unchanged.
- **The DSP crate may gain inputs, never a behaviour change.** The instrument's own default must be
  what it always was, and the promotion must pin that — by golden digest, taken on the revision
  before the addition, with a file that compiles against both. `crates/mxm-mono-00-dsp/tests/spring_tanks.rs`
  is the worked example.
- **The standalone may open what the hardware fixed**, which is usually the reason to want it: the
  chorus's rate and depth, the spring's tank. What is measured stays the default, and what is
  chosen says so.

An effect whose DSP is not in a crate of its own yet is not an exception to the rule, only work
before it: extract it as this collection extracts anything, on the strength of the second consumer.

**Promoted so far**: `mxm-poly-06`'s chorus (`mxm-chorus-06`) and `mxm-mono-00`'s spring reverb
(`mxm-folded-spring`). **Waiting**: `mxm-mono-00`'s phaser and delay, which
`plans/plan-mxm-fx-collection.md` §1 holds back until each has a researched reference box and a
creative identity — not a doubt about promoting them, a doubt about what to call them.

**`mxm-bucket-delay` is not that promotion**, and the names are close enough to be worth separating
here. It is a new product built on the bucket brigade device
(`research:effects/bucket-brigade-delay.md`, `docs/briefs/mxm-bucket-delay.md`). `mxm-mono-00`'s
delay is a clean interpolated line modelling a *software* delay's chosen model; turning it into a
bucket brigade would change the instrument's render, which the second rule above forbids. Two
delays, two reference boxes, and the instrument's own one is still waiting.

## A developer channel in every editor

**Every plugin with a note port carries it, and it is off unless the environment asks for it.**
With `MXM_DEV_CC` set in the plugin's process environment when an instance is made — the player
started from a shell that exported it — two otherwise-undefined control changes drive the editor:

| CC | Value | Does |
|---|---|---|
| **119** | 0 Performance, 1 Modulators, 2 Sequencers, 3 Generators, 4 Tone, 5 Effects; 127 Parameters | selects the category's first card, or the separate diagnostic surface; other values ignored |
| **118** | ≥ 64 opens, < 64 closes | the editor's expander, where it has one |
| **117** | ≥ 64 opens, < 64 closes | the preset browser under the app bar |
| **116** | 0 light, 1 dark, 2 system | the app bar's theme control (`mxm_ui::theme::from_index`) |

**CC 116 changes the theme and does not remember it.** The control in the bar writes the person's
choice to `<config>/mxm/editor.json`; this channel exists so a screenshot run, a test or an AI can
see the other theme, and a capture that rewrote what somebody chose would be a debug facility with
a side effect. Same reasoning as `MXM_EDITOR_THEME`, which wins over the file for the same run.

Through the player, `mxm-cli cc 119 1` selects Modulators and `mxm-cli cc 119 127` selects
Parameters. Category requests are independent of window-dependent tab positions; navigation waits
through gestures, text entry, browsers and popups. Controller maps and CC 110/111 do not change. **Why it exists**: CLAP gives a host no way to ask a plugin's
editor to do anything — the player's CLI drives the player, not the window inside it — and the
owner's rule is that whoever builds an instrument must be able to debug all of it themselves,
views included. **Why the environment and not a parameter or a button**: a parameter shows in
every host and can be automated, a button can be pressed; an environment variable is invisible to
anyone who did not set it, and no host, preset or automation lane can trip it.

The shape is the same in every plugin, deliberately, so the verbs are the same too: five
constants in `lib.rs`, a `dev_cc: bool` read once at construction, four arms in the `MidiCC` match
guarded by it, four request slots on `Telemetry` (taken once — atomics, the only channel from the
audio thread to the editor; the DSP reads nothing), and the editor honouring them at the top of
`panel`. A plugin with no expander answers CC 118 with nothing, and says so. Each plugin's
`the_developer_channel_is_off_unless_the_environment_asked_for_it` holds the gate.

`scripts/capture_editor.ps1` is the screenshot run that channel exists for: it opens an editor
through the player, chooses a category (`-Category`, or its permanent CC address via `-View`),
theme (`-Theme`, via `MXM_EDITOR_THEME`), zoom and size, and captures the editor window rather than
the screen. It selects the first card of a category, not every page in a split category. With
`-Launch` it puts the owner's `settings.json` back afterwards, because the player it starts saves
into the real file as it goes (an effect capture's `fx add` stayed in the chain for good). Keep it
working when navigation or the app bar changes; native capture and its zoom coordinates need
manual verification.

**An effect carries none of it, and that is not an omission.** The channel is control changes, and
an effect has no note port — which is what makes it an effect. The player's `cc` verb reaches the
loaded instrument, never the chain after it, so there is no route even for a plugin that declared
one; giving an effect a note input for a debug facility would be a lie about what it is in every
host that lists ports. `mxm-chorus-06` records the deviation in its own doc and in `telemetry.rs`
where the request slots would have gone. Its editor has one view and no expander, so what is lost
is the preset browser's request and the theme's, and nothing else. **An effect still carries the
theme control itself** — the channel is how a script reaches it without a mouse, not how a person
does, and an effect's editor is a place a person works.

**An expander's state lives where its tree reads it, so the request writes it there directly.**
A card's disclosure is part of its `mxm_ui::tree`, built each frame from the state it finds: the
shared disclosure keeps its open flag in egui memory under `shell::disclosure_id(label)` (mono-00's
and mono-01's *Advanced*), and a collapsing header under `tree::collapsing_id(title)` (mono-02's and
poly-06's *Bender*, para-07's *Setup*). Both ids are the tree's own, not a widget's position-derived
one, so `panel` sets the requested state before any card is built and the card opens that frame.
egui's `CollapsingHeader`, whose state key only its own call can name, is no longer drawn in a card.

**And the editor asks for a frame every 50 ms while it is open.** A request is taken when `panel`
runs, and egui runs `panel` when something asks it to; on a view where nothing animates that was
the pointer, so a view switch sent over the channel waited for a mouse that was not there
(mono-02's Parameters view, seen). The meter wants the cadence anyway. Closed, nothing runs.

## Per-note pitch expression is accepted; nothing else per-note is

Every instrument handles **`NoteEvent::PolyTuning`** — CLAP's `CLAP_NOTE_EXPRESSION_TUNING`, which is
what a pitch curve drawn against one note in a DAW's piano roll arrives as. It is semitones, it
sums with the channel bend and the tuning control, and it reaches the DSP as a patch field of its
own (`expression_semitones`) rather than folded into bend, so the patch can still say which of the
two moved.

**Routed by note, because an expression names one.** A monophonic instrument plays one note, and an
expression for any other note belongs to no gate here — applying it would bend a note the host never
asked to bend. mxm-mono-03 asks its own `sounding`; mxm-mono-01 asks `Voice::sounding`, because its
note stack can hand the voice back to an older held key on a note-off and a plugin-side mirror of
"the last note-on" would then be wrong.

**Not smoothed**, exactly as bend is not: the host delivers a stream of timed events and `process`
already splits its block on each one, so a smoother would blur a value that is *already*
sample-accurate.

**A non-finite tuning is dropped, and the note keeps the offset it had.** A NaN in the pitch sum
reaches an oscillator's phase and never leaves it. The plugin's `PolyTuning` arm refuses it, or the
DSP call the arm makes does (`mxm-para-07`); `mxm-mono-02-dsp`'s keyboard block and
`mxm-poly-06-dsp`'s ledger refuse it again. `a_non_finite_tuning_expression_is_dropped_and_the_pitch_stays_finite`
in `mxm-mono-01`, `-02`, `-03` and `mxm-poly-06` renders against an instance never sent it.

**A new note clears it; a note-off does not.** The offset a note was bent to holds through its
release, where zeroing it would snap the pitch back mid-tail. The note that begins a slide is a new
note, gate held open or not, so it starts clean.

**But the voice moving to a different note does clear it**, which is mxm-mono-01 only and is the
non-obvious case: its note stack can change what is sounding *without a note-on*, when releasing the
newest key under a chord returns the voice to an older held one. The bend drawn against the key that
left must not follow the voice onto the key that stays, so the plugin remembers which `NoteId` the
offset belongs to and drops it when `Voice::sounding` reports a different one — and only a
*different* one, never `None`, because going silent is a release and not a move. mxm-mono-03 has no
stack: the only thing that changes its sounding note is a note-on, which already clears.

**The other expressions are still dropped** — `PolyPressure`, `PolyVibrato`, `PolyExpression`,
`PolyBrightness` and CLAP polyphonic *parameter* modulation. They reach `handle_event` (every plugin
declares `MidiConfig::MidiCCs`, which clears the wrapper's `>= Basic` gate) and fall through to
`_ => {}`. That is the honest state for one voice: they are per-voice quantities, and there is one
voice.

**The polyphonic instrument answered it, and the answer is per instrument.** `mxm-poly-06` makes the
pitch expression per-voice state, routed by its ledger to the presses an expression names. It still
drops per-note pressure, vibrato, brightness and expression — **no longer because the machine has
nothing for them to drive**: `plan-modulation-routing.md` decision 1.7 made velocity and channel
pressure routing sources on every instrument, and that instrument's conversion gave them per-voice
targets. What it declines is the *per-note* form, because decision 1.7 names channel pressure and MPE
is out of scope. It also declines `CLAP_POLY_MODULATION_CONFIG`, which nice-plug couples to the
voice-info extension, for a reason the routing did not change: that extension advertises a host
offsetting a *parameter* for one voice, and routing added per-voice sources, not per-voice parameter
destinations.

## Realtime rules for `process()`

Non-negotiable, and the source of most "fine in tests, crackles in the DAW" bugs:

- No allocation, no `Vec::push`, no `format!`, no logging.
- No locks, no `Mutex`, no blocking channels.
- No file or network I/O. Deferred work goes through `BackgroundTask` / `AsyncExecutor`.
- GUI durable-state restore is a control-thread transaction in the patched wrapper, never callback-
  deferred: it holds the plugin lock through field restore and reactivation, and restores the complete
  prior snapshot if a persistent field or activation rejects.
- Preallocate in `activate()`, which knows the max buffer size. Clear state in `reset()`, which must
  leave no tail from the previous playback.
- Cap internal blocks: `min(next_event, host_end, start + MAX_BLOCK)`.
- Return `ProcessStatus::Tail(n)` while releasing so hosts do not cut the tail; `Normal` when truly
  idle. Tail must reflect *audible* output, not hidden pre-VCA state.
- **Doing nothing costs nothing** — the owner's rule for the collection, 2026-09-04, and it has two
  halves. An effect or a synth with nothing to do reports a status the host can sleep on — `Normal`,
  which nice-plug maps to *continue if not quiet* — **and** runs none of its core while it has none.
  `mxm-chorus-06` parks its chorus at Mix zero once the mute has closed: the block costs a copy.
  Reporting alone is half the rule.

`assert_process_allocs` aborts on any allocation in `process()` — but **only in debug builds**. A
release validator run does not prove absence of allocation.

## Editor contract

- Editors are transient. The DAW opens and closes them at will, and audio must render normally with
  no editor open. **DSP never reads editor state.**
- **The editor is a panel, not a window.** `editor::panel` takes a `Ui` and draws into it; it does
  not create a window, run an event loop, or own a swapchain. That is what lets the same code be
  the CLAP editor and the standalone harness's contents, and it is what keeps native GUI hosting —
  deferred, not abandoned — from being foreclosed.
- **An editor frame never blocks on a modal OS loop.** A native file dialog opened inside `ui()`
  pumps the plugin window's messages while egui-baseview is still mid-frame; the re-entered handler
  panics inside a window procedure, which cannot unwind, and the host aborts (Browse took the player
  down this way, 2026-09-15). Run such work through `mxm_ui::offthread` and act on its result on a
  later frame, exactly as on a dropped file.
- Durable numeric Editor → DSP changes go through the parameter system, with correct
  `begin_set_parameter` / `end_set_parameter` bracketing per gesture, or automation records as
  unrelated jumps. **Immutable source audio is the creative sampler's narrow exception:** file
  acquisition is not a numeric parameter and cannot truthfully be automated. Its local AGENTS.md
  owns the bounded background prepare → complete snapshot publish → off-audio retire transaction;
  source-owned root/region/loop endpoints remain ordinary parameters.
  **`mxm-fx-curve` is the third narrow exception, granted by the owner on 2026-09-16:** its ordered
  point-and-stage model is durable numeric state but cannot truthfully be flattened into a fixed
  host parameter list. Its local AGENTS.md owns bounded decode, complete off-audio preparation,
  revision-ordered publication, one transition route and off-audio retirement. Input gain, Auto
  makeup and Mix remain ordinary parameters, model edits remain undoable state transactions, and DSP still reads no editor
  state. During an open drag it may prepare and publish complete intermediate engines as audible
  previews, but those previews change no durable state, history, fingerprint or host dirty flag and
  are rolled back if the gesture is abandoned. Release commits once before sending an
  already-committed, parameter-free `GuiContext::set_state` transaction; the vendored wrapper marks
  that GUI transaction dirty without taking the processor mutex or marking host restores.
- **A genuinely transient, non-value action may use a plugin-owned command path.** This is a narrow
  exception, not a second parameter system: the plugin's local AGENTS.md must name the action and
  explain why saving, recalling, automating, or host-modulating it would be false. The action has no
  parameter id, preset/state value, or control-map role. Its GUI-side producer is preallocated and
  non-blocking; distinct activations neither coalesce nor duplicate, and a bounded path reports a
  rejection instead of silently dropping it. The plugin shell consumes commands at a documented
  process boundary and calls the DSP's ordinary event operation; DSP still reads no editor state.
  Reset and panic cancellation, editor close/reopen, and races at the cancellation boundary are
  defined locally and tested. Exact counters, queues, capacities, and memory ordering belong to
  implementation, not a plan. This exception exists for acts such as a hardware momentary firing
  excluded from recall, not for a control that happens to be inconvenient to model as a parameter.
- DSP → editor telemetry uses atomics or a triple buffer, never a mutex read by the audio thread,
  and may drop frames. Throttle or stop it when the editor is hidden.

  mxm-mono-01 expresses this as a struct rather than a convention: `src/telemetry.rs` is the **only**
  channel, `Arc`-shared, atomics only, written once per block. Two rules in it are worth carrying
  to the next instrument — a **peak is max-combined and reset on read**, so a frame the UI missed
  cannot hide a transient, and a **clip latches until the user acknowledges it**, per design system
  §5.4. A meter that quietly forgets is worse than no meter.
- Widgets and theme come from `crates/ui`. Plugins supply labels, bindings and data, not styling.
- **The zoom control is `mxm_ui::shell::zoom_control`**, as the theme control is
  `mxm_ui::shell::editor_theme_control`. Collection-wide controls belong to the shared crate, not a
  local copy.
- **The Parameters view has no tab.** It stays implemented and reachable by the developer channel,
  which is what the CLI and a host's automation list use; it is simply not a tab in front of a
  musician. Its developer address is 127; do not include it in the paging inventory. It scrolls in
  `mxm_ui::shell::scroll_list`, which keeps its last column clear of the scroll bar.
- **No utility menu for Init.** Init belongs to the preset browser; duplicating it behind a `⋮`
  creates two controls for one action.
- **Every editor carries the theme control, and it is `mxm_ui::shell::editor_theme_control`.**
  §3.1 slot 5 and §10: Dark, Light and System, switching immediately. One widget in `crates/ui`
  rather than one per plugin — a bar each product arranges for itself is how a collection stops
  looking like one — drawn at the **left end of the bar's right-hand group**, which is where the
  player keeps its own. The player draws the same control from the same module.

  **The choice is remembered in `<config>/mxm/editor.json`, one file for the whole collection**,
  because somebody who chose dark chose it for the collection and not for one instrument. Not in
  plugin state: that is where presets and a host's project live, and §10 forbids a preset carrying
  the theme. A write that fails is not reported — the theme is on screen either way, and an editor
  inside somebody else's DAW has nowhere better to say it.

- **The theme an editor opens in comes from `mxm_ui::theme::preference`, never a literal.** In
  order: `MXM_EDITOR_THEME=dark|light|system`, then the saved choice, then Light — because egui's
  `System` resolves against a system theme egui-baseview does not report and falls back to Dark,
  and an editor must not open dark by accident.

  The environment variable wins and is **never written back**: it exists so a screenshot run, a
  test or an AI can have a known theme, and a capture that changed what somebody chose would be a
  debug facility with a side effect. CC 116 is the same bargain at runtime.
- **Every editor reflows, and every editor carries the scale control.** The window resizes and the
  cards wrap into rows (`mxm_ui::flow`); the 75–200% zoom steps stay in the app bar because they are
  a different thing — how large the instrument is *drawn*, not how much of it fits. Zoom is still
  **chosen, not derived from the window**: deriving it feeds back, zoom sets size, size sets zoom,
  and the window jitters. Every editor walked into that once, and reflow does not repeal it.
- **An instrument's master output is in the app bar; an effect's Mix stays in its card** (owner,
  2026-09-18; design system §3.1 item 6). Every instrument draws its one overall output level as an
  inline slider beside the level meter, through `mxm_ui::navigation::bar_card` and a binding
  `slider_inline` that passes `ParamView::widest` so the bar does not move while it is dragged. It
  never sits beside a single voice's, slot's or layer's controls. Effects keep Mix, and any output
  level, as knobs in their Output card.
- **Musician surfaces use `mxm_ui::paging::editor`, including every effect that uses cards.** Supply stable
  card keys, primary categories, kinds, floors, ceilings and same-category preferred groups, and
  each card's body as a `mxm_ui::tree` (`crates/ui/AGENTS.md`, *A card body as data*).
  **A card's floor is computed**: `tree::card_floor` from its tree, or a usability minimum the
  plugin declares beside it where that is larger (design system §4.3).
  Page count follows width and height, not authored Synth/Mod/FX assignments; one page has no bar.
  Cards always receive top-down body Uis, never a horizontal row's inherited layout.
- **Nothing is drawn to learn a size.** A card's tree is built each frame from parameters, editor
  state and text formatted from telemetry; a plugin states its own visuals' sizes beside their
  drawing, and a size never comes from a telemetry value. Read destructive telemetry once, before
  `show`. What is not shown is reserved — a disclosure's body, a `Reserve`'s alternatives. Call
  `hold` for exact entry and preset browser/naming/search before developer navigation. The shared
  renderer defers replacement through pointer release and popups.
- **Every card passes the tree checks in every state that changes it.** `tree_checks::card`
  (`mxm_plugin_test::tree_checks`) draws the card as the paging renderer does, in the room it plans
  from the reserved height, and holds the floor, the exact content floor, the stated height against
  the drawn one, the card at exactly its room, every leaf taking only its own rectangle and painting
  over no other leaf's, nothing painted outside the card, and no reading — monospace value text —
  cut off with an ellipsis (a status line or file name that shortens is proportional and exempt);
  `MXM_FLOORS=1` prints each card's
  numbers. Each plugin runs it over its
  structural-state matrix — Init, every route revealed at its widest reading, disclosures open and
  closed, and its editor-state variants. `tree_checks::pictures` renders every page, light and
  dark, for review.
- **Category order precedes authored order:** Performance, Modulators, Sequencers, Generators,
  Tone, Effects. A mixed-purpose card stays intact with one locally documented primary category.
  Former cross-category groups are not a reason to change controller-page mappings.
- **Test what the editor *paints*, not only how much room it takes.** A layout test that measures
  the panel's height passes a panel that is one row wide and entirely off screen, and it passes a
  label that has been squeezed to seven points and wrapped into a column of letters. Both shipped in
  `mxm-bucket-delay` and both were invisible to its fit test. Read egui's own shape list —
  `Context::run_ui` returns `FullOutput`, whose `shapes` carry every `Shape::Text` with the rect it
  landed in — and assert that each control's label is there, inside the window, and not taller than
  it is wide. `plugins/mxm-bucket-delay/src/editor.rs` has the three tests; they need no dependency
  and take a millisecond.
- **A plugin can be auditioned through the player with no window and no sound card**, so do it
  before calling an editor or an effect done: `mxm_player_harness::harness::Harness::with_fx`
  builds the real chain and renders it, from the plugin's own `host-tests` package. See
  `apps/mxm-player/AGENTS.md`'s *Auditioning a plugin here*.
- **Cards in a row start and end on one line, and a row is as tall as its tallest card** (§3.3).
  The paging renderer enforces that floor before painting; a plugin owes honest measurements.
  `shell::level_columns` remains for non-paged consumers, not plugin musician pages.
- **The window's minimum is one card wide**, not one group: grouping is a preference the packing
  applies while it can, never a floor on the window (§4.3). Declare it through
  `ResizeHint::size_constraints`. **It is never narrower than the app bar at its last compact step**
  (owner, 2026-09-26): each editor's `the_app_bar_holds_in_the_minimum_window` runs
  `opening_size::bar_holds_from_the_minimum`, which fails with the width to take when the `…` menu
  would be cut or anything in the bar drawn over anything else. Where an instrument's master output
  and meter make the bar the wider, the bar sets `MINIMUM`.
- **An editor opens at the quarter-4K budget, hugged** (owner, 2026-09-09) — design system §4.3.
  Lay the panel out at the budget, take away the slack **every page** leaves, and that is
  `REFERENCE`: a window hugged to the opening page alone is as small as that page happens to be.
  `mxm_plugin_test::opening_size::derive` visits every planned page and does it, and
  `opening_size::is_the_budget_hugged` is the standing check each editor carries, to a gutter's
  tolerance; both judge the panel with disclosures open, which is what §4.2 asks for. **A test that
  wants a particular card must request it**, not assume the opening page still holds it: which
  modules those are follows the measurement and moves with it.
- **Every editing page must fit the quarter-4K budget in design-system §4.2.** Record physical
  window size and DPI, include shell/chrome, and test painted content for overlap, not just card
  bounds. Keep the physical window fixed when testing increased zoom. This is not proof that
  existing editors comply; verify each edited plugin. Compact controls may improve density without
  lowering the shared typography/pointer floors or imposing a collection-wide page count.
- **Check every derived page at the opening size with disclosures open.** Total unpaged height does
  not set the window size. Use `paging::editor::report` and painted shapes, not
  `globally_used_rect`, which includes the whole central panel. Fitting pages do not scroll;
  indivisible overflow retains both-axis reachability. `mxm_plugin_test::paging_checks` is the shared check
  for real panels at 1×/2× with a fixed simulated physical budget; this does not operate
  native zoom controls or establish physical DPI. Tall whole-surface canvases prove component geometry.
- **A column test keeps passing on a layout with no columns.** Every editor here had one, and each
  went on holding "the columns end level" while nothing lined up across the panel. Assert **rows**,
  read off the drawn rectangles, which is what a person sees.
- A §14 design brief in [`docs/briefs/`](https://github.com/mxm-audio/newdawn-workspace/blob/main/docs/AGENTS.md) is written **before** the editor.

### Private editor surfaces are tested in process

The player’s conformance sweep covers host-visible state. Presets, views, zoom and naming have no
CLAP host-to-plugin control protocol and are tested by each plugin’s egui harness plus the player’s
editor hosting tests. Do not invent a CLI protocol for private editor state.

## The keyboard cursor runs in every editor, and each one owes it three things

`mxm_ui::navigation` is tracker-style keyboard editing — `Shift`+arrows between modules/cards,
`Command`+arrows between the parameters inside one, bare arrows for the value, left/right fine and
up/down coarse, and `Alt` for a finer layer of both (design system §11). `mxm_ui::navigation::running` keeps a *surface* without a cursor — the developer
Parameters list — on bare-arrow editing.

An editor is three things, and no new tables:

- **`panel` takes a `&mut mxm_ui::navigation::State`**, held in the app struct beside `view` and the
  text buffers. It is transient editor state — never a parameter, and nothing durable reads it.
- **On a card surface, `panel` calls `navigation::paged` before the cards are drawn**, after
  applying any pending developer-view request. Its one argument is the same *inert* condition the
  paging renderer's `hold` gets — **name it once and pass it to both**, because it is one question:
  does another surface own this frame's keyboard? `PresetUi::holds_the_keyboard` answers the preset
  half, and an open text entry is the editor's own. `paged` derives the plan's flattened
  category-first order — **never raw authored order** — asks the renderer for a card the cursor
  reached on another page, and sets the outline. A `Shift`/`Command` arrow must be consumed before a
  control sees it, or one press is spent twice. It also publishes the selected parameter as the
  keyboard authority, because a custom-painted control may lose native egui focus while the visible
  cursor still names it. On every cardless surface, call `navigation::stop` so an invisible previous
  cursor cannot retain keyboard ownership.
- **The binding wraps each control in `navigation::at(ui, id, …)`**, fills `ParamView::stepping`
  from `ErasedParam::stepping`, and hands `ParamView::stepping_by` itself: `Bound` implements
  `control::NextValue` over its `law`, so each press lands where `ErasedParam::step_from` says.
  **A pitch, a filter corner in hertz or a cents tune declares its law where its `Bound` is built**
  (`.law(StepLaw::…)`, or a `step_law(id)` table beside `binding_for`) — at every construction
  site, since a panel may draw a parameter with its own `Bound::new` beside the `binding_for` the
  Parameters surface uses; `mxm-mono-01` does. The owner's scope (2026-09-23): semitone-stepped
  parameters (coarse tunes, transposes, Shift, every Bend range) take `Semitones`, pitch and filter
  corners in hertz `Hertz`, the ±100/±200-cent tunes `Cents`; continuous semitone tunes, route
  amounts, rates and %-valued cutoffs keep `Own` — except `mxm-mono-08`'s pitch routes, which read in octaves and step by `StepLaw::Interval` through `mxm_modulation_params::ui::stack_with_law` (owner, same day, for that instrument only), and its five step levels, which step by `StepLaw::Voltage`: to the next semitone coarse, so a one-octave pitch route plays semitones, and by 1 % fine. **Every law has the `Alt` layer** (`mxm_preset::StepLaw`, 2026-09-24): `Own` 1 % and 0.1 %; the pitch laws ten cents and a cent; `Cents` a cent and a tenth; `Voltage` ten cents and a tenth of its fine step. A plugin declares nothing more for it.
  **A law may follow another parameter's value**, because `Bound` is built every frame:
  `mxm-mono-08`'s modulation frequency is a rate in its low range and a pitch in its high one, and
  takes `Hertz` only while the high range is selected.
  This is every control that edits a parameter, wherever it is drawn:
  the knobs and sliders `mxm_preset::binding` draws, and the switches, selectors and source menus a `sections`
  module draws through `mxm_ui::control` directly. Without the scope a control paints identically
  and simply does not join the registry — silently unreachable, which is why every editor carries
  the reachability check below.
  An editor-only control drawn *inside* a parameter's scope — a picker that chooses which parameter
  the control beside it edits — goes in `navigation::aside`, or it registers as a second cell of
  that parameter and answers the same bare arrow.

**Do not build a parameter-to-card table for this.** The registry is assembled from what the frame
paints, so it is right about disclosures, reflow and hidden cards by construction; a declared table
would be wrong the first time any of the three changed.

**Every editor proves its own coverage**, with one call to
`mxm_plugin_test::keyboard_checks::the_cursor_reaches_and_operates`, the shared check beside
`mxm_plugin_test::paging_checks`. It paints the real panel: first that
the cursor lands, that a bare arrow edits it as one balanced host gesture (which is also what proves
the step law arrived), and that `Shift`+arrow selects instead; then that requesting each card in
turn, with the editor's `REVEAL` opening whatever it keeps behind a disclosure, registers
`Coverage::Exactly` the editor's inventory — or `Coverage::Within` it, where some parameters are
reached only through a card's own picker (mxm-mono-08's routing amounts). **A missed `at` scope is
invisible to every other test**, which is the whole reason this one exists. An editor whose opening
control edits something other than a host parameter calls `the_cursor_reaches_and_operates_from`
with its first parameter, which a pointer press lands the cursor on before the arrow
(mxm-fx-convolution, whose opening control is the response's Interpretation). An editor with no
paging renderer passes no items, and one pass sees every card (mxm-fx-curve, whose cards are all
bar cards).

**`paged` only knows the cards the paging renderer reports.** A parameter drawn *outside* the
renderer — every instrument's app-bar master output — is painted and clickable and registers
nothing unless it opens a card scope of its own, because a spot needs a card scope and a parameter
scope both open, and the cursor's card order and geometry come from that report. So the editor
draws it inside `navigation::bar_card` under a key outside the page keys and drives the cursor with
`navigation::paged_with_bar`, which adds last frame's bar rectangle; see `crates/ui/AGENTS.md`.
**The coverage check is what found this**, on the sampler after it had shipped — so an editor with
any surface above the cards owes the check before it claims the cursor runs.

## A knob shows the parameter; modulation is drawn over it

`ErasedParam` reports the parameter's **own** value and, separately, how far a host is modulating it.
Neither is derived from the other and they are never added together for display.

- **The knob draws the unmodulated value**, and so does the number beside it. A control drawn at
  `value + modulation` jumps under your hand while the thing you are setting sits still — and the two
  are one control, so a position and a number that disagreed would be worse than either alone.
- **`ParamView::marked` is set when the two differ**, and `ParamView::modulation` carries how far —
  the knob draws an arc from its own position to where the modulation takes it. The two shapes carry
  the two facts: the marker is what the knob is set to, the arc is what is being played. A knob that
  stays put while the sound moves is the *right* behaviour, and without the arc it reads as the
  control refusing input. In the MXM player that something is the step sequencer and the
  mark means "this step deviates from the patch", but **the editor does not know that and must not
  need to** — it works in any host that modulates, and no protocol between host and plugin exists or
  is wanted.
- **A drag draws its own delta, live.** Host modulation does not exist until the sequencer has
  applied an offset, so a line drawn only from it appears one bar too late — authoring was blind.
  While a gesture is open the editor draws the distance back to where the drag began, remembered in
  egui's own memory keyed by the parameter; in the common case the origin is the patch, so the line
  is the deviation being authored as it is made. Host modulation and an open drag never overlap,
  because a dragged parameter is not modulated.
- **The mark is painted, never laid out.** A knob's column is a fixed grid, name and value included;
  a dot in the label flow would shift the name and make a marked knob a different shape from an
  unmarked one.

nice-plug applies modulation as `(unmodulated + offset).clamp(0, 1)` and a later offset **replaces**
the previous one. The clamp is why an offset is a deviation and not a value: a host sending an
absolute value here would saturate anything already near the top of its range.

## Licensing

Each plugin folder carries its own `LICENSE`. MIT where all code is original. Decide before copying
from a GPL project, not after.

# Work Guidance

## Adding a new plugin

The ordered walkthrough — with the traps that belong to no single scope, and a copy-paste checklist
— is [`docs/adding-an-instrument.md`](https://github.com/mxm-audio/newdawn-workspace/blob/main/docs/adding-an-instrument.md). It owns no rules; the
contracts stay here. The short form:

1. Write the §14 design brief in `docs/briefs/<plugin>.md` first.
2. Create `plugins/<plugin>/` with a `Cargo.toml` joining the workspace, plus its own `LICENSE`.
3. Pick a permanent `CLAP_ID` (`dk.mxm.<plugin>`). It cannot collide — the namespace is a domain the
   project owns — so the only thing to get right is that it is **permanent**.
4. DSP first, in `crates/<plugin>-dsp`, with unit tests — silence in gives silence out, no NaN/inf
   under extreme params, filters bounded under a stated numeric limit.
5. Params, then MIDI, then presets — `impl mxm_preset::Instrument` and a factory set, nothing
   copied — then the editor.
6. Write `control-map.json`, mapping the collection's roles to this plugin's parameter ids. Leave
   out every role the instrument does not have.
7. `cargo xtask bundle`, `clap-validator validate`, then a real host before declaring victory.
8. `README.md` in the plugin folder, linked from the root `README.md`.
9. Add both crates to the workspace `members` **and to `default-members`** — the root's bare
   `cargo build` gate covers `default-members` only, so a crate in `members` alone is never built by
   it. Set each MSRV explicitly and record both in the root MSRV table.

## Shared here, local below

A fact about one machine belongs in `plugins/<plugin>/AGENTS.md`; a rule for every plugin belongs
here. Local deviations from a shared contract must remain local so later products do not inherit
them as policy.

# Verification

```bash
cargo test -p mxm-mono-01
cargo clippy -p mxm-mono-01 --all-targets

cargo xtask bundle mxm-mono-01 --release          # -> target/bundled/mxm-mono-01.clap
clap-validator validate "target/bundled/mxm-mono-01.clap"
```

**A change that only moves or sizes things on screen is verified with the layout checks, not the
full suites** (the owner, 2026-09-25: *"it is ridiculous that a resizing of a knob takes 30 minutes
of testing? Why can't you calculate it directly? That was the whole point of the new flow design"*).
The layout tree computes every size, so work the geometry out from it and pin it in a `tree` test in
`crates/ui`. Then run only the affected plugins' layout tests, e.g.
`cargo test -p <p> … --lib -- opening_size tree_checks fits minimum reflow`, and look at
`MXM_PICTURES=after cargo test -p <p> --lib tree_pictures -- --ignored`. Audio, preset and state
suites do not run for it. Anything that touches parameters, `process()` or state is not layout-only.

Then, in order, before calling a change done:

1. A **debug** build run — `assert_process_allocs` only fires in debug.
2. `cargo run -p mxm-player --release` and actually play it. **"Show editor" opens the plugin's own
   interface in a floating window**, so an editor change is visible here now — and this is the only
   place the *hosting* path is exercised: `create`, ownership, `destroy`, and reopening afterwards.
3. `cargo run -p mxm-mono-01-standalone` and actually use the editor. The editor with no host at all,
   which is where to work on the editor itself rather than on hosting.
4. A real DAW at a small buffer size, with automation on the modulated parameters. **Currently a
   recorded-unmet gate**, not a step to perform: the standalone wrapper owns its own window, while
   under CLAP the editor is handed a parent and is resized and scaled by the host. Parenting,
   host-driven resize, scale changes and open/close ordering are unverified until someone runs it
   in a host.

`clap-validator` is **not a cargo dependency and not on `PATH` by default** — install it once:

```bash
cargo install --git https://github.com/free-audio/clap-validator.git --locked
```

Nothing in the repository installs it, and its absence is silent: the command simply is not found,
and a verification block that "passed" without it has skipped the gate that matters most.

## Reading a validator crash

A test that reports `crashed: exit code: 0xc0000409` and two failed allocations of a few dozen
bytes is **a panic, not memory pressure**. `0xc0000409` is Rust's `abort`; a panic cannot unwind
across the CLAP boundary, so it aborts, and under a stripped release build the panic text is lost
— what surfaces is the panic machinery's own allocation failing. Chasing the byte counts leads
nowhere.

To recover the message, build the bundle with the release profile's `strip = "symbols"`
temporarily set to `"none"` and add `debug = 1`, then re-run with `RUST_BACKTRACE=1`. The panic
line and its location appear, and `--in-process -t "<test name>"` narrows the run to one test.

**Which fuzz test crashed tells you something on its own.** `param-fuzz-bounds`, `-modulation`
and `-sample-accurate` snap each parameter to one end of its range; only `param-fuzz-basic` draws
the interior. An enum's *middle* variant has no end to snap to, so anything reachable only
through one — `Reader::Stretch`, `LoopMode::Forward` in `mxm-creative-sampler` — is exercised by
`param-fuzz-basic` alone. A crash there and nowhere else points at a middle variant before you
read a line of code.

**Building the crate does not build the bundle, and nothing says so.** `cargo build -p mxm-mono-01`,
`cargo test`, `cargo clippy` and `apps/mxm-mono-01-standalone` all compile the library and all show the
change immediately. `target/bundled/mxm-mono-01.clap` is a **separate artifact**, and a host loads that
one. Change the editor, run the standalone, see the change, screenshot it — and the plugin a person
actually opens is still the previous build.

**The standalone is what makes this trap convincing**, because it is the easy thing to drive and it
is honest about the code it was built from. It is just not the thing being shipped. So: after any
editor or parameter change, `cargo xtask bundle mxm-mono-01 --release` **before** looking at the plugin,
and check the timestamp on the `.clap` if what you see disagrees with what you wrote. Reported once
as *"it didn't stick"*, with a screenshot of the old panel taken minutes after a screenshot of the
new one.

**Nothing may be holding the bundle when you rebundle.** `cargo xtask bundle` fails with
`Access is denied (os error 5)` — *"Could not remove file before reflinking"* — if a player still has
the `.clap` loaded, and the failure is easy to read past. **The dangerous part is what happens next:
the previous bundle is still sitting there, so the validator runs and passes against the build you
meant to replace.** Check that bundling printed `Created a CLAP bundle` before believing a validator
result, and close the player first.

**A clean validator run has zero failures in debug and release.** Instruments and effects exercise
different subsets, so their passed counts need not match. If any test fails, first confirm that the
`[patch.crates-io]` redirect to [`vendor/nice-plug`](https://github.com/mxm-audio/newdawn-workspace/blob/main/vendor/AGENTS.md) is still active; its
regressions protect fixes that remain unfixed upstream. Wrapper defects 6 and 7 — the first sample-accurate parameter event's timestamp, and an out-of-range timestamp used as an audio split point — have their bundled-host regressions in `plugins/mxm-para-07/host-tests/tests/behaviour.rs`. Wrapper defects 8 and 9 — GUI state restoration with complete rollback, and a rejected persistent field — have theirs in `plugins/mxm-fx-convolution/host-tests/tests/behaviour.rs`.


The debug run is the one that matters: `assert_process_allocs` only fires in debug, so a release
run proves strictly less. Allocation defects are invisible in release.

Wrapper regression tests live in `apps/mxm-player/tests/plugin_robustness.rs`,
`plugins/mxm-para-07/host-tests/tests/behaviour.rs` and
`plugins/mxm-fx-convolution/host-tests/tests/behaviour.rs`; all three run by default.
`mxm-mono-01`'s own `upstream_defects` unit test pins the arithmetic behind one of them, so the
recorded diagnosis cannot silently go stale.

## Installing a bundle in a DAW

Install locations for manual DAW testing:

- Windows: `%COMMONPROGRAMFILES%\CLAP` or `%LOCALAPPDATA%\Programs\Common\CLAP`
- macOS: `~/Library/Audio/Plug-Ins/CLAP` · Linux: `~/.clap`

Symlink rather than copy where you can, so a rebundle needs no second step. **On the Windows
development machine you cannot.** A symlink needs an elevated session or Developer Mode, and this
box has neither — `HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock\`
`AllowDevelopmentWithoutDevLicense` is `0`. `New-Item -ItemType SymbolicLink` fails with
*"Administrator privilege required for this operation"* **even for a link inside `%TEMP%`**, so the
refusal is about the privilege and not the destination. Do not go hunting for a writable directory
that makes it work.

**A hard link is not the workaround.** `New-Item -ItemType HardLink` does succeed unelevated on the
same volume, but `nice_plug_xtask` *removes and recreates* the `.clap` on every bundle — the same
delete that yields `Access is denied (os error 5)` when a player still holds it. The link then
keeps pointing at the old content and nothing says so, which is the "it didn't stick" trap above
made permanent.

**A per-user path is only an install location once the host has been told about it.**
`%LOCALAPPDATA%\Programs\Common\CLAP` is the standard unelevated CLAP directory, but a host scans
the locations it is *configured* with, not the ones the specification lists. Bitwig 6.0 here ships
exactly one CLAP entry — `C:\Program Files\Common Files\CLAP` — and the per-user path appears
nowhere in `%LOCALAPPDATA%\Bitwig Studio\prefs\*.prefs`. Read the host's own location list before
believing an install landed somewhere it will be found: a plugin copied into an unscanned directory
is indistinguishable from a plugin that never built.

So the working Windows procedure is an elevated copy into the system directory — one UAC click:

```powershell
# The source must be ABSOLUTE: the elevated shell does not inherit this working directory.
$src = "$PWD\target\bundled\mxm-shimmer.clap"
Start-Process powershell -Verb RunAs -Wait -ArgumentList "-NoProfile","-Command",
  "Copy-Item '$src' 'C:\Program Files\Common Files\CLAP\' -Force"
```

**Because that is a copy, the stale-bundle trap is back and avoiding it is now manual: re-run the
copy after every `cargo xtask bundle`.** Rebundling without re-copying leaves the DAW loading the
previous plugin while every local check — standalone, tests, validator — honestly reports the new
one. If the UAC prompt is the part that grates, add `%LOCALAPPDATA%\Programs\Common\CLAP` to the
host's plugin locations once and install there unelevated from then on.

Installing is not the Bitwig gate. It only puts the artifact where the host can find it; the
per-plugin DAW checks recorded in each child doc still have to be run and reported.

# Child DOX Index

| Doc | Scope |
|---|---|
| [`mxm-mono-01/AGENTS.md`](https://github.com/mxm-audio/mxm-mono-01/blob/main/plugins/mxm-mono-01/AGENTS.md) | Identity compatibility, routing, presets and editor |
| [`mxm-mono-03/AGENTS.md`](https://github.com/mxm-audio/mxm-mono-03/blob/main/plugins/mxm-mono-03/AGENTS.md) | Identity, routing and the Env Mod floor, slide, presets and editor |
| [`mxm-poly-06/AGENTS.md`](https://github.com/mxm-audio/mxm-poly-06/blob/main/plugins/mxm-poly-06/AGENTS.md) | Identity, built-in chorus, idle deviation, modulation and editor |
| [`mxm-mono-00/AGENTS.md`](https://github.com/mxm-audio/mxm-mono-00/blob/main/plugins/mxm-mono-00/AGENTS.md) | Identity, the patch bay's 433 offered routing pairs and the ids that retired for and with them, activity, tempo sync, presets and editor |
| [`mxm-mono-02/AGENTS.md`](https://github.com/mxm-audio/mxm-mono-02/blob/main/plugins/mxm-mono-02/AGENTS.md) | Identity, routing and its init patch, the retired external input, performance ownership, activity and editor |
| [`mxm-mono-08/AGENTS.md`](https://github.com/mxm-audio/mxm-mono-08/blob/main/plugins/mxm-mono-08/AGENTS.md) | Identity, Once, layouts, performance ownership, presets and editor |
| [`mxm-mono-pr1/AGENTS.md`](https://github.com/mxm-audio/mxm-mono-pr1/blob/main/plugins/mxm-mono-pr1/AGENTS.md) | Identity, layouts, the retired external inputs, event handling, activity, presets and editor |
| [`mxm-para-07/AGENTS.md`](https://github.com/mxm-audio/mxm-para-07/blob/main/plugins/mxm-para-07/AGENTS.md) | Identity, the retired external input, retained-owner MIDI, activity, fifty presets, map, telemetry and editor |
| [`mxm-chorus-06/AGENTS.md`](https://github.com/mxm-audio/mxm-chorus-06/blob/main/plugins/mxm-chorus-06/AGENTS.md) | Identity, circuit equivalence, Off/activity, layouts and presets |
| [`mxm-folded-spring/AGENTS.md`](https://github.com/mxm-audio/mxm-folded-spring/blob/main/plugins/mxm-folded-spring/AGENTS.md) | Identity, tank/feedback departures, transitions, layouts and map |
| [`mxm-bucket-delay/AGENTS.md`](https://github.com/mxm-audio/mxm-bucket-delay/blob/main/plugins/mxm-bucket-delay/AGENTS.md) | Identity, twenty-one controls, activity, layouts, presets and editor |
| [`mxm-shimmer/AGENTS.md`](https://github.com/mxm-audio/mxm-shimmer/blob/main/plugins/mxm-shimmer/AGENTS.md) | Identity, feedback/Freeze/activity, presets, map and editor |
| [`mxm-classic-verb/AGENTS.md`](https://github.com/mxm-audio/mxm-classic-verb/blob/main/plugins/mxm-classic-verb/AGENTS.md) | Identity, parameter kinds, layouts, activity, provisional presets, editor, and in-plugin loading: decode, fit generations, the staged commit and the loaded-space payload |
| [`mxm-grain-fx/AGENTS.md`](https://github.com/mxm-audio/mxm-grain-fx/blob/main/plugins/mxm-grain-fx/AGENTS.md) | Provisional identity, granular laws, activity and editor |
| [`mxm-fx-convolution/AGENTS.md`](mxm-fx-convolution/AGENTS.md) | Identity, embedded response state, the collection's impulses folder, presets, activity and editor |
| [`mxm-fx-curve/AGENTS.md`](https://github.com/mxm-audio/mxm-fx-curve/blob/main/plugins/mxm-fx-curve/AGENTS.md) | Identity, schema, Input gain / Auto makeup / Mix parameters, sixty durable-state factory presets, stretch-to-window canvas with post-gain In and post-Mix Out meter rails, bounded undo, host dirty-state, fixed-slot publication and transitions |
| [`mxm-fx-delay/AGENTS.md`](https://github.com/mxm-audio/mxm-fx-delay/blob/main/plugins/mxm-fx-delay/AGENTS.md) | Identity, synced Time, factory Mix preservation, layouts, activity, presets and five-card editor |
| [`mxm-creative-sampler/AGENTS.md`](https://github.com/mxm-audio/mxm-creative-sampler/blob/main/plugins/mxm-creative-sampler/AGENTS.md) | Identity, native acquisition, embedded assets, transactions, events and editor |
