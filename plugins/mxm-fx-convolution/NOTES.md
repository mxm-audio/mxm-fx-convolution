# NOTES.md — plugins/mxm-fx-convolution/

The detail behind this folder's AGENTS.md: history, measurements, rationale and worked examples.
AGENTS.md is the contract; this file is the reference it links to.

## Files

- `src/lib.rs` — CLAP export, two layouts, activation, bounded processing, activity and state
  preflight.
- `src/params.rs` — the two legacy ids, five neutral wet-post ids, neutral Feedback and the
  generated-response default patch.
- `src/response.rs` — path-independent canonical response state, project-authored starter response,
  WAV decoding, reversible Size/Decay/Damping modelling, source-preserving overlay merge,
  band-limited resampling, rate-dependent deadline validation, latest-request staging of complete
  prepared engines, bounded display cache and preset capture.
- `src/telemetry.rs` — lock-free peak, tail, clip and numeric-fault telemetry.
- `src/deferred.rs` — the production deferred-preset owner: off-thread complete/overlay preparation,
  latest-request cancellation, state-boundary publication, lock-free callback acknowledgement and
  post-ack identity installation across editor reconstruction.
- `src/impulses.rs` — where the collection's impulse responses are installed, and the folder Browse
  opens in.
- `src/editor.rs` and `src/editor/` — shared app/preset shell, Effects paging, keyboard parameter
  cursor, semantic response/output status, nine live bindings, release-coalesced preparation edits
  and non-destructive telemetry views.
- `src/preset.rs` — the shared synchronous complete-payload seam, opt-in deferred model-overlay/Init
  seam and generated Init. No factory set is compiled in: `src/impulses.rs` scans the collection's
  impulses folder for it.
- `control-map.json` — the existing `fx.reverb` role assigned to Mix; no invented page or role.
- `README.md` and `Cargo.toml` — product description, crate boundary and pinned dependencies. (The
  original list also named a `LICENSE` with MIT terms; there is none in this folder, and the licence
  is the workspace's GPL-3.0-or-later.)

## Status

Native WAV acquisition, reversible preparation controls and portable user-preset response capture
are delivered without promoting model edits into permanent host parameters. Revision 7 adds the
ordinary live Feedback knob without changing the existing four cards or the response display, and
renders a successful budget clamp as neutral information with committed-value readback. Broad native
visual/DAW review and listening remain later factory checkpoints.

## Identity and live controls

### Normalisation, Mix and the live ids

**Every response is normalised, and Mix is the one level** (the owner, 2026-09-28: *why not do the
level with the mix, and normalise the impulses?*). `response::loudness_scale` gives the whole source
a mean energy of one per channel at the processing rate, in `prepare()` straight after decoding, so
every path — the starter, a loaded file, a preset, a restore — gets it, and no stored state or
fingerprint changes. Energy rather than peak, because a response's peak is its direct click;
normalised at Size 100 % against the resampler's DC-keeping law, so the same room at 48 and 96 kHz
lands at one level and Size, Onset, Extent, Decay and Damping keep the effect on level they had. At
Mix 50 % the reverb of broadband sound is as loud as the sound. Below `SILENT_ENERGY` a response is
left alone. `every_response_is_prepared_at_one_loudness` holds it.

**Wet gain is deleted, id and all** (the owner: *there are no old projects — just delete the id*):
the DSP's wet gain went with it, and a restored identity drops a baseline that still names it. The
shipped legacy ids are `mix` and `predelay`. Mix is a linear signal quantity; Pre-delay moves a DSP
tap and is not given a second parameter smoother. New automatable ids are `lowcut`, `highcut`,
`tone`, `width`, `modulation` and `feedback`; Low cut, Tone, Width, Modulation and Feedback default
to exact DSP-neutral values (0 Hz, 0 dB, 100%, 0%, and zero return). High cut is deliberately
different after the owner's correction: it is a real 0–20 kHz low-pass, fully closed at the bottom,
with its real maximum displayed as Open at the top. Feedback's pre-listening response-relative
ceiling is 1.25 and values above the contraction mark are labelled self-oscillation-capable rather
than Unity. Size, Decay and Damping remain non-automatable response-model state and have no host ids.

### Pre-delay sync

**Revision 8 adds `predelaysync`** (2026-09-25, `plans/plan-tempo-sync-controls.md`), the
collection's one tempo sync: the quarter note beside Pre-delay, on `params::PRE_DELAY_SYNC` (1/64 to
a quarter note, the top the longest), resolved once a call and handed to the DSP's own tap crossfade
in place of the knob. It defaults off. `filter_state` inserts it off into every older state, so an
old project never inherits an instance's sync, and `migrate_legacy_identity` extends an exact
Revision 7 baseline with it at zero, so a loaded preset stays clean. The legacy resolver supplies it
off for older preset files, as it does Feedback. `Telemetry::tempo` lets the knob read its division.

