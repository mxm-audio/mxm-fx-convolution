//! Hostile activation, callback and event shapes for `mxm-fx-convolution` through a direct CLAP
//! host. Run this file against both release and debug bundles; the debug bundle's nice-plug process
//! allocation guard aborts on any allocation inside the plugin callback.

use mxm_player_harness::harness;

use clack_extensions::audio_ports_config::{AudioPortsConfigBuffer, PluginAudioPortsConfig};
use clack_extensions::state::PluginState;
use clack_extensions::tail::{PluginTail, TailLength};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;
use clack_host::utils::Cookie;
use mxm_player::envelope::negotiate_effect;
use mxm_player::params::ParamSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

const PLUGIN: &str = "dk.mxm.mxm-fx-convolution";
const SKIP: &str = "skipping: run `cargo xtask bundle mxm-fx-convolution --release`";

fn bundle() -> Option<PathBuf> {
    let path = mxm_player_harness::workspace_root().join("target/bundled/mxm-fx-convolution.clap");
    path.exists().then_some(path)
}

#[derive(Copy, Clone)]
enum Target {
    Mix,
    Predelay,
    LowCut,
}

#[derive(Copy, Clone)]
enum Signal {
    Constant(f32),
    StereoConstant([f32; 2]),
    StereoImpulse([f32; 2]),
    Impulse,
    HostileFinite,
}

#[derive(Clone, Debug)]
struct Run {
    channels: Vec<Vec<f32>>,
    statuses: Vec<ProcessStatus>,
    tails: Vec<TailLength>,
    state_before: Vec<u8>,
    state_after: Vec<u8>,
}

fn run(
    file: &std::path::Path,
    sample_rate: f64,
    max_frames: u32,
    block_sizes: &[usize],
    signal: Signal,
    events: &[(u64, Target, f64)],
) -> Result<Run, String> {
    run_config(
        file,
        None,
        sample_rate,
        max_frames,
        block_sizes,
        signal,
        events,
    )
}

fn run_config(
    file: &std::path::Path,
    config_index: Option<u32>,
    sample_rate: f64,
    max_frames: u32,
    block_sizes: &[usize],
    signal: Signal,
    events: &[(u64, Target, f64)],
) -> Result<Run, String> {
    run_config_interpretation(
        file,
        config_index,
        None,
        sample_rate,
        max_frames,
        block_sizes,
        signal,
        events,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_config_interpretation(
    file: &std::path::Path,
    config_index: Option<u32>,
    interpretation: Option<&str>,
    sample_rate: f64,
    max_frames: u32,
    block_sizes: &[usize],
    signal: Signal,
    events: &[(u64, Target, f64)],
) -> Result<Run, String> {
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.map_err(|error| error.to_string())?;
    let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN)?;
    if let Some(interpretation) = interpretation {
        set_response_interpretation(&mut instance, interpretation);
    }
    let (input_channels, output_channels) = if let Some(config_index) = config_index {
        let extension: PluginAudioPortsConfig = instance
            .plugin_shared_handle()
            .get_extension()
            .expect("audio-ports-config extension");
        assert_eq!(extension.count(&mut instance.plugin_handle()), 2);
        let mut buffer = AudioPortsConfigBuffer::new();
        let config = extension
            .get(&mut instance.plugin_handle(), config_index, &mut buffer)
            .expect("advertised convolution configuration");
        let id = config.id;
        let input_channels = config.main_input.expect("main input").channel_count as usize;
        let output_channels = config.main_output.expect("main output").channel_count as usize;
        extension
            .select(&mut instance.plugin_handle(), id)
            .expect("select configuration while deactivated");
        (input_channels, output_channels)
    } else {
        let envelope = negotiate_effect(&mut instance).map_err(|error| error.to_string())?;
        (
            envelope.input.channel_count as usize,
            envelope.output.channel_count as usize,
        )
    };
    let params = ParamSet::read(&mut instance);
    let state_before = save_state(&mut instance);
    let tail: PluginTail = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("tail extension");
    let parameter_id = |name| {
        ClapId::from_raw(
            params
                .params
                .iter()
                .find(|parameter| parameter.name == name)
                .unwrap_or_else(|| panic!("{name} parameter"))
                .id,
        )
        .expect("valid parameter id")
    };
    let mix = parameter_id("Mix");
    let predelay = parameter_id("Pre-delay");
    let low_cut = parameter_id("Low cut");

    let processor = instance
        .activate(
            |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
            PluginAudioConfiguration {
                sample_rate,
                min_frames_count: 1,
                max_frames_count: max_frames,
            },
        )
        .map_err(|error| error.to_string())?;

    let mut channels = vec![Vec::new(); output_channels];
    let mut statuses = Vec::with_capacity(block_sizes.len());
    let mut tails = Vec::with_capacity(block_sizes.len());
    let stopped = {
        let mut processor = processor
            .start_processing()
            .map_err(|error| error.to_string())?;
        let capacity = max_frames as usize;
        let mut inputs = vec![vec![0.0f32; capacity]; input_channels];
        let mut outputs = vec![vec![0.0f32; capacity]; output_channels];
        let mut input_ports = AudioPorts::with_capacity(input_channels, 1);
        let mut output_ports = AudioPorts::with_capacity(output_channels, 1);
        let mut input_events = EventBuffer::new();
        let mut output_events = EventBuffer::new();
        let mut absolute = 0usize;
        let mut next_event = 0usize;

        for &frames in block_sizes {
            assert!(frames <= capacity);
            for (input_index, input) in inputs.iter_mut().enumerate() {
                for (offset, sample) in input[..frames].iter_mut().enumerate() {
                    let frame = absolute + offset;
                    *sample = match signal {
                        Signal::Constant(value) => value,
                        Signal::StereoConstant(values) => values[input_index],
                        Signal::StereoImpulse(values) => {
                            if frame == 0 {
                                values[input_index]
                            } else {
                                0.0
                            }
                        }
                        Signal::Impulse => (frame == 0) as u8 as f32,
                        Signal::HostileFinite => {
                            if frame.is_multiple_of(997) {
                                f32::MAX
                            } else {
                                (frame as f32 * 0.013_579).sin() * 0.9
                            }
                        }
                    };
                }
            }
            for output in &mut outputs {
                output[..frames].fill(0.0);
            }
            input_events.clear();
            output_events.clear();
            while next_event < events.len() && events[next_event].0 < (absolute + frames) as u64 {
                let (at, target, value) = events[next_event];
                input_events.push(&ParamValueEvent::new(
                    at.saturating_sub(absolute as u64) as u32,
                    match target {
                        Target::Mix => mix,
                        Target::Predelay => predelay,
                        Target::LowCut => low_cut,
                    },
                    Pckn::match_all(),
                    value,
                    Cookie::empty(),
                ));
                next_event += 1;
            }

            let audio_inputs = input_ports.with_input_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only(
                    inputs
                        .iter_mut()
                        .map(|buffer| InputChannel::variable(&mut buffer[..frames])),
                ),
            }]);
            let mut audio_outputs = output_ports.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    outputs.iter_mut().map(|buffer| &mut buffer[..frames]),
                ),
            }]);
            let status = processor
                .process(
                    &audio_inputs,
                    &mut audio_outputs,
                    &InputEvents::from_buffer(&input_events),
                    &mut OutputEvents::from_buffer(&mut output_events),
                    Some(absolute as u64),
                    None,
                )
                .map_err(|error| error.to_string())?;
            assert!(output_events.is_empty(), "effect emitted an output event");
            statuses.push(status);
            tails.push(tail.get(&processor.plugin_handle()));
            for (destination, output) in channels.iter_mut().zip(&outputs) {
                destination.extend_from_slice(&output[..frames]);
            }
            absolute += frames;
        }
        processor.stop_processing()
    };
    instance.deactivate(stopped);
    let state_after = save_state(&mut instance);
    Ok(Run {
        channels,
        statuses,
        tails,
        state_before,
        state_after,
    })
}

