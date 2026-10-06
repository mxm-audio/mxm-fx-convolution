# mxm-fx-convolution

An original convolution effect for embedded mono and stereo responses. It keeps the active response
inside host state rather than depending on a source path, then applies it with a zero-delay direct
head and partitioned FFT tail.

Part of the [MXM Synth Collection](../../README.md). MIT licensed, CLAP only.

## What it does

- **Mix** — a linear dry/wet crossfade, and the one level: every response is normalised to the same
  loudness as it is loaded, so at 50 % the reverb is as loud as the dry sound whichever room it is.
  Zero is a real Off state: the response history fades out, clears and parks while dry audio
  continues unchanged.
- **Pre-delay** — places the wet onset up to 500 ms after the dry sound, with a bounded tap
  transition rather than a discontinuous jump.
- **Low cut, High cut and Tone** — linear Wet EQ after the response crossfade. High cut runs from a
  real closed 0 Hz at the bottom to a real 20 kHz maximum displayed as Open at the top.
- **Width** — a neutral-by-default mid/side matrix over the wet output pair.
- **Modulation** — neutral-by-default bounded post-convolution movement.
- **Feedback** — zero is an exact bypass. Raising it sends the raw convolved wet signal back through
  the same response, lengthening the tail while also smearing and usually darkening it; this is not
  the same room held longer. Past unity a qualifying response can sustain, with the return-only soft
  saturator shaping it into a self-oscillating roar rather than acting as an output safety limiter.
- **Response** — Browse or drop a mono/stereo WAV, choose its explicit mono, mono-to-stereo or
  diagonal-stereo interpretation, and prepare it reversibly with onset, extent, Reverse, linked
  Size, four unchanged tail shapes, Decay and time-varying Damping. Up to 10 seconds is retained
  unchanged; longer files
  are accepted and smoothly faded to exact zero at the 10-second canonical boundary. Immutable
  source samples and the model are embedded; paths and prepared FFT data are never project or user-
  preset state.
- **Factory presets are the impulses folder** — every WAV in the collection's impulses folder is a
  preset, found when the editor opens and listed under its folder (*Halls*, *Rooms*, …). Choosing
  one loads that room at the default settings; the room is then part of your project, so the
  project opens without the folder. With no folder installed the list is Init alone.

Mono input is rendered to stereo; stereo input retains independent left/right dry paths. Feedback's
soft saturator exists only inside its return. The dry path, feed-forward wet path, Wet EQ,
Modulation, Width and Mix remain linear; telemetry reports rather than hides output
clipping.

## Status

The shell, shipped content and dynamically paged four-card editor are implemented. The realtime core
exposes nine live parameters and persists Size/Decay/Damping as non-automatable model state, and
the editor binds all nine live parameters. WAV
acquisition, reversible preparation, truthful source plotting and source-bearing user presets remain
available. Headless tests cover private
state, accessibility, gestures, fixed-physical scaling and dynamic reflow; focused release/debug CLAP
and MXM Player tests cover both explicit layouts, discovery, state, automation boundaries, hostile
host configurations, tails, rendering, bypass and floating editor reopen. Native editor QA, owner
listening and a commercial-DAW run remain manual. A short deterministic project-authored starter
response keeps the inserted effect engaged before a WAV is chosen.

## Building

```bash
cargo xtask bundle mxm-fx-convolution --release
clap-validator validate "target/bundled/mxm-fx-convolution.clap"
```