### Feedback

Feedback zero is an exact structural bypass. Raising it re-convolves the raw wet signal through the
same response, lengthening, smearing and usually darkening the tail rather than holding the same
room longer. Response preparation analyses the complete damped loop's Nyquist crossing, including
return routing and its explicit sample delay, so displayed 1.00 is the linear self-oscillation edge
and 1.25 is past it for a qualifying response. A separate magnitude bound—not the edge—governs
conservative finite-tail certification. The one soft saturator is in that return only and is a
character bound, not a safety limiter; dry, feed-forward wet, Wet EQ, Modulation, Width and Mix
remain linear. Freeze stays absent, no threshold is printed, and self-oscillation is not promised
for every response.

### Pre-listening ranges and the High cut correction

The wet-post and Feedback non-neutral ranges and the Decay/Damping laws are pre-listening
engineering values awaiting owner approval, not values selected by listening. Their neutral states
and meanings are fixed for compatibility; a later verdict may tune a non-neutral range without
changing an id. **Owner correction, 2026-09-17:** High cut's original Open-at-bottom mapping was a
control defect, not a compatibility contract or a sentinel to move elsewhere. Zero is now a real
closed cutoff and 20 kHz is the real maximum displayed as Open. Existing plain-zero host state
migrates to that top and its loaded-preset baseline follows from normalized zero to one.

## Presets

### Init

Init values remain generated from CLAP defaults. In the shipped app bar, Init is an explicit
source-preserving deferred model overlay: Pending emits no gestures or identity change, Ready emits
eligible default gestures and commits one prebuilt response while processing is held at the wrapper
boundary, and identity clears only after the following callback acknowledges publication. The
shared synchronous Init entry point remains unchanged for every non-opt-in consumer.

### The factory set

**The factory set is the impulses folder** (below). The fifty parameter-only recipes it replaced —
ten Pre-delay positions crossed with five balances — were deleted on 2026-09-28. The default Mix is
30 %, the old 38 % at −3 dB folded into one Mix when Wet gain went. The legacy resolver still
supplies Feedback zero and Pre-delay sync off for older external preset files.

### The deferred preset transaction

User presets capture the complete response source and reversible preparation through the shared
durable-state seam. Shipped preset selection uses the opt-in `DeferredPresetTransaction` for
complete content or an explicit `MergeWithCommittedSource` model overlay: rejected, cancelled and
superseded preparation emits nothing; Ready writes only parameter bases whose begin-time
unmodulated bits and optional base revision are both unchanged, and identity follows callback
acknowledgment. Requiring both closes the interval between those separate begin-time reads. Each
process-block boundary, editor frame and transaction begin/publication samples every live
parameter's unmodulated normalized bits and advances its own monotonic base revision only when
those bits moved. Host modulation therefore cannot hide a preset base target; an away-and-back base
edit still wins when either leg crosses a sample point, even when its final bits equal the observed
value. An away-and-back edit completed wholly between two sample points is the explicit
observability bound. The transaction retains the preset's canonical target baseline separately and
captures its response fingerprint at publication, so edits during preparation or between
publication and acknowledgement remain Modified rather than being recaptured as Clean.

Ordinary WAV loads and response-model edits begin through the controller's same cancellation
domain; each cancels unpublished preset work immediately, and a worker superseded outside that API
retires its matching transaction on `Ok(false)` rather than remaining Pending. Once a preset
response has published, an ordinary successor preserves its callback acknowledgement: the preset
baseline is installed first and the successor response reports that identity Modified. A newer
deferred preset may prepare but cannot publish while its predecessor still owns the callback slot;
the predecessor's response request and retained identity wait independently, so cancelling or
rejecting the successor cannot orphan the already-committed response and parameters. Persistent
response restoration additionally advances a restore-only generation and clears staged work before
installing host state, so an older Pending, Ready or published-but-unacknowledged editor transaction
cannot later overwrite the restored parameters, response or identity. Pending suppresses Save/Save
As and an already-open Save As or Rename commit. A preset with `state = null` keeps its
preserve-only meaning: the app-bar adapter turns it into an identity overlay of the committed
response model so selection still follows the same bounded transaction. A room from the impulses
folder carries `{"impulse": <path>}` instead, read on the same background worker (below).