fn save_state(instance: &mut PluginInstance<mxm_player::host::MxmHost>) -> Vec<u8> {
    let extension: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    let mut bytes = Vec::new();
    extension
        .save(&mut instance.plugin_handle(), &mut bytes)
        .expect("save plugin state");
    bytes
}

fn set_response_interpretation(
    instance: &mut PluginInstance<mxm_player::host::MxmHost>,
    interpretation: &str,
) {
    let extension: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    let mut bytes = Vec::new();
    extension
        .save(&mut instance.plugin_handle(), &mut bytes)
        .expect("save response state");
    let size = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let mut state: serde_json::Value = serde_json::from_slice(&bytes[8..8 + size]).unwrap();
    let mut response: serde_json::Value = serde_json::from_str(
        state["fields"]["response"]
            .as_str()
            .expect("serialized response field"),
    )
    .unwrap();
    response["interpretation"] = serde_json::Value::String(interpretation.to_owned());
    state["fields"]["response"] =
        serde_json::Value::String(serde_json::to_string(&response).unwrap());
    let json = serde_json::to_vec(&state).unwrap();
    let mut encoded = Vec::with_capacity(json.len() + 8);
    encoded.extend_from_slice(&(json.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&json);
    extension
        .load(&mut instance.plugin_handle(), &mut encoded.as_slice())
        .expect("load response interpretation");
}

#[derive(Clone, Copy)]
enum RecursiveAction {
    None,
    Raise,
    FeedbackOff,
    MixParkAndWake,
    Reset,
}

struct RecursiveRun {
    channels: [Vec<f32>; 2],
    statuses: Vec<ProcessStatus>,
    tails: Vec<TailLength>,
    tail_changed_after_action: bool,
}

/// The owner's exact case: launch on the init patch, touch nothing, raise Feedback to its maximum.
///
/// Every other feedback proof loads state first, which forces a reactivation that re-applies the
/// control. This one never loads state, so it is the only test shaped like a user turning the knob
/// on a freshly opened plugin.
#[test]
fn init_patch_feedback_raised_by_automation_alone() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let render = |feedback: f32| -> (f64, f64) {
        // SAFETY: this is our own bundle from the build tree.
        let entry = unsafe { PluginEntry::load(&file) }.expect("load bundle");
        let (_shared, mut instance) =
            harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
        // Deliberately NO state load: this is the init patch exactly as it opens.
        let params = ParamSet::read(&mut instance);
        let feedback_id = ClapId::from_raw(
            params
                .params
                .iter()
                .find(|parameter| parameter.name == "Feedback")
                .expect("Feedback parameter")
                .id,
        )
        .unwrap();
        let processor = instance
            .activate(
                |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
                PluginAudioConfiguration {
                    sample_rate: 48_000.0,
                    min_frames_count: 64,
                    max_frames_count: 64,
                },
            )
            .expect("activate init patch");
        let mut processor = processor.start_processing().expect("start init patch");
        let mut before = 0.0f64;
        let mut after = 0.0f64;
        for block in 0..3_000 {
            let mut input = [0.0f32; 64];
            // Continuous excitation, as an instrument feeding the effect would be.
            for (offset, sample) in input.iter_mut().enumerate() {
                let n = (block * 64 + offset) as f32;
                *sample = (n * 0.01).sin() * 0.25;
            }
            let mut left = [0.0f32; 64];
            let mut right = [0.0f32; 64];
            let mut input_ports = AudioPorts::with_capacity(1, 1);
            let mut output_ports = AudioPorts::with_capacity(2, 1);
            let audio_inputs = input_ports.with_input_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only([InputChannel::variable(&mut input)]),
            }]);
            let mut audio_outputs = output_ports.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only([&mut left[..], &mut right[..]]),
            }]);
            let mut input_events = EventBuffer::new();
            if block == 750 {
                input_events.push(&ParamValueEvent::new(
                    0,
                    feedback_id,
                    Pckn::match_all(),
                    f64::from(feedback),
                    Cookie::empty(),
                ));
            }
            let mut output_events = EventBuffer::new();
            processor
                .process(
                    &audio_inputs,
                    &mut audio_outputs,
                    &InputEvents::from_buffer(&input_events),
                    &mut OutputEvents::from_buffer(&mut output_events),
                    Some((block * 64) as u64),
                    None,
                )
                .expect("process init patch");
            let energy: f64 = left
                .iter()
                .chain(right.iter())
                .map(|sample| f64::from(*sample).powi(2))
                .sum();
            if (500..750).contains(&block) {
                before += energy;
            }
            if block >= 2_750 {
                after += energy;
            }
        }
        instance.deactivate(processor.stop_processing());
        (before, after)
    };

    let (quiet_before, quiet_after) = render(0.0);
    let (hot_before, hot_after) = render(1.25);
    eprintln!("init patch, feedback stays 0:   before={quiet_before:e} after={quiet_after:e}");
    eprintln!("init patch, feedback -> 1.25:   before={hot_before:e} after={hot_after:e}");
    eprintln!(
        "ratio after/before  q=0 {:.3}   q=1.25 {:.3}",
        quiet_after / quiet_before,
        hot_after / hot_before
    );
}

