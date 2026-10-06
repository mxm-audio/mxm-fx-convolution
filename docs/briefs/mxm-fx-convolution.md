# mxm-fx-convolution — UI design brief

Required by mxm-kit's [`MXM_DESIGN_SYSTEM.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/MXM_DESIGN_SYSTEM.md) §14; Revision 7 is written before its clamp/Feedback editor rework.
This is an owner-named, original convolution effect, not a hardware copy and not an interface derived from another product.
Technique evidence is `research:effects/convolution-reverb.md`.

**Plugin:** `mxm-fx-convolution`; permanent CLAP id `dk.mxm.mxm-fx-convolution`. The crate,
package, bundle, `plugin_name!` literal and id all use the owner-approved `mxm-fx-convolution` name.
Product plan: `plans/plan-mxm-fx-convolution.md`.

## 1. Primary sound-design task

**Load a WAV, understand the response it contains, then turn that response into a space or a
creative long filter without losing the original.** File acquisition and response truth come first;
transformations are reversible settings derived from the embedded source rather than destructive
edits to it. **Owner ruling, 2026-09-14:** WAVs at or below 10.0 seconds are embedded untouched; a
longer WAV is accepted and canonicalized to its first 10.0 seconds with a smooth deterministic fade
to exact zero at that boundary. This sole acquisition-time bound is not a reversible model edit.

## 2. Controls reached for most

1. **Size** — the shipped linked resample, promoted to its primary name; duration and spectral scale
   move together, with visible adjustment to the largest value the active budget can play.
2. **Decay** — shortens or lengthens the perceived tail; lengthening can only retain or raise late
   energy already present inside immutable source support.
3. **Damping** — makes high-frequency energy decay progressively sooner while preparing the fixed IR.
4. **Feedback** — recirculates the prepared response from certified decay into response-dependent,
   bounded self-oscillation; high settings may become a saturating, progressively smeared roar, while
   zero is exact Revision 6 behavior.
5. **Modulation** — adds bounded stereo movement after convolution, without animated IR partitions.

Mix remains the persistent output balance and Off control. Width and wet Low cut, High cut and Tone
are direct secondary controls. High cut is a real 0–20 kHz frequency: its bottom is fully closed and
its real maximum is displayed as Open at the top; no public endpoint is a bypass sentinel. The source
drop target, interpretation, onset/extent and Reverse remain equally
direct operations, but acquisition and response rebuilding are not performance knobs.

## 3. Signal flow that must be visible

```text
WAV -> validate / interpretation -> onset/extent / Reverse / Tail shape / Decay / Damping / Size
                                                                                 |
input -> excitation sum ------------> active/new partitioned FIR -> Pre-delay ---+
            ^                                  |                                 v
            +-- Feedback <- z^-1 <- soft sat <- return routing <-+ raw FIR blend   -> response crossfade
                                                                                 |
                                                                      wet Low/High/Tone
                                                                                 |
                                                                      modulated wet delay
                                                                                 |
                                                                       mid/side Width ----+
input -----------------------------------------------------------------------> dry -------+-> Mix
```

The panel must communicate that the WAV is the filter, that mono/stereo channel interpretation is an
explicit choice rather than a file-width guess, that preparation creates one complete response off
the audio thread, and that a loaded response owns the audible tail. Size, Decay and Damping are
preparation-only recipe state. Feedback, Pre-delay, Wet Low cut, High cut, Tone, Modulation, Width
and Mix are live automatable controls. **Mix is the one level** (the owner, 2026-09-28): every
response is normalised to one loudness as it is prepared, so at Mix 50 % the reverb is as loud as
the dry sound whichever room is loaded, and Wet gain is gone.

Feedback takes the complementary raw FIR blend before Pre-delay, applies the declared mono-fold or
diagonal return routing, response-relative gain and one explicit bounded soft saturator, then returns
it through one sample of delay to convolution excitation. Its **Self-osc possible** mark is where the
conservative decay certificate ends, not a calibrated unity threshold. Above that mark a supported
loop mode may grow into self-oscillation until the return curve balances it, depending on the loaded
response and routing; the interface does not promise sustain for every WAV. Pre-delay follows each response FIR before old/new response crossfade. Wet EQ,
Modulation and Width are single linear stages after that crossfade; Width always reads the resulting
stereo wet pair regardless of host input width. Engaged Modulation makes the feed-forward wet path
linear time-varying rather than LTI. The return saturator is the only nonlinearity and is a character
choice, not ear/speaker protection. No limiter, sample/output clamp, automatic gain or saturator on
dry audio, wet output or post-convolution stages is hidden from this diagram; the one level change
is the response's normalisation, a fixed gain set when it is prepared.

## 4. Play view

There is no Play view and no authored view bar. This is a standalone effect. Space-derived paging
shows one or more Effects pages without changing controller pages or card identities.

## 5. Advanced controls and disclosure

No sound control is hidden. Status and uncommon source diagnostics may use a disclosure, but the
loaded file, interpretation, preparation settings and output controls remain directly reachable.