## Durable response state

### State migration

Two-channel width never chooses meaning. Validation rejects wrong schema/rate/count/length,
malformed base64 and non-finite coefficients. The state filter canonicalizes valid older response
payloads after serde supplies new neutral model defaults, before the wrapper's atomic durable-field
comparison. For an exact clean Revision 5 identity — `loaded = Some`, exactly the three legacy
baseline ids and a fingerprint matching that state's old response — it also adds the five neutral
normalized baselines and replaces the fingerprint with the canonical response fingerprint, so the
same named preset remains Clean, then adds Feedback's neutral baseline to exact clean R5/R6
identities. Missing Feedback in older host state is explicitly zero even when loaded over a
recursive patch. A mismatched fingerprint is an already-modified response and is deliberately left
Modified rather than laundered. A malformed field makes the complete incoming state a no-op, so
parameters cannot apply around an old response. Revision 6 host state that stores High cut's old
plain-zero Open value in empty/0.1.0 host state migrates to the real 20 kHz maximum before parameter
restore; matching loaded-preset identities migrate their normalized baseline from the old linear law
to the corrected perceptual law. Non-neutral plain frequencies retain their hertz value. Version
0.1.1 state does not take this migration, so its real closed zero round-trips.

### The ten-second source

**Owner ruling, 2026-09-14:** canonical source duration is 10.0 seconds. WAVs at or below the
boundary are embedded unfaded and bit-for-bit unchanged; longer WAVs are accepted, bounded to the
first 10.0 seconds and given a deterministic 50 ms half-cosine fade that reaches exact zero on the
last retained sample. At stereo 48 kHz the base64 state is about 5 MiB. Canonical source and wet
processing rates remain 8–384 kHz. Every finite positive host rate may activate: outside that wet
range, activation keeps the source untouched, makes the response inert, passes exact finite dry
audio and publishes the rejected rate and reason for the editor. A later supported activation
prepares the canonical response again.

### The preparation budget

The two-tier engine guarantees a full 10.0 prepared seconds through 96 kHz. Duration conversion and
this ceiling share one checked duration-first rounding law, including fractional processing rates.
Above 96 kHz only the measured 4,096-sample early-tier budget is accepted; the late tier cannot meet
the maximum-rate callback deadline on the measured machine. Rate × source extent × linked-time
combinations outside the applicable budget run one off-audio normalizer using preparation's exact
checked sample-count law. A direct Size or Extent edit chooses the greatest fitting representable
value; an open-ended Onset edit chooses the earliest fitting onset. Whole model/state/preset ingress
tries Size, then Extent, then conditional Onset. The normalized model is committed, displayed and
serialized immediately during restore, before any activation; publication leaves a neutral
informational status. Above 96 kHz the fixed 4,096-sample architecture envelope is not clamped: an
over-budget request remains a visible failure-atomic refusal. Canonical coefficients are never
truncated after construction. Both steady processing and phase-aligned 20 ms old/new transitions
are measured at every envelope corner.

### Activation and restore

Activation uses canonical source rate plus reversible onset/extent, Reverse, linked 25–400% Size,
the unchanged Natural/Fade/Swell/Gate law, then Decay and time-varying Damping before Size
resampling. Neutral Decay/Damping take exact legacy branches. Every operation derives from the
immutable canonical source, remains within selected support and never accumulates edits.
Windowed-sinc conversion preserves duration while rejecting energy above the target Nyquist, then
complete spectra and runtime history are prepared before the wet response becomes active. A
response over budget leaves host activation successful with the same visible inert-wet fallback
used outside 8–384 kHz. The patched nice-plug wrapper performs GUI restore off audio, holds its
plugin lock through deserialize/reactivate and restores its pre-mutation snapshot if either
durable-field preparation or activation rejects; rollback reactivation does not call `reset()`,
because rejection must preserve response, delay and tail history as well as durable bytes. Its
persistent-field guard admits a changed serialized value only when that exact field explicitly
reports successful canonical publication inside the current restore; an unmarked mismatch remains a
complete rejection. The inactive real-host clamp proof loads and re-saves without activation and
requires the applied maximum immediately. The active CLAP state proof changes parameters alongside
a deadline-invalid response in one of two matched processing instances, requires byte-identical
rollback, and then requires its nonzero live tail to remain bit-identical to the untouched control.
A changed response at the same rate/layout commits as the new primary and retains the old engine
for a chosen 20 ms convex transition; the one wrapper-issued reset preserves that transition. A
newer synchronous restore supersedes an in-flight transition by installing its complete engine
directly on the control thread rather than waiting for a callback.