fn encode_f32_channel(samples: &[f32]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(TABLE[((bits >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((bits >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((bits >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(bits & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

/// Install an arbitrary-length response plus live parameters, so response length becomes a variable.
fn configure_response_and_params(
    instance: &mut PluginInstance<mxm_player::host::MxmHost>,
    samples: &[f32],
    feedback: f32,
    mix: f32,
) {
    let mut bytes = save_state(instance);
    let size = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let mut state: serde_json::Value = serde_json::from_slice(&bytes[8..8 + size]).unwrap();
    state["fields"]["response"] = serde_json::Value::String(
        serde_json::to_string(&serde_json::json!({
            "schema": 1,
            "name": "Owner-scale fixture",
            "sample_rate": 48_000,
            "frames": samples.len(),
            "interpretation": "mono",
            "channels": [encode_f32_channel(samples)],
            "preparation": {
                "onset": 0,
                "extent": 0,
                "reverse": false,
                "time_percent": 100,
                "tail_shape": "natural",
                "decay_percent": 100,
                "damping_percent": 0
            }
        }))
        .unwrap(),
    );
    for (id, value) in [
        ("mix", mix),
        ("predelay", 0.0),
        ("lowcut", 0.0),
        ("highcut", 20_000.0),
        ("tone", 0.0),
        ("width", 1.0),
        ("modulation", 0.0),
        ("feedback", feedback),
    ] {
        state["params"][id] = serde_json::json!({ "f32": value });
    }
    let json = serde_json::to_vec(&state).unwrap();
    bytes.clear();
    bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&json);
    let extension: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    extension
        .load(&mut instance.plugin_handle(), &mut bytes.as_slice())
        .expect("load owner-scale fixture");
}

/// Isolates the two variables separating a passing measurement from the owner's silent session:
/// response length, and whether Feedback arrives by state load or by a live host automation event.
#[test]
fn feedback_across_response_length_and_control_route() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };

    fn decaying(len: usize, decay: f32) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        (0..len)
            .map(|index| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                noise * (-(index as f32) / decay).exp()
            })
            .collect()
    }

    let render = |samples: &[f32], feedback: f32, automate: bool| -> f64 {
        // SAFETY: this is our own bundle from the build tree.
        let entry = unsafe { PluginEntry::load(&file) }.expect("load bundle");
        let (_shared, mut instance) =
            harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
        let initial = if automate { 0.0 } else { feedback };
        configure_response_and_params(&mut instance, samples, initial, 0.38);
        let params = ParamSet::read(&mut instance);
        let feedback_id = ClapId::from_raw(
            params
                .params
                .iter()
                .find(|parameter| parameter.name == "Feedback")
                .expect("Feedback parameter")
                .id,
        )
        .unwrap();
        let processor = instance
            .activate(
                |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
                PluginAudioConfiguration {
                    sample_rate: 48_000.0,
                    min_frames_count: 64,
                    max_frames_count: 64,
                },
            )
            .expect("activate fixture");
        let mut processor = processor.start_processing().expect("start fixture");
        let mut energy = 0.0f64;
        // Ten seconds. A two-second impulse returns its energy smeared across the following two
        // seconds, so a 0.8 s window cannot complete even one pass round the loop, let alone show
        // whether it grows.
        for block in 0..7_500 {
            let mut input = [0.0f32; 64];
            if block == 0 {
                input[0] = 0.5;
            }
            let mut left = [0.0f32; 64];
            let mut right = [0.0f32; 64];
            let mut input_ports = AudioPorts::with_capacity(1, 1);
            let mut output_ports = AudioPorts::with_capacity(2, 1);
            let audio_inputs = input_ports.with_input_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only([InputChannel::variable(&mut input)]),
            }]);
            let mut audio_outputs = output_ports.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only([&mut left[..], &mut right[..]]),
            }]);
            let mut input_events = EventBuffer::new();
            if automate && block == 1 {
                input_events.push(&ParamValueEvent::new(
                    0,
                    feedback_id,
                    Pckn::match_all(),
                    f64::from(feedback),
                    Cookie::empty(),
                ));
            }
            let mut output_events = EventBuffer::new();
            processor
                .process(
                    &audio_inputs,
                    &mut audio_outputs,
                    &InputEvents::from_buffer(&input_events),
                    &mut OutputEvents::from_buffer(&mut output_events),
                    Some((block * 64) as u64),
                    None,
                )
                .expect("process fixture");
            // Past the dry impulse; only the response and its recirculation remain.
            if block >= 8 {
                for sample in left.iter().chain(right.iter()) {
                    energy += f64::from(*sample).powi(2);
                }
            }
        }
        instance.deactivate(processor.stop_processing());
        energy
    };

    let short = decaying(336, 100.0);
    let long = decaying(96_000, 30_000.0);
    for (label, samples) in [("short 7ms", &short), ("long 2s", &long)] {
        let quiet = render(samples, 0.0, false);
        let loaded = render(samples, 1.25, false);
        let automated = render(samples, 1.25, true);
        eprintln!(
            "{label}: q=0 {quiet:e} | q=1.25 state-load {loaded:e} | q=1.25 automated {automated:e}"
        );
    }
}
///
/// Every other recursive fixture replaces the response with a one-tap `H = [1]`, where the peak
/// magnitude is exactly 1 and `q` is trivially the loop gain. That cannot detect a defect whose
/// size depends on the response, which is the configuration an owner actually listens to.
fn configure_real_response_feedback(
    instance: &mut PluginInstance<mxm_player::host::MxmHost>,
    feedback: f32,
    mix: f32,
) {
    let mut bytes = save_state(instance);
    let size = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let mut state: serde_json::Value = serde_json::from_slice(&bytes[8..8 + size]).unwrap();
    for (id, value) in [
        ("mix", mix),
        ("predelay", 0.0),
        ("lowcut", 0.0),
        ("highcut", 20_000.0),
        ("tone", 0.0),
        ("width", 1.0),
        ("modulation", 0.0),
        ("feedback", feedback),
    ] {
        state["params"][id] = serde_json::json!({ "f32": value });
    }
    let json = serde_json::to_vec(&state).unwrap();
    bytes.clear();
    bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&json);
    let extension: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    extension
        .load(&mut instance.plugin_handle(), &mut bytes.as_slice())
        .expect("load real-response feedback fixture");
}

#[test]
fn feedback_is_audible_on_the_plugins_own_response() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let render = |feedback: f32| -> (f64, f32) {
        // SAFETY: this is our own bundle from the build tree.
        let entry = unsafe { PluginEntry::load(&file) }.expect("load bundle");
        let (_shared, mut instance) =
            harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
        configure_real_response_feedback(&mut instance, feedback, 1.0);
        let processor = instance
            .activate(
                |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
                PluginAudioConfiguration {
                    sample_rate: 48_000.0,
                    min_frames_count: 64,
                    max_frames_count: 64,
                },
            )
            .expect("activate real-response fixture");
        let mut processor = processor
            .start_processing()
            .expect("start real-response fixture");
        let mut energy = 0.0f64;
        let mut peak = 0.0f32;
        for block in 0..200 {
            let mut input = [0.0f32; 64];
            if block == 0 {
                input[0] = 0.5;
            }
            let mut left = [0.0f32; 64];
            let mut right = [0.0f32; 64];
            let mut input_ports = AudioPorts::with_capacity(1, 1);
            let mut output_ports = AudioPorts::with_capacity(2, 1);
            let audio_inputs = input_ports.with_input_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only([InputChannel::variable(&mut input)]),
            }]);
            let mut audio_outputs = output_ports.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only([&mut left[..], &mut right[..]]),
            }]);
            let mut input_events = EventBuffer::new();
            let mut output_events = EventBuffer::new();
            processor
                .process(
                    &audio_inputs,
                    &mut audio_outputs,
                    &InputEvents::from_buffer(&input_events),
                    &mut OutputEvents::from_buffer(&mut output_events),
                    Some((block * 64) as u64),
                    None,
                )
                .expect("process real-response fixture");
            input_events.clear();
            // Past the dry impulse and the embedded response, so only recirculation survives.
            if block >= 4 {
                for sample in left.iter().chain(right.iter()) {
                    energy += f64::from(*sample).powi(2);
                    peak = peak.max(sample.abs());
                }
            }
        }
        instance.deactivate(processor.stop_processing());
        (energy, peak)
    };

    let (quiet, quiet_peak) = render(0.0);
    let (hot, hot_peak) = render(1.25);
    eprintln!("own response: q=0 energy={quiet:e} peak={quiet_peak:e}");
    eprintln!("own response: q=1.25 energy={hot:e} peak={hot_peak:e}");
    assert!(
        hot > quiet * 2.0,
        "Feedback at its maximum was inert on the plugin's own response: \
         q=0 {quiet:e} vs q=1.25 {hot:e}"
    );
}

fn configure_one_tap_feedback(
    instance: &mut PluginInstance<mxm_player::host::MxmHost>,
    feedback: f32,
    room_scale: bool,
) {
    let mut bytes = save_state(instance);
    let size = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let mut state: serde_json::Value = serde_json::from_slice(&bytes[8..8 + size]).unwrap();
    // The one-tap is the sharp host-tail oracle: response-specific Nyquist analysis now puts its
    // edge at 1.0, while its nearby H-infinity peak still certifies 0.99 as contractive. The
    // room-scale branch remains for stop/reset and audibility proofs, where a realistic long tail
    // gives the loop time and spectral density to establish visibly.
    let coefficients: Vec<f32> = {
        let mut state = 0x1234_5678u32;
        (0..if room_scale { 100_000 } else { 1 })
            .map(|index| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                if room_scale {
                    noise * (-(index as f32) / 30_000.0).exp()
                } else {
                    1.0
                }
            })
            .collect()
    };
    let response = serde_json::json!({
        "schema": 1,
        "name": "Room-scale recursive host fixture",
        "sample_rate": 48_000,
        "frames": coefficients.len(),
        "interpretation": "mono",
        "channels": [encode_f32_channel(&coefficients)],
        "preparation": {
            "onset": 0,
            "extent": 0,
            "reverse": false,
            "time_percent": 100,
            "tail_shape": "natural",
            "decay_percent": 100,
            "damping_percent": 0
        }
    });
    state["fields"]["response"] =
        serde_json::Value::String(serde_json::to_string(&response).unwrap());
    for (id, value) in [
        ("mix", 1.0),
        ("predelay", 0.0),
        ("lowcut", 0.0),
        ("highcut", 20_000.0),
        ("tone", 0.0),
        ("width", 1.0),
        ("modulation", 0.0),
        ("feedback", feedback),
    ] {
        state["params"][id] = serde_json::json!({"f32": value});
    }
    let json = serde_json::to_vec(&state).unwrap();
    bytes.clear();
    bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&json);
    let extension: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    extension
        .load(&mut instance.plugin_handle(), &mut bytes.as_slice())
        .expect("load one-tap recursive fixture");
}