| Stable card | Primary content | Job |
|---|---|---|
| **Response** | WAV drop/Browse, source name and validity, waveform/energy envelope, channel interpretation | Establish exactly what coefficients will be used |
| **Time** | onset/extent, Reverse, Size and Decay | Arrange when response energy arrives and dies away |
| **Shape** | existing Tail shape, Damping, wet Low cut/High cut/Tone, Modulation and Feedback | Turn the response into a filtered, moving or recursively lengthened space |
| **Output** | Width, Mix, response/loading state and level | Place and balance the result and make Off or pending work explicit |

The embedded WAV and its declared channel interpretation are **source-owned**. Onset/extent,
Reverse, existing Tail shape, Size, Decay and Damping are **recipe-owned preparation state**: Init
and a model-overlay recipe may reset or change them while preserving source. Feedback, Wet EQ,
Width, Modulation and the existing output controls are **live automatable parameters**. Useful ranges and voicing are implementation
listening outcomes, not guessed in this brief. Neutral means Size identity, natural Decay, Damping,
Modulation and Feedback off, Wet EQ flat/open and Width at unchanged L/R reconstruction. Feedback zero
is exact Revision 6/pre-Feedback behavior. Existing Natural/Fade/Swell/Gate behavior remains a distinct stage;
neutral Decay leaves every class unchanged rather than collapsing them into one law.

A control which requires response rebuilding shows pending/ready/result state and never changes sound
through a half-prepared response. At rates through the full-duration 96 kHz envelope, Size and Extent
clamp to the greatest fitting value; an open-ended Onset moved too early clamps to the earliest
fitting onset. Reverse, Tail shape, Decay, Damping and Feedback do not increase prepared support and
do not deadline-clamp. The edited control immediately displays the exact normalized request, a short
neutral information message says it was set to maximum, and the same value becomes committed state.
This is never red Error styling and never hidden coefficient truncation. A genuinely unsatisfiable or
high-rate request retains the plain visible refusal and the committed model unchanged.

All preparation edits are latest-wins: a drag coalesces obsolete work instead of queueing a backlog,
the committed response never stops, and only the newest complete candidate enters the existing
bounded crossfade. While work is pending, **Committed** names what is sounding and what can be saved;
**Requested** is visibly transient and Save remains unavailable until it either commits or is
cancelled.

## 6. Categories, cards, and grouping

All four cards have primary category **Effects** and follow preparation and signal order: Response,
Time, Shape, Output. Response and Time are a preferred same-category group while space permits,
because source interpretation and time preparation jointly define the FIR. Cards remain indivisible;
the shared paging renderer derives page count from available width and height.

**Owner decision, 2026-09-15:** there is no Convolution hardware-controller page. Mix keeps its
existing `fx.reverb` role; Pre-delay, Low cut, High cut, Tone, Width, Modulation and Feedback
remain unmapped in the MIDI bank while staying reachable here and through host automation.

## 7. Identity accent

Use the collection accent unchanged in both themes. File state, pending work, informational budget
adjustments, errors and clipping use distinct semantic status tokens plus text or shape, never hue
alone. This inherits the shared token contrast;
actual dark/light contrast and native rendering are still measured at the §15 gate.

## 8. Live visualization

The Response card keeps the shipped bounded energy overview unchanged: a fixed 0–10 s axis, peak and
RMS per 50 ms, and a 0 to −72 dB range, with the existing original/active-region and tail overlays.
Size, Decay, Damping, Width, Modulation and Feedback neither rescale nor repurpose it; the display
continues to show the prepared FIR, not recursive generations. It is not a decorative
room picture or a claim about physical geometry. A transition indicator distinguishes
**Preparing**, **Changing**, **Ready** and **Error**; a clamp is a separate neutral informational
message, not Error. The previous response remains visibly and audibly active until replacement is
ready.
The app bar retains the shared peak/clip meter.

## 9. What is removed from source layouts

There is no source hardware or software face to preserve. The public method contributes an FIR,
channel matrix, preparation pipeline and safe replacement discipline, not a panel. Product-specific
room graphics, microphone diagrams, rack metaphors, proprietary mode names and reference layouts are
absent. IR catalogues remain outside Git; only the project-authored starter is shipped in the
plugin. **The impulses folder is the factory set** (the owner, 2026-09-28, reversing *outside factory
presets*): every WAV in it is a preset, found when the editor opens and filed under its folder, and
loading one embeds its response as Browse does. **Every response is normalised by its energy** (the owner, 2026-09-28, reversing this
brief's first answer, which kept each file at its recorded level so as not to *erase evidence in
the source*): a room recorded quiet and one recorded loud now sit at one loudness, and Mix means the
same balance with either. One gain for the whole response, measured on its energy and not its peak,
so no channel's ratio to another moves and nothing about the file is edited — no largest-peak
trim, silence trim or channel guess.