### The starter response

The 7 ms stereo starter response is deterministic project-authored code and remains valid at the
384 kHz deadline corner. Its reflection/tail constants, and the Wet EQ/Modulation constants, are
pre-listening values awaiting owner approval, not values selected by listening, measured room facts,
copied response data or a room from the impulses folder.

### Acquisition

Browse and native drop accept mono/stereo WAVs; acquisition canonicalizes only the owner-ruled
over-10-second boundary above. Canonical source samples remain immutable and path-free; model
operations are metadata applied afresh. Background requests are monotonic latest-wins, expensive
slider work submits once at release, and each fully prepared candidate carries its canonical state,
complete engine and bounded waveform display into the wrapper transaction. Decode/allocation/deadline
failure keeps the old response active and displays the error.

## The collection's impulse responses

- **Installed per user, outside the plugin** (owner, 2026-09-16): `mxm/impulses/` under the
  platform's local data directory, holding `crates/mxm-room-ir`'s release in its own layout
  (`<family>/<slug>/` WAVs, `metadata/` sidecars, `manifest.json`; the crate is in mxm-tools). It
  arrives as a versioned download beside the plugin, never inside it, and a VST3 build reads the
  same folder.
- **Browse opens there when it is installed.** The editor resolves the folder once
  (`impulses::root`) and injects it into the Response card. Whether it exists is checked on the
  dialog's thread, and a missing folder leaves the dialog where the platform opens it; nothing
  creates the folder. The pinned `rfd` passes the folder to every backend, but only Windows has been
  run, and a Linux desktop portal may ignore it.
- **Projects never depend on it.** A response loaded from it is embedded in state like any other, so
  a project opens on a machine without the folder.
- **What is in it is the factory set** (the owner, 2026-09-28: *every impulse file should be a
  reverb preset… what is in that folder is the default presets*). `impulses::scan` walks it each
  time the editor opens — every `.wav`/`.wave`, at most four levels and 2,000 files, hidden entries
  skipped — so a file added is a preset the next time, and no file is named in code. Each is Init
  with that room: named from the file up to its first dot in words (*Concrete stairwell*), filed
  under the first folder it sits in (*Halls*), a repeated name numbered with the first number free
  against every name so far, Init's included (`init.wav` is *Init 2*). They reach the preset library
  as `mxm_preset::Found`, which lists them as Factory and shows each folder as a category (mxm-kit's
  `crates/mxm-preset/AGENTS.md`). A preset carries only `{"impulse": <path relative to the
  folder>}`; `impulses::load` reads the file when it is chosen — on the deferred worker, whose folder
  `DeferredPresetController::with_impulses` injects, exactly as Browse reads one — so a hundred rooms
  cost an 8 ms walk to list, and a folder of anything else costs at most 20,000 entries read.
  Exporting a bank leaves found presets out (`mxm_preset::Entry::found`): they name a file, and a
  bank must load without the folder. A missing or unreadable file fails the load with the response
  playing kept, and a path that leaves the folder is refused. With no folder the list is Init alone.
  The catalogue's 100 rooms list as nine families. `the_folder_is_scanned_into_one_preset_per_wav`,
  `a_found_preset_loads_its_file_and_reads_clean` and
  `a_missing_impulse_fails_and_keeps_the_current_response` hold it.
- **A published preset wakes the audio side** (the owner, 2026-09-28: a room chosen in the MXM
  player left the bar at *No preset*). Publication is acknowledged only by the next process
  callback, and a host that sleeps an idle effect — the player does — may have spent the wake the
  parameter gestures asked for before publication was ready. `DeferredPresetController::service`
  reports that it published, and the editor then queues `EditorTask::Wake` on the GUI path, which
  vendored nice-plug turns into the host's `request_process` (its GUI-task process-wake patch), as
  mxm-creative-sampler and mxm-mono-08 do. This settles plan §7.2's note differently from its
  design: a path in the folder, not a catalogue identity with a hash, because what is in the folder
  is what the owner wants listed.