fn run_recursive(
    file: &std::path::Path,
    initial_feedback: f32,
    action: RecursiveAction,
    room_scale: bool,
) -> RecursiveRun {
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
    let (shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
    configure_one_tap_feedback(&mut instance, initial_feedback, room_scale);
    let params = ParamSet::read(&mut instance);
    let id = |name| {
        ClapId::from_raw(
            params
                .params
                .iter()
                .find(|parameter| parameter.name == name)
                .unwrap_or_else(|| panic!("{name} parameter"))
                .id,
        )
        .unwrap()
    };
    let mix = id("Mix");
    let feedback = id("Feedback");
    let tail: PluginTail = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("tail extension");
    let processor = instance
        .activate(
            |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
            PluginAudioConfiguration {
                sample_rate: 48_000.0,
                min_frames_count: 64,
                max_frames_count: 64,
            },
        )
        .expect("activate recursive fixture");
    let mut processor = processor
        .start_processing()
        .expect("start recursive fixture");
    let mut channels = [Vec::new(), Vec::new()];
    let mut statuses = Vec::new();
    let mut tails = Vec::new();

    // 4,000 blocks, not 100. At 64 frames each that is 256,000 samples, enough for the room-scale
    // fixture's own 100,000-sample feed-forward tail to finish and for the loop to reach exact idle.
    // The old 100-block window was ample for a one-tap and cannot observe an end state at all once
    // the response is the length an owner actually loads: every assertion that failed here was an
    // end-state one - `ContinueIfNotQuiet`, `Finite(0)`, trailing silence - while the decay and
    // sustain assertions before them passed.
    for block in 0..4_000 {
        if (block == 1 && matches!(action, RecursiveAction::Raise))
            || (block == 20 && !matches!(action, RecursiveAction::Raise | RecursiveAction::None))
        {
            shared
                .notifications
                .tail_changed
                .store(false, Ordering::Release);
        }
        if block == 20 && matches!(action, RecursiveAction::Reset) {
            processor.reset();
        }
        let mut input = [0.0f32; 64];
        if block == 0 {
            input[0] = 0.5;
        }
        let mut left = [0.0f32; 64];
        let mut right = [0.0f32; 64];
        let mut input_ports = AudioPorts::with_capacity(1, 1);
        let mut output_ports = AudioPorts::with_capacity(2, 1);
        let audio_inputs = input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only([InputChannel::variable(&mut input)]),
        }]);
        let mut audio_outputs = output_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only([&mut left[..], &mut right[..]]),
        }]);
        let mut input_events = EventBuffer::new();
        if block == 1 && matches!(action, RecursiveAction::Raise) {
            input_events.push(&ParamValueEvent::new(
                0,
                feedback,
                Pckn::match_all(),
                1.25,
                Cookie::empty(),
            ));
        }
        if block == 20 {
            let target = match action {
                RecursiveAction::FeedbackOff => Some((feedback, 0.0)),
                RecursiveAction::MixParkAndWake => Some((mix, 0.0)),
                RecursiveAction::None | RecursiveAction::Raise | RecursiveAction::Reset => None,
            };
            if let Some((parameter, value)) = target {
                input_events.push(&ParamValueEvent::new(
                    0,
                    parameter,
                    Pckn::match_all(),
                    value,
                    Cookie::empty(),
                ));
            }
        }
        if block == 60 && matches!(action, RecursiveAction::MixParkAndWake) {
            input_events.push(&ParamValueEvent::new(
                0,
                mix,
                Pckn::match_all(),
                1.0,
                Cookie::empty(),
            ));
        }
        let mut output_events = EventBuffer::new();
        let status = processor
            .process(
                &audio_inputs,
                &mut audio_outputs,
                &InputEvents::from_buffer(&input_events),
                &mut OutputEvents::from_buffer(&mut output_events),
                Some((block * 64) as u64),
                None,
            )
            .expect("process recursive fixture");
        statuses.push(status);
        tails.push(tail.get(&processor.plugin_handle()));
        channels[0].extend_from_slice(&left);
        channels[1].extend_from_slice(&right);
    }
    let tail_changed_after_action = shared.notifications.tail_changed.load(Ordering::Acquire);
    instance.deactivate(processor.stop_processing());
    RecursiveRun {
        channels,
        statuses,
        tails,
        tail_changed_after_action,
    }
}

#[test]
fn a_parameter_event_inside_one_callback_splits_the_render_at_its_sample_offset() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let baseline =
        run(&file, 48_000.0, 2048, &[2048], Signal::Constant(0.25), &[]).expect("baseline renders");
    // The opening point establishes the lane's current value — the default Mix, so the first 257
    // samples match the baseline; the second point is the boundary the wrapper must split at rather
    // than applying to the start of the callback.
    let automated = run(
        &file,
        48_000.0,
        2048,
        &[2048],
        Signal::Constant(0.25),
        &[(0, Target::Mix, 0.30), (257, Target::Mix, 0.0)],
    )
    .expect("automated render");

    for (before, after) in baseline.channels.iter().zip(&automated.channels) {
        assert_eq!(&after[..257], &before[..257], "the event moved early");
        assert!(
            after[900..].iter().all(|sample| *sample == 0.25),
            "Mix zero did not finish at exact dry after its bounded fade"
        );
        assert!(
            before[900..]
                .iter()
                .zip(&after[900..])
                .any(|(left, right)| (left - right).abs() > 1e-5),
            "the event did not change the post-event render"
        );
    }
}

#[test]
fn a_wet_post_event_inside_one_callback_starts_at_its_sample_offset() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let baseline =
        run(&file, 48_000.0, 2048, &[2048], Signal::Constant(0.25), &[]).expect("baseline renders");
    let automated = run(
        &file,
        48_000.0,
        2048,
        &[2048],
        Signal::Constant(0.25),
        &[(0, Target::LowCut, 0.0), (257, Target::LowCut, 1.0)],
    )
    .expect("wet-post automation renders");

    for (before, after) in baseline.channels.iter().zip(&automated.channels) {
        assert_eq!(
            &after[..257],
            &before[..257],
            "the Low cut event moved early"
        );
        assert!(
            before[900..]
                .iter()
                .zip(&after[900..])
                .any(|(left, right)| left.to_bits() != right.to_bits()),
            "the Low cut event did not reach the wet render"
        );
        assert!(after.iter().all(|sample| sample.is_finite()));
    }
}