Convolution's static prepared response, deterministic replay of IR noise, linked Size
time/spectral movement, Decay/Damping-induced spectral change, bounded modulation-delay movement,
long source-owned tail and potentially extreme spectral-overlap-dependent level variation are
audible boundaries of the chosen method. Feedback repeatedly convolves the raw wet signal with the
same response: it lengthens the audible decay but increasingly smears and usually darkens it. It is
**not the same room held longer**, and on very short responses the mandatory one-sample return can
sound comb-like, metallic or pitched. Below the **Self-osc possible** mark its composed return bound
proves geometric decay. Above the mark that proof no longer applies; depending on the loaded response,
the loop may sustain without input and grow until the explicit return-only soft curve balances it.
High Feedback can therefore become a self-oscillating, saturating character—not a longer version of
the same room. The curve bounds loop excitation and implies the response- and setting-dependent wet bound
specified by plan §4.2, but it does not promise 0 dBFS or protect ears and speakers. A sustaining loop
uses `ProcessStatus::KeepAlive`; the wrapper notifies the host whenever automation or state moves
between finite and infinite tail classes. It remains alive until Feedback is lowered or zeroed, or
Mix/Off/reset clears it. Freeze remains excluded. The Output card shows a latched clip indicator, but telemetry never changes gain or
audio. A limiter, sample/output clamp, gain reducer, a level follower, or saturator anywhere
outside the Feedback return is deliberately out of scope; the response's one normalising gain is
fixed at preparation and is not one. NaN/inf recovery and denormal flushing
remain fault recovery, never level control.
Circular wrap, missed partition deadlines, sample-rate mismatch, denormal stalls, unsafe file
publication and response-change clicks are defects, not character.

## 10. Fit, reflow, zoom, and Init

Expected use is 44.1 and 48 kHz. Revision 7 changes ordinary model overshoot into a visible clamp but
does not change shipped sample-rate rounding, activation or validator gates. The panel is laid out at the 1920 × 1080 physical quarter-4K budget and its opening size is derived by
hugging what the complete panel draws; no opening dimensions are guessed here. Each card's floor is
the larger of its paint-overflow and usable-control floors. At smaller sizes the shared renderer
pages whole cards and gives an indivisible overflow both-axis reachability. At 150% and 200% editor
zoom the physical test window stays fixed and controls remain reachable through reflow or scrolling.
Implementation records the development machine's DPI scale and verifies every derived page, both
themes, disclosures open, native window chrome included.

Init uses a deterministic project-authored starter response so a newly inserted effect is audibly
engaged without shipping third-party audio. After a user load, shipped app-bar Init preserves the
embedded WAV and its declared channel interpretation, resets onset/extent to the complete source, and
resets Reverse, Tail shape, Size, Decay, Damping, Wet EQ, Width, Modulation, Feedback and output
settings to neutral. It uses plan §3.4's opt-in Pending → Ready → process-acknowledged transaction: no default
parameter gesture or response change occurs while preparation is pending, and preset identity does
not clear until the first callback after complete publication acknowledges it. Cancellation leaves
sound, parameters and identity unchanged; a newer selection, Init, WAV load or response-model edit
supersedes unpublished work. Parameter edits made during preparation or after publication remain
outside the preset's retained target baseline, so the acknowledged preset immediately says Modified.
Save and Save As are unavailable while the transaction is pending, including a Save As row opened
before it began. Model-overlay recipes may change
that same recipe-owned set while preserving source-owned content; they never substitute somebody
else's capture. Every shipped live id remains permanent; Feedback takes the new id `feedback` and
is unmapped in the MIDI bank. Old `time_percent` keeps its meaning under the Size label, old
`tail_shape` retains its exact Natural/Fade/Swell/Gate stage, and missing Feedback loads as zero. All
fifty existing parameter-only recipes plus the starter response must render bit-identically to the
pre-Feedback Revision 7 baseline at Feedback zero, retaining their existing Revision 6 golden null.
The normalisation and Wet gain's deletion (2026-09-28) recaptured that null, with a measured reason:
the starter's reverb rises 4.84 dB at 48 kHz, and each recipe's Mix and Wet gain fold into one Mix
keeping its ratio of reverb to dry sound.

## Sign-off

- [x] §14's ten questions are answered before editor work.
- [x] Product identity and source-versus-setting boundary are explicit.
- [x] Signal flow, stable card inventory and responsive behavior are defined without fixed geometry.
- [x] Owner listening fixes the Revision 6 control families and preparation/live classification.
- [x] The shared asynchronous overlay transaction is wired through production preset selection and model-reset Init.
- [ ] Revision 7 proves exact visible Size/Extent/conditional-Onset clamping and genuine refusal.
- [ ] Revision 7 proves the composed return bound, certified-decay/self-oscillation behavior, staged
  response replacement, finite/`KeepAlive` tail honesty and host notification, explicit loop delay,
  and exact zero compatibility.
- [ ] Revision 6 listening fixes useful ranges and the one-knob modulation law.
- [x] Owner leaves the new live controls unmapped; Mix alone retains `fx.reverb`.
- [ ] Native dark/light, DPI, fixed-window zoom, keyboard and contrast checks pass §15.
- [ ] Owner listening and visual sign-off.