## Host proof

- `plugins/mxm-fx-convolution/host-tests/tests/behaviour.rs` names this product only at the test
  boundary. It covers ordinary discovery and the unchanged map; ten ids/defaults; complete current
  and legacy-three-id state with neutral Feedback migration; successful over-budget Size publication
  with exact maximum readback; the corrected `0x3511_36eb_64c2_fa04` Feedback-zero/awkward-partition
  digest after old bottom-Open High cut state migrates to real 20 kHz; malformed-state atomicity and
  active high-rate refusal rollback with an unchanged live tail; audible finite rendering, exact idle
  and bit-identical Player bypass. Direct Extent and open-ended Onset are model-editor operations
  with no CLAP control and remain pinned at the plugin seam.
- `plugins/mxm-fx-convolution/host-tests/tests/robustness.rs` explicitly selects both advertised
  layouts while deactivated, proves distinguishable dry channels and Mono-to-stereo wet folding with
  an empty output-event sink, drives 8–384 kHz and callback sizes crossing 64-sample internal spans,
  proves positive rates through 768 kHz still activate with finite exact-dry output when wet
  preparation is unavailable, proves in-callback boundaries for legacy Mix and wet-post Low cut
  automation, queries finite tail publication, and runs the same stress against a debug bundle where
  nice-plug aborts on a process allocation. Its project-authored one-tap response proves finite
  sub-unity recursive decay, Feedback automation into infinite `KeepAlive` with
  `clap_host_tail.changed` observed by the real Player host, and exact stop/empty wake through
  Feedback Off, Mix-zero park and reset. Its ignored owner-catalogue rig brackets sustained
  oscillation at displayed 0.98–1.02 on a concrete stairwell, small echo chamber and walk-in closet
  through a freshly staged release bundle; unlike the earlier measurement-only versions, a missed or
  unbracketed edge fails the test.
- The per-plugin control map uses the current player schema (`instruments` array). Its first content
  version incorrectly copied the newer single-product `clap_id`/`roles` shape from an unintegrated
  pre-gate effect; ordinary Player discovery rejected it, and the real-host test is the guard.

## Editor

### Cards and the layout tree

Four stable Effects cards follow the approved signal order: Response, Time, Shape, Output. Response
and Time are the sole preferred group; every card remains indivisible, and each is as wide as its
floor.

**Every card body is a `mxm_ui::tree`** (`editor/sections.rs`): `card` describes it from the
parameters, the response view, the telemetry snapshot and the editor's in-progress preparation
values; `paging::editor::show` measures it and `paint` draws each leaf — knobs through the binding,
and the response model's own values on the collection's controls (owner's ruling, 2026-09-24: the
plain egui widgets are replaced): the preparation values on `control::slider`
(`sections::release_slider`, a `ParamView` over the value's range, held locally while dragged and
submitted on release), Reverse on `control::toggle`, the interpretation on `control::segmented`, and
the tail shape as four pictures on `control::segmented_waves` (`Wave::Level`, `Fade`, `Rise`,
`Gated`: the envelope each law multiplies the response by). Each joins the keyboard cursor
(`sections::PREPARATION_IDS`); because the starter response is stereo, the opening control is the
Interpretation switch, so the keyboard check proves the arrow on Pre-delay
(`the_cursor_reaches_and_operates_from`). Browse and Acknowledge fault are the collection's button,
the app bar's own. **Floors are computed** every frame by `tree::card_floor`; there is no typed
floor and no declared usability minimum. Knob rows stand in the collection's knob column
(`control::knob_column`) one item spacing apart. The response plot states `sections::PLOT_MIN` with
`SPACE_2` of its own below it, the linear strip fills its card at `sections::STRIP_HEIGHT`, and a
preparation slider holds its widest reading (`Preparing::widest`). The interpretation switch exists
only for a stereo response. **Still text (E1):** the response status reserves every line it can say
about the response (a decoder's own message is open-ended and still re-wraps), the Output status its
three lines, the fault and its button their room while nothing is wrong (an empty space then), and
the Time card's closing reading its widest (`PREPARED_WIDEST`). **No help text on a card** (the
owner, 2026-09-27; design system §7.6): the *Pure linear response preparation* heading and its note,
the idle *Linear output* line and the Prepared reading's *remains immutable* are gone; what the
linear strip means is its hover text, written for the player. A drop lands on the plot's own
rectangle. `every_card_passes_the_tree_checks_in_every_state` runs `mxm_plugin_test::tree_checks`'s
checks over the starter response, a mono response preparing a file, a failed ten-second response
with Mix at Off, a tail and a fault, a host-rejected rate, and the panel without acquisition
callbacks.