#[test]
fn an_impulse_reports_a_finite_tail_then_exact_idle() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let blocks = vec![64usize; 220];
    let rendered =
        run(&file, 48_000.0, 64, &blocks, Signal::Impulse, &[]).expect("impulse renders");
    assert!(matches!(
        rendered.statuses.first(),
        Some(ProcessStatus::ContinueIfNotQuiet)
    ));
    assert!(
        rendered
            .statuses
            .iter()
            .any(|status| matches!(status, ProcessStatus::Continue)),
        "nice-plug must keep the host running while finite tail work remains"
    );
    assert!(
        rendered
            .tails
            .iter()
            .any(|tail| matches!(tail, TailLength::Finite(frames) if *frames > 0)),
        "the tail extension must publish finite remaining work"
    );
    assert!(matches!(
        rendered.statuses.last(),
        Some(ProcessStatus::ContinueIfNotQuiet)
    ));
    assert_eq!(rendered.tails.last(), Some(&TailLength::Finite(0)));
    for channel in &rendered.channels {
        assert!(channel.iter().all(|sample| sample.is_finite()));
        assert!(channel.iter().any(|sample| *sample != 0.0));
        assert!(
            channel[channel.len() - 1024..]
                .iter()
                .all(|sample| *sample == 0.0),
            "the response did not reach exact finite idle"
        );
    }
}

#[test]
fn sub_unity_feedback_has_a_finite_decaying_tail_and_reaches_exact_idle() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let rendered = run_recursive(&file, 0.75, RecursiveAction::None, false);
    assert!(
        rendered
            .tails
            .iter()
            .all(|tail| matches!(tail, TailLength::Finite(_)))
    );
    assert!(
        rendered
            .tails
            .iter()
            .any(|tail| matches!(tail, TailLength::Finite(frames) if *frames > 0))
    );
    let block_peak = |block: usize| {
        rendered
            .channels
            .iter()
            .flat_map(|channel| &channel[block * 64..(block + 1) * 64])
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    };
    assert!(block_peak(1) > block_peak(8));
    assert!(matches!(
        rendered.statuses.last(),
        Some(ProcessStatus::ContinueIfNotQuiet)
    ));
    assert_eq!(rendered.tails.last(), Some(&TailLength::Finite(0)));
    assert!(rendered.channels.iter().all(|channel| {
        channel[channel.len() - 512..]
            .iter()
            .all(|sample| *sample == 0.0)
    }));
}

#[test]
fn feedback_automation_crosses_to_infinite_tail_and_notifies_the_real_host() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    // A project-authored one-tap `H = [1]` fixture makes q > 1 genuinely self-oscillating; no
    // factory response is assumed to cross that response-dependent boundary.
    let rendered = run_recursive(&file, 0.99, RecursiveAction::Raise, false);
    assert!(
        rendered
            .tails
            .iter()
            .take(1)
            .all(|tail| matches!(tail, TailLength::Finite(_)))
    );
    assert!(
        rendered
            .tails
            .iter()
            .any(|tail| matches!(tail, TailLength::Infinite))
    );
    assert!(
        rendered
            .statuses
            .iter()
            .any(|status| matches!(status, ProcessStatus::Continue))
    );
    assert!(
        rendered.tail_changed_after_action,
        "clap_host_tail.changed did not reach the Player host after finite -> infinite automation"
    );
    assert!(
        rendered
            .channels
            .iter()
            .all(|channel| channel.iter().all(|sample| sample.is_finite()))
    );
    assert!(rendered.channels.iter().any(|channel| {
        channel[channel.len() - 512..]
            .iter()
            .any(|sample| *sample != 0.0)
    }));
}

#[test]
fn feedback_off_mix_park_and_reset_each_stop_self_oscillation_exactly() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    for (name, action) in [
        ("Feedback Off", RecursiveAction::FeedbackOff),
        ("Mix-zero park and wake", RecursiveAction::MixParkAndWake),
        ("reset", RecursiveAction::Reset),
    ] {
        let rendered = run_recursive(&file, 1.25, action, true);
        assert!(
            rendered.tails[..20]
                .iter()
                .any(|tail| matches!(tail, TailLength::Infinite)),
            "{name}: fixture did not sustain before stop"
        );
        assert!(
            rendered.tail_changed_after_action,
            "{name}: host was not notified of infinite -> finite"
        );
        assert_eq!(
            rendered.tails.last(),
            Some(&TailLength::Finite(0)),
            "{name}: tail did not become finite idle"
        );
        assert!(
            rendered.channels.iter().all(|channel| {
                channel[channel.len() - 512..]
                    .iter()
                    .all(|sample| *sample == 0.0)
            }),
            "{name}: stale recursion survived stop/park/reset"
        );
    }
}

#[test]
fn both_advertised_layouts_are_selected_and_obey_dry_and_wet_routing() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let blocks = [256usize; 8];
    let mono_dry = run_config(
        &file,
        Some(0),
        48_000.0,
        256,
        &blocks,
        Signal::Constant(0.25),
        &[(0, Target::Mix, 0.0)],
    )
    .expect("mono-in/stereo-out layout");
    let stereo_dry = run_config(
        &file,
        Some(1),
        48_000.0,
        256,
        &blocks,
        Signal::StereoConstant([0.25, -0.5]),
        &[(0, Target::Mix, 0.0)],
    )
    .expect("stereo-in/stereo-out layout");
    assert_eq!(mono_dry.channels.len(), 2);
    assert_eq!(stereo_dry.channels.len(), 2);
    assert!(
        mono_dry.channels[0][900..]
            .iter()
            .all(|sample| *sample == 0.25)
    );
    assert!(
        mono_dry.channels[1][900..]
            .iter()
            .all(|sample| *sample == 0.25)
    );
    assert!(
        stereo_dry.channels[0][900..]
            .iter()
            .all(|sample| *sample == 0.25)
    );
    assert!(
        stereo_dry.channels[1][900..]
            .iter()
            .all(|sample| *sample == -0.5)
    );

    // The embedded source is MonoToStereo. Tail samples have no dry component, so the selected
    // stereo layout must excite exactly the equal-power fold of its two input channels.
    let mono_wet = run_config(
        &file,
        Some(0),
        48_000.0,
        256,
        &blocks,
        Signal::Impulse,
        &[(0, Target::Mix, 1.0)],
    )
    .expect("mono wet layout");
    let stereo_wet = run_config(
        &file,
        Some(1),
        48_000.0,
        256,
        &blocks,
        Signal::StereoImpulse([0.25, -0.5]),
        &[(0, Target::Mix, 1.0)],
    )
    .expect("stereo wet layout");
    let ratio = -0.25 / core::f32::consts::SQRT_2;
    let mut compared = 0;
    for channel in 0..2 {
        for index in 1..mono_wet.channels[channel].len() {
            let reference = mono_wet.channels[channel][index];
            if reference.abs() > 1.0e-5 {
                assert!((stereo_wet.channels[channel][index] - reference * ratio).abs() < 2.0e-5);
                compared += 1;
            }
        }
    }
    assert!(
        compared > 10,
        "starter response did not exercise Mono-to-stereo wet routing"
    );

    let mono_diagonal = run_config_interpretation(
        &file,
        Some(0),
        Some("diagonal_stereo"),
        48_000.0,
        256,
        &blocks,
        Signal::Impulse,
        &[(0, Target::Mix, 1.0)],
    )
    .expect("mono diagonal reference");
    let stereo_diagonal = run_config_interpretation(
        &file,
        Some(1),
        Some("diagonal_stereo"),
        48_000.0,
        256,
        &blocks,
        Signal::StereoImpulse([0.25, -0.5]),
        &[(0, Target::Mix, 1.0)],
    )
    .expect("stereo diagonal layout");
    let mut compared = 0;
    for (channel, gain) in [0.25, -0.5].into_iter().enumerate() {
        for index in 1..mono_diagonal.channels[channel].len() {
            let reference = mono_diagonal.channels[channel][index];
            if reference.abs() > 1.0e-5 {
                assert!(
                    (stereo_diagonal.channels[channel][index] - reference * gain).abs() < 2.0e-5
                );
                compared += 1;
            }
        }
    }
    assert!(
        compared > 10,
        "starter response did not exercise diagonal wet routing"
    );
}