### Opening size and scales

The opening frame is the quarter-4K budget hugged on the headless 1× harness (`REFERENCE`, held by
`the_opening_size_is_the_budget_hugged` and `the_opening_frame_is_inside_the_quarter_4k_budget`).
The minimum is one widest card plus gutters (`MINIMUM`, held by
`the_minimum_holds_the_widest_card`). Headless proof keeps the quarter-4K content size as a fixed
physical budget at 1×, 1.5× and 2× in both themes
(`every_control_and_semantic_view_is_accessible_in_both_themes_at_fixed_physical_scales`); native
DPI/chrome, visual and DAW checks remain manual rather than being implied by that geometry.

### App bar

The app bar uses the shared preset row's opt-in deferred adapter, peak/latched-clip meter, fixed
user zoom and remembered Light/Dark/System selector. Selection and Init dispatch `EditorTask`
preparation; pending disables Save/Save As but leaves newer selection live. Unpublished work is
cancelled on close; an already published request is acknowledged by `Plugin::process` and its
background task even if the editor has closed. The effect has no note port and therefore no
developer CC channel.

### Knobs, the binding and the model sliders

All ten host parameters use shared knobs, exact entry, tooltips, keyboard navigation and one
balanced gesture-binding path. **`editor/binding.rs` re-exports `mxm_preset::binding`**, the
collection's one binding, since 2026-09-24; it was a reduced copy (`new` and `knob` only) until
2026-09-23. The one behaviour that changed when it became the full one was the reset: a
double-click on a knob a host is modulating keeps its value, as in every other plugin, where the
reduced copy always returned to the default. Low cut and High cut step from the keyboard by
`StepLaw::Hertz` — a semitone and an octave — and 0 Hz, Low cut's Open, is left and reached by the
parameter's own step, because no ratio can. Feedback sits before the existing feed-forward Wet
EQ/Modulation controls because its return surrounds the FIR; its control prints no oscillation
threshold. Size, Decay and Damping remain model state: their sliders retain transient drag values
locally and submit only the released candidate to background preparation; they emit no host
gestures. The held values are cleared once the response is idle again — never on the frame a drag
is let go, when the slider is still submitting what it holds. Nothing is drawn to measure a card:
telemetry is copied once before the frame and only painting reads it, so no gesture, preparation
request or second peak read can come from sizing.

### Response states and the plot

Response metadata, explicit interpretation, committed/loading/information/error state, exact Off,
active tail and numeric fault are named in text as well as colour. Preparation caches a fixed
200-bin energy display; paint clones only that bounded view on revision changes and never decodes
or clones embedded payload. **The plot is energy, not a waveform** (owner, 2026-09-15: a linear
trace hid the energy, and a car cabin read like a cathedral): each 50 ms bin's peak and RMS level in
dB, 0 to −72 dB, on a fixed ten-second axis with second and 24 dB guides, so responses compare by
where their energy lies instead of each filling the plot. It is the committed source with its
operation region, not a synthetic decay. The resolved caption font height reserves the axis-label
row inside the canvas clip, and the prepared-region frame ends at the plot baseline above that row.
Response owns Browse/drop and interpretation; **Browse's dialog runs through `mxm_ui::offthread`**
and its pick is loaded on a later frame: a modal dialog opened inside the frame re-entered
egui-baseview and aborted the host (2026-09-15); Time owns onset/extent, Reverse and linked time;
Shape owns Natural/Fade/Swell/Gate, Damping, Feedback and the existing Wet EQ/Modulation controls
while retaining the pure-linear preparation statement. A Ready background completion is consumed
directly at build/update, so closing during Loading and reopening cannot strand it behind a matching
revision baseline. DSP reads no editor state.