#[test]
fn predelay_edit_during_silence_keeps_the_real_wrapper_awake_until_exact_idle() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let blocks = vec![64usize; 1_600];
    let rendered = run(
        &file,
        48_000.0,
        64,
        &blocks,
        Signal::Impulse,
        &[(700, Target::Predelay, 0.025)],
    )
    .expect("predelay automation renders");
    let late_peak = rendered
        .channels
        .iter()
        .flat_map(|channel| channel[800..4_000].iter().enumerate())
        .max_by(|left, right| left.1.abs().total_cmp(&right.1.abs()))
        .map(|(index, sample)| (index + 800, sample.abs()))
        .unwrap();
    assert!(
        late_peak.1 > 1.0e-6,
        "longer tap did not expose retained wet history; late peak {late_peak:?}"
    );
    assert!(
        rendered.statuses[12..390]
            .iter()
            .all(|status| matches!(status, ProcessStatus::Continue)),
        "the wrapper stopped reporting Tail before the edited tap history expired"
    );
    assert!(
        rendered.statuses[420..]
            .iter()
            .all(|status| matches!(status, ProcessStatus::ContinueIfNotQuiet)),
        "the wrapper did not return to exact-idle status after the conservative bound"
    );
    assert!(
        rendered
            .channels
            .iter()
            .all(|channel| channel[99_000..].iter().all(|sample| *sample == 0.0))
    );
}

#[test]
fn hostile_supported_rates_and_every_internal_span_boundary_stay_finite() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let sizes = [1usize, 2, 63, 64, 65, 127, 255, 1024, 4097, 8192];
    for rate in [8_000.0, 12_345.67, 44_100.0, 192_000.0, 384_000.0] {
        let rendered = run(&file, rate, 8192, &sizes, Signal::HostileFinite, &[])
            .unwrap_or_else(|error| panic!("{rate} Hz refused: {error}"));
        assert!(
            rendered
                .channels
                .iter()
                .flatten()
                .all(|sample| sample.is_finite()),
            "non-finite output at {rate} Hz"
        );
    }
}

#[test]
fn rates_outside_response_preparation_activate_with_finite_exact_dry_output() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    // The plugin-local activation and editor proofs distinguish the telemetry reason and visible
    // text. This real-bundle seam proves that rejection applies to the wet response, not the host.
    for rate in [1_000.0, 1_234.57, 7_999.5, 384_000.5, 768_000.0] {
        let rendered = run_config(
            &file,
            Some(1),
            rate,
            64,
            &[1, 2, 63, 64],
            Signal::StereoConstant([0.25, -0.125]),
            &[],
        )
        .unwrap_or_else(|error| panic!("host activation at {rate} Hz failed: {error}"));
        assert_eq!(
            rendered.state_after, rendered.state_before,
            "host-only rate changed canonical state at {rate} Hz"
        );
        assert!(
            rendered.channels[0].iter().all(|sample| *sample == 0.25)
                && rendered.channels[1].iter().all(|sample| *sample == -0.125),
            "the inert response was not exact dry passthrough at {rate} Hz"
        );
        assert!(
            rendered
                .channels
                .iter()
                .flatten()
                .all(|sample| sample.is_finite() && !sample.is_subnormal()),
            "invalid dry output at {rate} Hz"
        );
    }
}

#[test]
fn process_path_is_allocation_free_under_the_debug_bundle_guard() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    // nice-plug's `assert_process_allocs` is enabled workspace-wide. With a debug bundle, any
    // allocation inside Plugin::process aborts this test; release runs retain the finite oracle.
    let sizes = [8192usize, 4097, 65, 64, 63, 2, 1, 8192];
    let events: Vec<_> = (0..48)
        .map(|index| {
            (
                37 + index * 101,
                Target::Mix,
                if index % 2 == 0 { 0.0 } else { 0.73 },
            )
        })
        .collect();
    let rendered = run(
        &file,
        48_000.0,
        8192,
        &sizes,
        Signal::Constant(0.2),
        &events,
    )
    .expect("dense split render");
    assert!(
        rendered
            .channels
            .iter()
            .flatten()
            .all(|sample| sample.is_finite())
    );
}

/// How this loop compares to an effect the owner already approved.
///
/// Renders each reverb twice - its feedback control at zero, then at its maximum - through the real
/// bundles, and reports the energy ratio. Shimmer is the reference: its `Regen` is scaled internally
/// by `OUTER_REGEN_SCALE` 0.15 and filtered at 180 Hz / 8 kHz, the same corners this loop adopted.
#[test]
fn feedback_ballpark_against_shimmer() {
    let Some(convolution) = bundle() else {
        panic!("{SKIP}");
    };
    let shimmer = convolution.with_file_name("mxm-shimmer.clap");
    if !shimmer.exists() {
        eprintln!("skipping: bundle mxm-shimmer first");
        return;
    }

    let measure = |file: &std::path::Path,
                   plugin: &str,
                   control: &str,
                   maximum: f64|
     -> (f64, f64) {
        let render = |value: f64| -> f64 {
            // SAFETY: our own bundle from the build tree.
            let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
            let (_shared, mut instance) =
                harness::instantiate(&entry, plugin).expect("instantiate");
            // Convolution needs a response and a wet mix before it has any tail at all; without
            // them it renders dry only and the post-input window is exactly zero, which is not a
            // statement about Feedback. Shimmer is algorithmic and needs no such setup.
            if plugin == PLUGIN {
                configure_one_tap_feedback(&mut instance, value as f32, true);
            }
            let params = ParamSet::read(&mut instance);
            let id = ClapId::from_raw(
                params
                    .params
                    .iter()
                    .find(|p| p.name == control)
                    .unwrap_or_else(|| panic!("{control} on {plugin}"))
                    .id,
            )
            .unwrap();
            let processor = instance
                .activate(
                    |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
                    PluginAudioConfiguration {
                        sample_rate: 48_000.0,
                        min_frames_count: 64,
                        max_frames_count: 64,
                    },
                )
                .expect("activate");
            let mut processor = processor.start_processing().expect("start");
            let mut late = 0.0f64;
            for block in 0..3_000 {
                let mut input = [0.0f32; 64];
                for (offset, sample) in input.iter_mut().enumerate() {
                    let n = (block * 64 + offset) as f32;
                    *sample = if block < 1_500 {
                        (n * 0.01).sin() * 0.25
                    } else {
                        0.0
                    };
                }
                let mut left = [0.0f32; 64];
                let mut right = [0.0f32; 64];
                let mut ip = AudioPorts::with_capacity(1, 1);
                let mut op = AudioPorts::with_capacity(2, 1);
                let ai = ip.with_input_buffers([AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only([InputChannel::variable(
                        &mut input,
                    )]),
                }]);
                let mut ao = op.with_output_buffers([AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_output_only([&mut left[..], &mut right[..]]),
                }]);
                let mut ev = EventBuffer::new();
                if block == 0 {
                    ev.push(&ParamValueEvent::new(
                        0,
                        id,
                        Pckn::match_all(),
                        value,
                        Cookie::empty(),
                    ));
                }
                let mut out_ev = EventBuffer::new();
                processor
                    .process(
                        &ai,
                        &mut ao,
                        &InputEvents::from_buffer(&ev),
                        &mut OutputEvents::from_buffer(&mut out_ev),
                        Some((block * 64) as u64),
                        None,
                    )
                    .expect("process");
                if block >= 2_000 {
                    for sample in left.iter().chain(right.iter()) {
                        late += f64::from(*sample).powi(2);
                    }
                }
            }
            instance.deactivate(processor.stop_processing());
            late
        };
        (render(0.0), render(maximum))
    };

    let (conv_zero, conv_max) = measure(&convolution, PLUGIN, "Feedback", 1.25);
    let (shim_zero, shim_max) = measure(&shimmer, "dk.mxm.mxm-shimmer", "Regen", 1.0);
    eprintln!(
        "convolution Feedback: 0 -> {conv_zero:e}   max -> {conv_max:e}   ratio {:.2}",
        conv_max / conv_zero.max(f64::MIN_POSITIVE)
    );
    eprintln!(
        "shimmer     Regen   : 0 -> {shim_zero:e}   max -> {shim_max:e}   ratio {:.2}",
        shim_max / shim_zero.max(f64::MIN_POSITIVE)
    );
}