### Editor proof

`src/editor/proof.rs` paints the real panel and proves private preset/fault state emits no audio
edit, the production app-bar path routes selection and Init through the deferred adapter while
pending Save is inert, every parameter joins the keyboard cursor, all nine live knobs and the sync
bracket one host gesture, the clamp notice is neutral while true refusal remains danger, the Size
control reads back the committed maximum, preparation drags submit once on release without host
gestures, repeated ten-second-stereo paints do not rebuild/decode the bounded display, semantic
views are accessible in both themes at fixed-physical 1×/1.5×/2×, and narrow/default/wide reflow
preserves floors, ceilings, sequence, non-overlap, aligned rows, the Response/Time group and the
no-lone-card-stretch rule.

The focused Player behavior test proves the release bundle advertises the floating effect editor;
its display-backed ignored case opens, closes and reopens that editor twice. This proves the
Player-owned floating lifecycle, not real-DAW parenting or native visual quality.

`clap-validator` 0.4.1 on 2026-09-14 passed the complete debug and release suites: 32 passed, zero
failed and 11 inapplicable cases skipped in each profile. Its denormal timing heuristic warned at
7.56× debug and 3.62× release; this is recorded warning evidence, not a clean-performance claim.

## The response status line

**The response's status line speaks to the player** (the owner, 2026-09-28: it read *Committed ·
embedded source and preparation are the active filter*): *Response loaded*, *Updating the
response*, *Loading the preset*, *Loading* a file, *Switching to* it, *Could not load*, and a
rejection that says the reverb is off and the dry sound passes through. While another response
loads, the line says the current one keeps playing. An edit's own line had never shown — every label
gained " model" on its way into the status — and `a_loading_response_is_named_in_words` holds that
it does now.

## What the focused tests cover

The focused tests cover identity/bundle agreement, the impulses folder's location and Browse's
installed-only start folder, layouts/no-note contract, the two legacy ids plus seven new live ids
and formatter round trips, generated nine-id Init, an empty compiled factory set with the impulses
folder scanned into presets and loaded from, High cut's real 0 Hz closed bottom / real 20 kHz Open
top mapping and old bottom-Open state migration, synchronous and production deferred durable-state
hooks, split value/revision capture protection, sampled unmodulated per-parameter edit revisions,
modulation/base separation, host-restore supersession, ordinary post-publication model/WAV
successors, published-predecessor identity across cancelled/rejected deferred successors, app-bar
selection/Init dispatch, process-callback acknowledgement, WAV stage/commit, exact-boundary unfaded
import, smooth over-boundary 10-second canonicalization, full ten-second playback, latest-wins
preparation, reversible models, spectral resampling, fractional-rate ceiling agreement,
accepted-limit transition coverage, rate/deadline rejection, engaged output, finite tail,
pre-delay-edit history, wet-post control wiring, exact dry Off in both layouts, hostile-input
recovery, telemetry latching, editor private state, accessibility, gestures and responsive geometry.
The starter's digest was recaptured on 2026-09-28 with the normalisation and Wet gain's deletion
(measured: its reverb rises 4.84 dB at 48 kHz), after the High cut correction had moved it from its
capture at `fda4f94`; the fifty recipe digests beside it went with the recipes; the old
three-id/schema-1 host-state fixture begins with a loaded clean identity and remains Clean under the
same name after neutral Decay/Damping, five neutral live baselines and the canonical response
fingerprint are migrated. The focused debug and release bundles, ordinary Player discovery/chain,
actual CLAP state extension, floating editor open/close/reopen, hostile rates/blocks and debug
process-allocation guard are proved by the two Player files above. Each profile passes the complete
validator suite before the mutable bundle slot is replaced.

## Past verification runs

### Pre-F018 source-to-artifact proof (2026-09-15)