/// The decisive measurement: the owner's own room impulse, through the real staged bundle.
///
/// Every other feedback fixture in this file is either a one-tap `H = [1]`, which cannot cross unity
/// at all, or short enough that the loop never leaves the linear region - and the loop saturator
/// `A * tanh(v / A)` cancels *exactly* at small signal. Those fixtures are therefore structurally
/// blind to the loop ceiling, which is why they moved barely 1% across a four-fold change in it.
/// This one drives the 5.35 s impulse the owner actually heard self-oscillate, so a ceiling change
/// is visible here or it is not real.
#[test]
#[ignore = "tuning instrument: needs the owner's room-IR catalogue on disk"]
fn owner_room_impulse_oscillation_against_the_dry_input() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    // Identify the binary actually under measurement. A stale staged bundle has produced
    // bit-identical "results" across genuinely different constants more than once in this work.
    let meta = std::fs::metadata(&file).expect("bundle metadata");
    eprintln!(
        "bundle: {} bytes, modified {:?}",
        meta.len(),
        meta.modified()
    );

    // Five impulses chosen because they spanned the *old* bound's spread: under an L2 divisor these
    // crossed anywhere from a displayed 0.56 to 2.32, a 4.1x scatter that no single calibration
    // constant could remove. If the per-response bound works, they now all cross near 1.00. Picking
    // the extremes is what makes this falsifiable rather than a demonstration.
    let catalogue = std::env::var_os("MXM_ROOM_IR_CATALOGUE")
        .map(std::path::PathBuf::from)
        .expect("MXM_ROOM_IR_CATALOGUE must name the local owner-IR catalogue");
    let impulses = [
        (
            "concrete-stairwell",
            "concrete-stairwell.far.stereo.wav",
            0.56,
        ),
        (
            "small-echo-chamber",
            "small-echo-chamber.far.stereo.wav",
            1.00,
        ),
        ("walk-in-closet", "walk-in-closet.far.full.wav", 2.32),
    ];

    fn load(path: &std::path::Path) -> Option<(Vec<f32>, u32)> {
        // Left channel only: the injection helper declares a mono interpretation. The ten-million
        // frame test ceiling is above the product's ten-second source limit at every supported rate.
        let decoded = mxm_audio_file_decode::decode_file(
            path,
            &mxm_audio_file_decode::Limits::new(
                10_000_000,
                mxm_audio_file_decode::AtLimit::Refuse,
                mxm_audio_file_decode::Keep::First(1),
            ),
        )
        .ok()?;
        Some((decoded.interleaved, decoded.sample_rate))
    }

    // Long enough for several round trips, because one pass through the loop takes the response's
    // whole length. At 6,000 blocks (8 s) a 9 s impulse never completed a single pass, so a loop
    // above unity had no opportunity to grow and read as "does not oscillate" - the short impulses
    // crossed only because 1.1 s fits five passes into the same window. 40,000 blocks is 53 s, which
    // gives even a 10 s impulse five passes, and the final 13 s is what gets measured.
    const BLOCKS: usize = 40_000;
    const LATE: usize = 28_000;
    let render = |response: &[f32],
                  mix: f32,
                  feedback: f32,
                  drive_blocks: usize|
     -> (f64, f32, f64) {
        // SAFETY: our own bundle from the build tree.
        let entry = unsafe { PluginEntry::load(&file) }.expect("load bundle");
        let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate");
        configure_response_and_params(&mut instance, response, feedback, mix);
        let processor = instance
            .activate(
                |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
                PluginAudioConfiguration {
                    sample_rate: 48_000.0,
                    min_frames_count: 64,
                    max_frames_count: 64,
                },
            )
            .expect("activate");
        let mut processor = processor.start_processing().expect("start");
        let mut energy = 0.0f64;
        let mut peak = 0.0f32;
        let mut early_half = 0.0f64;
        let mut late_half = 0.0f64;
        for block in 0..BLOCKS {
            let mut input = [0.0f32; 64];
            // Exact silence once the drive stops, so what remains is the loop's own output.
            if block < drive_blocks {
                for (offset, sample) in input.iter_mut().enumerate() {
                    let n = (block * 64 + offset) as f32;
                    *sample = (n * 0.01).sin() * 0.25;
                }
            }
            let mut left = [0.0f32; 64];
            let mut right = [0.0f32; 64];
            let mut ip = AudioPorts::with_capacity(1, 1);
            let mut op = AudioPorts::with_capacity(2, 1);
            let ai = ip.with_input_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only([InputChannel::variable(&mut input)]),
            }]);
            let mut ao = op.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only([&mut left[..], &mut right[..]]),
            }]);
            let ev = EventBuffer::new();
            let mut out_ev = EventBuffer::new();
            processor
                .process(
                    &ai,
                    &mut ao,
                    &InputEvents::from_buffer(&ev),
                    &mut OutputEvents::from_buffer(&mut out_ev),
                    Some((block * 64) as u64),
                    None,
                )
                .expect("process");
            if block >= LATE {
                // Two successive halves of the measured region. A loop that merely has *some* tail
                // decays steeply between them; one at or above the edge holds or grows. Comparing
                // against the q=0 tail instead only asks "is anything there", which every nonzero
                // feedback passes on a long impulse - that false detector put the reported edge at
                // q=0.05, where the loop gain is 0.05 and sustain is impossible.
                let half = LATE + (BLOCKS - LATE) / 2;
                for sample in left {
                    peak = peak.max(sample.abs());
                    energy += f64::from(sample).powi(2);
                    if block < half {
                        early_half += f64::from(sample).powi(2);
                    } else {
                        late_half += f64::from(sample).powi(2);
                    }
                }
            }
        }
        // `late_half / early_half` is the sustain ratio: below 1 the tail is still shrinking between
        // the two halves of the measured region, at or above 1 it is holding or growing. This is
        // what distinguishes self-oscillation from "there is some tail", which is all the previous
        // detector tested - and why it reported an edge at q=0.05, where the loop gain is 0.05.
        (energy, peak, late_half / early_half.max(f64::MIN_POSITIVE))
    };

    // Where the dial actually crosses into self-oscillation, per impulse. Drive for 26.7 seconds,
    // then feed exact silence for 26.7 and compare the final 12.8 in two halves: a loop that is
    // merely regenerating decays away, one past the edge holds or grows. The owner's contract is
    // that a displayed 1.00 is that edge, so this locates it rather than assuming it.
    eprintln!("-- crossing per impulse: drive 26.7 s, then silence, compare the final 12.8 s --");
    eprintln!(
        "   (old L2 bound scattered these 0.56 - 2.32; a working per-response bound lands them near 1.00)"
    );
    let mut crossings = Vec::new();
    for (name, leaf, old_edge) in impulses {
        let path = catalogue.join(name).join(leaf);
        let Some((response, rate)) = load(&path) else {
            eprintln!("   {name:<20} skipped: not on this machine");
            continue;
        };
        if rate != 48_000 {
            eprintln!("   {name:<20} skipped: {rate} Hz, injection declares 48 kHz");
            continue;
        }
        // Drive for half the window, then silence. The q grid brackets the contract rather than
        // scanning: with a 53 s render per point a full sweep across five impulses is minutes of
        // wall clock for no extra information. 0.8 must decay and 1.2 must sustain if the edge is
        // where the arithmetic puts it.
        let (_, _quiet_peak, quiet_ratio) = render(&response, 0.38, 0.0, 20_000);
        let mut ratios: Vec<(f32, f64)> = Vec::new();
        let mut crossed = f32::NAN;
        // Report a bracket, not the first rung that happened to cross. Twice now the lowest q in
        // the grid was itself a crossing, which reads as a precise edge but only says the grid
        // started too high - the same artifact as an all-NaN sweep, inverted. `decayed` must end up
        // Some(_) or the reading is meaningless, and that is stated in the output rather than left
        // for the reader to infer.
        let mut decayed: Option<f32> = None;
        for q in [0.9f32, 0.98, 1.0, 1.02, 1.1] {
            let (_, _peak, ratio) = render(&response, 0.38, q, 20_000);
            // Print every ratio, not just the verdict. A threshold picked without seeing the
            // distribution is how the previous detector reported an edge at q=0.05: the numbers
            // have to show a step, or the cut is arbitrary and the crossing is not real.
            ratios.push((q, ratio));
            // Sustain, not presence: the tail must stop shrinking between the two halves.
            // A bounded oscillation is stationary, so finite accumulation and f32 rendering can
            // put its two nominally equal windows a fraction below one. One percent is the stated
            // measurement tolerance; the 0.98 control readings are far below it on all three IRs.
            if ratio >= 0.99 {
                if crossed.is_nan() {
                    crossed = q;
                }
            } else if crossed.is_nan() {
                decayed = Some(q);
            }
        }
        eprintln!(
            "   {name:<20} q=0 reference ratio {quiet_ratio:.4}  |  {}",
            ratios
                .iter()
                .map(|(q, r)| format!("{q:.2}:{r:.3}"))
                .collect::<Vec<_>>()
                .join("  ")
        );
        match (decayed, crossed.is_nan()) {
            (Some(low), false) => {
                eprintln!(
                    "   {name:<20} {} frames  edge between q={low:.2} and q={crossed:.2}  (old bound: {old_edge:.2})",
                    response.len()
                );
                assert!(
                    low <= 1.0 && (0.98..=1.02).contains(&crossed),
                    "{name}: measured edge {low:.2}..{crossed:.2} missed the required 1.00 boundary"
                );
            }
            (None, false) => panic!(
                "{name}: crossing at the lowest probe q={crossed:.2}; the edge is unbracketed"
            ),
            (_, true) => panic!("{name}: no self-oscillation crossing through q=1.10"),
        }
        crossings.push((name, crossed));
    }
    let located: Vec<_> = crossings.iter().filter(|(_, q)| !q.is_nan()).collect();
    if located.len() >= 2 {
        let lo = located.iter().map(|(_, q)| *q).fold(f32::MAX, f32::min);
        let hi = located.iter().map(|(_, q)| *q).fold(0.0f32, f32::max);
        if lo <= 0.05 {
            eprintln!(
                "   spread NOT established: the lowest crossing sits on the grid floor, so this is a probe artifact"
            );
        } else {
            eprintln!(
                "   spread across impulses: {lo:.2} - {hi:.2} ({:.1}x)  (was 0.56 - 2.32, a 4.1x scatter)",
                hi / lo
            );
        }
    }

    let input_peak = 0.25f32;
    let control = impulses[2];
    let control_path = catalogue.join(control.0).join(control.1);
    let Some((response, _)) = load(&control_path) else {
        eprintln!("control impulse absent; ceiling section skipped");
        return;
    };
    let (dry_energy, dry_peak, _) = render(&response, 0.0, 0.0, BLOCKS);
    eprintln!("-- ceiling on {}: continuous drive, last 2 s --", control.0);
    eprintln!("   mix 0.00 q=0.00 (pure dry): peak={dry_peak:.6} energy={dry_energy:.6e}");
    for q in [0.0f32, 1.0, 1.25] {
        let (energy, peak, _) = render(&response, 0.38, q, BLOCKS);
        eprintln!(
            "   q={q:.2}: peak={peak:.6} energy={energy:.6e} vs_input={:.2}x",
            peak / input_peak
        );
    }
}