The most recent pre-F018 source-to-artifact proof, repeated by the operator at commit `bbf3494`
after the F016/F017 repair and its formatting commit (2026-09-15, 18:07 UTC), produced the explicit
`Created a CLAP bundle` line for each profile. Debug `clap-validator` reported `44 tests run, 32
passed, 0 failed, 1 warnings, 11 skipped`; against that debug artifact Player behavior reported 8
passed, 0 failed and 1 native-window ignored test, and Player robustness reported 8 passed,
including the debug callback-allocation guard. Release was then rebuilt and `clap-validator`
reported the same `44 tests run, 32 passed, 0 failed, 1 warnings, 11 skipped`. After that release
bundle replaced the debug artifact, both Player suites were rerun with the same results; the
robustness allocation-named case retains only the finite-output oracle there, because allocation
assertions are a debug-bundle property. The same tree passed the operator's all-`mxm-preset`-consumer
sweep (29 test binaries, 934 passed, 0 failed, 32 ignored, including this plugin) and workspace
Clippy with `-D warnings`. The DSP crate did not change after its last proof at `327d7ce` (55 unit +
8 proof + 1 compatibility passes). The release DLL `target/release/mxm_fx_convolution.dll` and the
staged Windows CLAP binary `target/bundled/mxm-fx-convolution.clap` (9,361,920 bytes) both have
SHA-256 `734e2b0bfc39eeab5342769bbec9b0c1283d2cd4edff43b9c4cf74112ce97f0e`. The later F018
compiled repair supersedes that artifact pending the operator's next release-last gate; the earlier
`0739c961…` artifact from `327d7ce` is also superseded. The warning is the validator's denormal
timing heuristic; its skips are unsupported or Unix-only capabilities. No skipped validator
capability or ignored native-window case is counted as a pass.

### Revision 8 Feedback repair

The later Revision 8 Feedback repair was verified manually in the existing worktree, without the
factory controller. DSP reported 77 passed / 2 ignored, 11 proof passes and the Revision 6 lock;
plugin tests reported 85 passed / 1 generator ignored; scoped Clippy passed with `-D warnings` and
format check passed. A fresh release bundle (9,376,768 bytes, SHA-256
`1452d9543dd2d686c4155d98e14ca3e082cd18905f6c6204a3b881a4be50dc05`) passed validator with 32
passed, zero failed, one denormal warning and 11 inapplicable skips. Against that artifact Player
behavior reported 10 passed / 1 native-window ignored and robustness 16 passed / 1 owner-catalogue
ignored. The catalogue case was then run explicitly and passed: concrete stairwell 1.00–1.02, small
echo chamber 0.98–1.00 and walk-in closet 1.00–1.02, each bracket asserted rather than printed only.
The debug bundle, ignored native-window case and broad workspace gates were not rerun in this
repair.

### The 0.1.1 High cut correction

The 0.1.1 High cut correction then passed 78 DSP unit tests / 2 tuning ignores, 11 proof tests, the
private Revision 6 DSP compatibility lock, and 87 plugin tests / 1 generator ignore. Scoped Clippy
passed with warnings denied. A fresh release bundle (9,375,232 bytes, SHA-256
`5827402bd0672744f13cbc344478d2ef96fc7498e1e79b0a77beffce82d23117`) passed validator with 32
passed, zero failed, one denormal warning and 11 inapplicable skips. Against that artifact Player
behavior reported 10 passed / 1 native-window ignored and robustness 16 passed / 1 owner-catalogue
ignored; the catalogue case was run explicitly and retained its asserted 0.98–1.02 Feedback-edge
brackets. The package-version migration test proves empty/0.1.0 bottom-Open state moves to top
while 0.1.1's real closed zero survives a second filter pass. The debug bundle, native-window case,
broad workspace gates and `mxm-shimmer` tests were not rerun; shimmer was read and found to expose
no Off sentinel, so no code changed there.

### The Response-plot caption repair

After the Response-plot caption repair and merge with main's impulse-folder/audio-file work, all 91
plugin tests passed / 1 generator ignored, including the painted-shape regression that requires both
axis-caption bounds inside the canvas clip; DSP reported 78 passed / 2 tuning ignores, 11 proof
passes and the Revision 6 lock. Scoped all-target Clippy passed with warnings denied, scoped format
check and `git diff --check` passed, and the combined nice-plug wrapper's five focused
state/event/tail tests passed. The fresh release bundle is 9,402,880 bytes with SHA-256
`bc32ec2597214ee217eaf90b862396fa7f54e0ec0475e2f6f7a54853e8b60d0f`; validator reported 32 passed,
zero failed, one denormal warning and 11 skips. Player behavior reported 10 passed / 1
native-window ignore and robustness 16 passed / 1 owner-catalogue ignore. MXM Player was rebuilt
and started against that bundle as PID 20504. The ignored native-window and owner-catalogue cases
and broad workspace gates were not run.