/// Owner report: "when it starts oscillating I can only hear a sine, and the dry signal is turned
/// down; turning feedback down brings the dry back up."
///
/// The saturator sits only in the feedback return, so the dry path should be untouched by Feedback
/// at any setting. At Mix 0 the output is pure dry, so two renders that differ prove a gain path
/// that should not exist.
#[test]
fn feedback_must_not_touch_the_dry_path() {
    let Some(file) = bundle() else {
        panic!("{SKIP}");
    };
    let response: Vec<f32> = {
        let mut state = 0x1234_5678u32;
        (0..100_000)
            .map(|index| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                noise * (-(index as f32) / 30_000.0).exp()
            })
            .collect()
    };
    let render = |mix: f32, feedback: f32| -> (f64, f32) {
        // SAFETY: our own bundle from the build tree.
        let entry = unsafe { PluginEntry::load(&file) }.expect("load bundle");
        let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate");
        configure_response_and_params(&mut instance, &response, feedback, mix);
        let processor = instance
            .activate(
                |shared, _| mxm_player::host::HostAudioProcessor::new(shared),
                PluginAudioConfiguration {
                    sample_rate: 48_000.0,
                    min_frames_count: 64,
                    max_frames_count: 64,
                },
            )
            .expect("activate");
        let mut processor = processor.start_processing().expect("start");
        let mut energy = 0.0f64;
        let mut peak = 0.0f32;
        for block in 0..3_000 {
            let mut input = [0.0f32; 64];
            for (offset, sample) in input.iter_mut().enumerate() {
                let n = (block * 64 + offset) as f32;
                *sample = (n * 0.01).sin() * 0.25;
            }
            let mut left = [0.0f32; 64];
            let mut right = [0.0f32; 64];
            let mut ip = AudioPorts::with_capacity(1, 1);
            let mut op = AudioPorts::with_capacity(2, 1);
            let ai = ip.with_input_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only([InputChannel::variable(&mut input)]),
            }]);
            let mut ao = op.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only([&mut left[..], &mut right[..]]),
            }]);
            let ev = EventBuffer::new();
            let mut out_ev = EventBuffer::new();
            processor
                .process(
                    &ai,
                    &mut ao,
                    &InputEvents::from_buffer(&ev),
                    &mut OutputEvents::from_buffer(&mut out_ev),
                    Some((block * 64) as u64),
                    None,
                )
                .expect("process");
            if block >= 2_500 {
                for sample in left.iter().chain(right.iter()) {
                    energy += f64::from(*sample).powi(2);
                    peak = peak.max(sample.abs());
                }
            }
        }
        instance.deactivate(processor.stop_processing());
        (energy, peak)
    };

    let (dry_quiet, dry_quiet_peak) = render(0.0, 0.0);
    let (dry_hot, dry_hot_peak) = render(0.0, 1.25);
    eprintln!("MIX 0 (pure dry): feedback 0 -> energy {dry_quiet:e} peak {dry_quiet_peak:e}");
    eprintln!("MIX 0 (pure dry): feedback 1.25 -> energy {dry_hot:e} peak {dry_hot_peak:e}");
    let (mixed_quiet, mixed_quiet_peak) = render(0.38, 0.0);
    let (mixed_hot, mixed_hot_peak) = render(0.38, 1.25);
    eprintln!("MIX 0.38: feedback 0 -> energy {mixed_quiet:e} peak {mixed_quiet_peak:e}");
    eprintln!("MIX 0.38: feedback 1.25 -> energy {mixed_hot:e} peak {mixed_hot_peak:e}");
    assert!(
        (dry_hot - dry_quiet).abs() <= dry_quiet * 1.0e-6,
        "Feedback changed the dry path: {dry_quiet:e} -> {dry_hot:e}"
    );
}
