//! `mxm-fx-convolution` through MXM Player's ordinary discovery, state and effect-chain paths.
//!
//! Product code is not linked into the host. Build the bundle first with
//! `cargo xtask bundle mxm-fx-convolution --release`; missing artifacts skip with that instruction.

use mxm_player_harness::app_harness;
use mxm_player_harness::harness;

use clack_extensions::audio_ports_config::{AudioPortsConfigBuffer, PluginAudioPortsConfig};
use clack_extensions::state::PluginState;
use clack_host::prelude::*;
use harness::{CHANNELS, Harness, mxm_mono_01};
use mxm_player::engine::Engine;
use mxm_player::engine::editor::{EditorTarget, Ownership};
use mxm_player::events::input::Payload;
use mxm_player::host::HostAudioProcessor;
use mxm_player::params::ParamSet;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

const PLUGIN: &str = "dk.mxm.mxm-fx-convolution";
const SOURCE: &str = "dk.mxm.mxm-mono-01";
const BLOCK: usize = 256;
const BLOCKS: usize = 500;
const SKIP: &str =
    "skipping: run `cargo xtask bundle mxm-fx-convolution --release` and bundle mxm-mono-01";

fn bundle() -> Option<(PathBuf, PathBuf)> {
    let dir = app_harness::any_bundled_dir()?;
    let file = dir.join("mxm-fx-convolution.clap");
    file.exists().then_some((dir, file))
}

#[test]
fn editor_is_advertised_through_the_player_effect_path() {
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    let mut engine = Engine::new();
    engine
        .add_fx(&file, PLUGIN)
        .expect("load convolution effect");
    assert!(
        engine.fx_editor_floating_supported(0),
        "the production editor does not support the player's floating effect path"
    );
}

/// Real lifecycle proof, ignored in ordinary package runs because it creates an OS window. The
/// factory invokes it deliberately after bundling; visual quality and DAW parenting remain manual.
#[test]
#[ignore = "creates a real window; needs a display"]
fn editor_opens_closes_and_reopens_through_the_player() {
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    let mut engine = Engine::new();
    engine
        .add_fx(&file, PLUGIN)
        .expect("load convolution effect");
    for attempt in 1..=2 {
        let state = engine
            .open_fx_editor(0, None)
            .unwrap_or_else(|why| panic!("attempt {attempt}: {}", why.message()));
        assert!(state.open, "attempt {attempt}: editor did not report open");
        assert!(
            matches!(state.owned, Ownership::Unowned(_)),
            "attempt {attempt}: claimed ownership without a host window"
        );
        engine.close_editor_for(EditorTarget::Fx(0));
        assert!(
            !engine.editor_state_for(EditorTarget::Fx(0)).open,
            "attempt {attempt}: close was not acknowledged"
        );
    }
}

fn decode_state(bytes: &[u8]) -> Value {
    let size = u64::from_le_bytes(bytes[..8].try_into().expect("CLAP state length prefix"));
    assert_eq!(size as usize, bytes.len() - 8);
    serde_json::from_slice(&bytes[8..]).expect("uncompressed nice-plug state")
}

fn encode_state(value: &Value) -> Vec<u8> {
    let json = serde_json::to_vec(value).expect("state JSON");
    let mut bytes = Vec::with_capacity(json.len() + 8);
    bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes
}

/// Independent bundle-side oracle for the response identity on either side of the Revision 6
/// Decay/Damping addition. The product crate is deliberately not linked into Player tests.
fn response_fingerprint(response: &Value, revision_six: bool) -> u64 {
    fn add(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    let number = |key: &str| response[key].as_u64().expect("integer response field") as u32;
    let preparation = response["preparation"]
        .as_object()
        .expect("preparation object");
    let prep_u32 = |key: &str| preparation[key].as_u64().expect("32-bit preparation field") as u32;
    let prep_u16 = |key: &str| preparation[key].as_u64().expect("16-bit preparation field") as u16;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    add(&mut hash, &number("schema").to_le_bytes());
    add(&mut hash, &number("sample_rate").to_le_bytes());
    add(&mut hash, &number("frames").to_le_bytes());
    add(
        &mut hash,
        &[
            match response["interpretation"].as_str().expect("interpretation") {
                "mono" => 0,
                "mono_to_stereo" => 1,
                "diagonal_stereo" => 2,
                other => panic!("unexpected interpretation {other}"),
            },
        ],
    );
    add(&mut hash, &prep_u32("onset").to_le_bytes());
    add(&mut hash, &prep_u32("extent").to_le_bytes());
    add(
        &mut hash,
        &[u8::from(
            preparation["reverse"].as_bool().expect("reverse flag"),
        )],
    );
    add(&mut hash, &prep_u16("time_percent").to_le_bytes());
    add(
        &mut hash,
        &[
            match preparation["tail_shape"].as_str().expect("tail shape") {
                "natural" => 0,
                "fade" => 1,
                "swell" => 2,
                "gate" => 3,
                other => panic!("unexpected tail shape {other}"),
            },
        ],
    );
    if revision_six {
        add(&mut hash, &prep_u16("decay_percent").to_le_bytes());
        add(&mut hash, &prep_u16("damping_percent").to_le_bytes());
    }
    for channel in response["channels"].as_array().expect("response channels") {
        add(
            &mut hash,
            channel
                .as_str()
                .expect("encoded response channel")
                .as_bytes(),
        );
        add(&mut hash, &[0xff]);
    }
    hash
}

fn save_state(instance: &mut PluginInstance<mxm_player::host::MxmHost>) -> Vec<u8> {
    let state: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    let mut bytes = Vec::new();
    state
        .save(&mut instance.plugin_handle(), &mut bytes)
        .expect("state saves");
    bytes
}

fn load_state(instance: &mut PluginInstance<mxm_player::host::MxmHost>, bytes: &[u8]) {
    let state: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    let mut input = bytes;
    state
        .load(&mut instance.plugin_handle(), &mut input)
        .expect("state loads");
}

fn state_load_succeeds(
    instance: &mut PluginInstance<mxm_player::host::MxmHost>,
    bytes: &[u8],
) -> bool {
    let state: PluginState = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    let mut input = bytes;
    state
        .load(&mut instance.plugin_handle(), &mut input)
        .is_ok()
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

fn response_with_channels(name: &str, channels: &[Vec<f32>]) -> Value {
    json!({
        "schema": 1,
        "name": name,
        "sample_rate": 48_000,
        "frames": channels[0].len(),
        "interpretation": if channels.len() == 1 { "mono" } else { "mono_to_stereo" },
        "channels": channels.iter().map(|channel| encode_f32_channel(channel)).collect::<Vec<_>>(),
        "preparation": {
            "onset": 0,
            "extent": 0,
            "reverse": false,
            "time_percent": 100,
            "tail_shape": "natural",
            "decay_percent": 100,
            "damping_percent": 0
        }
    })
}

#[test]
fn player_discovers_the_effect_and_its_data_shipped_map() {
    let Some((dir, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };

    let mut app = app_harness::AppHarness::new("convolution-discovery", vec![dir]);
    app.harness.state_mut().rescan();
    app.run();
    let state = app.state();
    let found = state
        .found
        .iter()
        .find(|found| found.id == PLUGIN)
        .expect("ordinary scan discovers the bundle");
    assert!(found.effect, "the player must accept it in an effect slot");
    assert!(
        !found.supported,
        "an input effect is not also an output-only instrument"
    );
    assert!(app.app().control_map().knows_instrument(PLUGIN));

    // Read the real CLAP parameter id, then ask the generic map which id fills the role.
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(&file) }.expect("load bundle");
    let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
    let params = ParamSet::read(&mut instance);
    let mix = params
        .params
        .iter()
        .find(|parameter| parameter.name == "Mix")
        .expect("Mix parameter");
    assert_eq!(
        app.app().control_map().param_for(PLUGIN, "fx.reverb"),
        Some(mix.id)
    );
}

#[test]
fn the_real_bundle_reports_the_eight_stable_engaged_defaults() {
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
    let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
    let params = ParamSet::read(&mut instance);
    assert_eq!(params.params.len(), 9);
    assert_eq!(
        params
            .params
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Mix",
            "Pre-delay",
            "Low cut",
            "High cut",
            "Tone",
            "Width",
            "Modulation",
            "Feedback",
            "Pre-delay sync"
        ]
    );
    for parameter in &params.params {
        assert_eq!(parameter.value, parameter.default, "{}", parameter.name);
        assert!(!parameter.text.is_empty(), "{}", parameter.name);
        assert!(!parameter.is_hidden && !parameter.is_read_only);
    }
    let parameter = |name| {
        params
            .params
            .iter()
            .find(|parameter| parameter.name == name)
            .unwrap_or_else(|| panic!("{name} parameter"))
    };
    assert!((parameter("Mix").default - 0.30).abs() < 1e-6);
    for (name, normalized) in [
        ("Low cut", 0.0),
        ("High cut", 1.0),
        ("Tone", 0.5),
        ("Width", 0.5),
        ("Modulation", 0.0),
        ("Feedback", 0.0),
    ] {
        assert_eq!(parameter(name).default, normalized, "{name}");
    }
    for (name, text) in [
        ("Pre-delay", "12 ms"),
        // Low cut opens at its zero-Hz bottom; High cut opens at its 20-kHz top sentinel.
        ("Low cut", "Open"),
        ("High cut", "Open"),
        ("Tone", "0.0 dB"),
        ("Width", "100 %"),
        ("Modulation", "0 %"),
        ("Feedback", "0.00"),
    ] {
        assert_eq!(parameter(name).text, text, "{name}");
    }
}

#[test]
fn clap_state_round_trips_parameters_response_and_preset_identity_atomically() {
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
    let (shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate effect");

    let defaults = save_state(&mut instance);
    let mut changed = decode_state(&defaults);
    let parameter_state = changed["params"].as_object().expect("parameter map");
    assert_eq!(parameter_state.len(), 9);
    for id in [
        "mix",
        "predelay",
        "lowcut",
        "highcut",
        "tone",
        "width",
        "modulation",
        "feedback",
    ] {
        assert!(parameter_state.contains_key(id), "missing state id {id}");
    }
    assert!(changed["fields"]["response"].is_string());
    assert!(changed["fields"]["preset"].is_string());
    let response = changed["fields"]["response"]
        .as_str()
        .expect("serialized response")
        .to_owned();
    let response_json: Value = serde_json::from_str(&response).expect("response JSON");
    assert_eq!(response_json["schema"], 1);
    assert_eq!(
        response_json["channels"]
            .as_array()
            .expect("coefficient channels")
            .len(),
        2
    );
    assert!(response_json.get("path").is_none());
    assert_eq!(response_json["preparation"]["time_percent"], 100);
    assert_eq!(response_json["preparation"]["decay_percent"], 100);
    assert_eq!(response_json["preparation"]["damping_percent"], 0);

    changed["params"]["mix"] = json!({"f32": 0.0});
    changed["params"]["predelay"] = json!({"f32": 0.45});
    changed["params"]["lowcut"] = json!({"f32": 120.0});
    changed["params"]["highcut"] = json!({"f32": 8000.0});
    changed["params"]["tone"] = json!({"f32": 2.0});
    changed["params"]["width"] = json!({"f32": 0.4});
    changed["params"]["modulation"] = json!({"f32": 0.35});
    changed["params"]["feedback"] = json!({"f32": 1.1});
    load_state(&mut instance, &encode_state(&changed));
    let params = ParamSet::read(&mut instance);
    assert_eq!(
        params.params.iter().find(|p| p.name == "Mix").unwrap().text,
        "0 %"
    );
    assert_eq!(
        params
            .params
            .iter()
            .find(|p| p.name == "Pre-delay")
            .unwrap()
            .text,
        "450 ms"
    );
    for (name, text) in [
        ("Low cut", "120 Hz"),
        ("High cut", "8.0 kHz"),
        ("Tone", "2.0 dB"),
        ("Width", "40 %"),
        ("Modulation", "35 %"),
        ("Feedback", "1.10"),
    ] {
        assert_eq!(
            params
                .params
                .iter()
                .find(|parameter| parameter.name == name)
                .unwrap_or_else(|| panic!("{name} parameter"))
                .text,
            text,
            "{name}"
        );
    }
    let committed = save_state(&mut instance);
    let committed_json = decode_state(&committed);
    assert_eq!(
        committed_json["fields"]["response"].as_str(),
        Some(response.as_str())
    );
    assert_eq!(committed_json["params"], changed["params"]);
    assert_ne!(
        shared.notifications.param_rescan.load(Ordering::Acquire),
        0,
        "state load must invalidate a host's parameter cache"
    );

    // A malformed response and different parameter values are one rejected transaction. The
    // nice-plug hook cannot return an error, so rejection is represented by preserving everything.
    let mut malformed = decode_state(&defaults);
    malformed["params"]["mix"] = json!({"f32": 0.91});
    malformed["fields"]["response"] = Value::String("{not response json".to_owned());
    load_state(&mut instance, &encode_state(&malformed));
    assert_eq!(save_state(&mut instance), committed);
}

#[test]
fn over_budget_state_is_accepted_with_the_applied_size_reading_back_at_the_exact_boundary() {
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
    let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
    let mut requested = decode_state(&save_state(&mut instance));
    let mut response = response_with_channels("Ten second clamp", &[vec![0.0; 480_000]]);
    response["preparation"]["time_percent"] = json!(400);
    requested["fields"]["response"] =
        Value::String(serde_json::to_string(&response).expect("response JSON"));

    assert!(
        state_load_succeeds(&mut instance, &encode_state(&requested)),
        "an ordinary Size overflow must publish its normalized model as information, not reject"
    );
    let committed = decode_state(&save_state(&mut instance));
    let committed_response: Value = serde_json::from_str(
        committed["fields"]["response"]
            .as_str()
            .expect("committed response"),
    )
    .expect("committed response JSON");
    assert_eq!(committed_response["preparation"]["time_percent"], 100);
    let frames = committed_response["frames"].as_u64().unwrap();
    let source_rate = committed_response["sample_rate"].as_u64().unwrap();
    let prepared_count = |size_percent: u64| {
        ((frames as f64 * size_percent as f64 / 100.0) * 48_000.0 / source_rate as f64).round()
            as u64
    };
    assert_eq!(prepared_count(100), 480_000);
    assert!(
        prepared_count(101) > 480_000,
        "one larger Size must exceed the same budget"
    );
    // The exact neutral notice text and the direct Extent/conditional-Onset boundary searches are
    // plugin-local seams with no CLAP representation; their focused tests pin those branches. This
    // real-host assertion pins the externally observable distinction: success plus normalized state,
    // while the existing 384 kHz test below pins genuine refusal.
}

#[test]
fn feedback_zero_matches_the_normalised_bundle_digest() {
    // Recaptured 2026-09-28 when every response was normalised and Wet gain deleted: this fixture's
    // response rises by its normalising gain, 1.6235 (+4.21 dB, mean energy 0.3794 to one), and its
    // old wet gain of 0.75 is gone. Before that, recaptured when old plain-zero Open state began
    // migrating to the real 20 kHz maximum. This remains the Feedback-zero and awkward-partition
    // lock.
    const EXPECTED: u64 = 0xe85c_f766_4f3e_5547;
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
    let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate effect");

    let extension: PluginAudioPortsConfig = instance
        .plugin_shared_handle()
        .get_extension()
        .expect("audio-ports-config extension");
    let mut config_buffer = AudioPortsConfigBuffer::new();
    let stereo = extension
        .get(&mut instance.plugin_handle(), 1, &mut config_buffer)
        .expect("stereo configuration");
    extension
        .select(&mut instance.plugin_handle(), stereo.id)
        .expect("select stereo configuration");

    let mut left_response = vec![0.0; 5_121];
    let mut right_response = vec![0.0; 5_121];
    for (at, left, right) in [
        (0, 0.25, -0.125),
        (63, -0.125, 0.25),
        (64, 0.5, 0.125),
        (4_095, 0.0625, -0.03125),
        (4_096, -0.25, 0.5),
        (5_120, 0.125, 0.0625),
    ] {
        left_response[at] = left;
        right_response[at] = right;
    }
    let mut state = decode_state(&save_state(&mut instance));
    state["version"] = json!("0.1.0");
    state["fields"]["response"] = Value::String(
        serde_json::to_string(&response_with_channels(
            "Revision 6 compatibility",
            &[left_response, right_response],
        ))
        .unwrap(),
    );
    for (id, value) in [
        ("mix", 0.625),
        ("predelay", 17.0 / 48_000.0),
        ("lowcut", 0.0),
        ("highcut", 0.0),
        ("tone", 0.0),
        ("width", 1.0),
        ("modulation", 0.0),
        ("feedback", 0.0),
    ] {
        state["params"][id] = json!({"f32": value});
    }
    load_state(&mut instance, &encode_state(&state));

    let processor = instance
        .activate(
            |shared, _| HostAudioProcessor::new(shared),
            PluginAudioConfiguration {
                sample_rate: 48_000.0,
                min_frames_count: 1,
                max_frames_count: 2_048,
            },
        )
        .expect("activate compatibility fixture");
    let mut processor = processor.start_processing().expect("start fixture");
    let mut digest = 0xcbf2_9ce4_8422_2325u64;
    let chunks = [1usize, 63, 2, 127, 17, 2_048, 31];
    let mut absolute = 0usize;
    let mut chunk = 0usize;
    while absolute < 14_337 {
        let frames = chunks[chunk % chunks.len()].min(14_337 - absolute);
        let mut input_left = vec![0.0f32; frames];
        let mut input_right = vec![0.0f32; frames];
        for (at, left, right) in [(0, 0.5, -0.25), (8_192, -0.25, 0.125)] {
            if (absolute..absolute + frames).contains(&at) {
                input_left[at - absolute] = left;
                input_right[at - absolute] = right;
            }
        }
        let mut output_left = vec![0.0f32; frames];
        let mut output_right = vec![0.0f32; frames];
        let mut input_ports = AudioPorts::with_capacity(2, 1);
        let mut output_ports = AudioPorts::with_capacity(2, 1);
        let audio_inputs = input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only([
                InputChannel::variable(&mut input_left),
                InputChannel::variable(&mut input_right),
            ]),
        }]);
        let mut audio_outputs = output_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only([
                &mut output_left[..],
                &mut output_right[..],
            ]),
        }]);
        let input_events = EventBuffer::new();
        let mut output_events = EventBuffer::new();
        processor
            .process(
                &audio_inputs,
                &mut audio_outputs,
                &InputEvents::from_buffer(&input_events),
                &mut OutputEvents::from_buffer(&mut output_events),
                Some(absolute as u64),
                None,
            )
            .expect("render compatibility fixture");
        for (left, right) in output_left.into_iter().zip(output_right) {
            for sample in [left, right] {
                digest ^= u64::from(sample.to_bits());
                digest = digest.wrapping_mul(0x100_0000_01b3);
            }
        }
        absolute += frames;
        chunk += 1;
    }
    instance.deactivate(processor.stop_processing());
    assert_eq!(digest, EXPECTED, "normalised bundle digest moved");
}

#[test]
fn legacy_loaded_three_id_state_restores_clean_with_the_same_name() {
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
    let (_shared, mut instance) = harness::instantiate(&entry, PLUGIN).expect("instantiate effect");
    let mut legacy = decode_state(&save_state(&mut instance));
    let parameter_map = legacy["params"].as_object_mut().expect("parameter map");
    parameter_map.retain(|id, _| matches!(id.as_str(), "mix" | "wetgain" | "predelay"));
    parameter_map.insert("mix".to_owned(), json!({"f32": 0.25}));
    // Wet gain, deleted on 2026-09-28: a state that still names it loads without it.
    parameter_map.insert("wetgain".to_owned(), json!({"f32": 0.063}));
    parameter_map.insert("predelay".to_owned(), json!({"f32": 0.0}));
    let mut response: Value = serde_json::from_str(
        legacy["fields"]["response"]
            .as_str()
            .expect("serialized response"),
    )
    .expect("response JSON");
    let preparation = response["preparation"]
        .as_object_mut()
        .expect("preparation object");
    preparation.remove("decay_percent");
    preparation.remove("damping_percent");
    let legacy_fingerprint = response_fingerprint(&response, false);
    let legacy_name = "Revision 5 room";
    let legacy_identity = json!({
        "version": 1,
        "loaded": {"name": legacy_name, "origin": "factory"},
        "baseline": {"mix": 0.25, "wetgain": 0.0, "predelay": 0.0},
        "state_fingerprint": legacy_fingerprint
    });
    legacy["fields"]["response"] =
        Value::String(serde_json::to_string(&response).expect("encode legacy response"));
    legacy["fields"]["preset"] =
        Value::String(serde_json::to_string(&legacy_identity).expect("encode legacy identity"));

    load_state(&mut instance, &encode_state(&legacy));
    let params = ParamSet::read(&mut instance);
    assert_eq!(params.params.len(), 9);
    for name in [
        "Low cut",
        "High cut",
        "Tone",
        "Width",
        "Modulation",
        "Feedback",
    ] {
        let parameter = params
            .params
            .iter()
            .find(|parameter| parameter.name == name)
            .unwrap_or_else(|| panic!("{name} parameter"));
        assert_eq!(parameter.value, parameter.default, "{name}");
    }
    let restored = decode_state(&save_state(&mut instance));
    assert_eq!(restored["params"].as_object().unwrap().len(), 9);
    let restored_response: Value = serde_json::from_str(
        restored["fields"]["response"]
            .as_str()
            .expect("restored response"),
    )
    .expect("restored response JSON");
    assert_eq!(restored_response["preparation"]["decay_percent"], 100);
    assert_eq!(restored_response["preparation"]["damping_percent"], 0);
    let restored_identity: Value = serde_json::from_str(
        restored["fields"]["preset"]
            .as_str()
            .expect("restored identity"),
    )
    .expect("restored identity JSON");
    assert_eq!(restored_identity["loaded"]["name"], legacy_name);
    assert_eq!(
        restored_identity["baseline"],
        json!({
            "mix": 0.25,
            "predelay": 0.0,
            "lowcut": 0.0,
            "highcut": 1.0,
            "tone": 0.5,
            "width": 0.5,
            "modulation": 0.0,
            "feedback": 0.0,
            "predelaysync": 0.0
        }),
        "the migrated baseline must equal the restored live values (Clean)"
    );
    assert_eq!(
        restored_identity["state_fingerprint"],
        response_fingerprint(&restored_response, true),
        "the migrated response fingerprint must equal the restored response (Clean)"
    );
}

#[test]
fn active_reactivation_failure_rolls_back_response_parameters_engine_and_live_tail() {
    let Some((_, file)) = bundle() else {
        eprintln!("{SKIP}");
        return;
    };
    // SAFETY: this is our own bundle from the build tree.
    let entry = unsafe { PluginEntry::load(file) }.expect("load bundle");
    let (_subject_shared, mut subject_instance) =
        harness::instantiate(&entry, PLUGIN).expect("instantiate subject effect");
    let (_control_shared, mut control_instance) =
        harness::instantiate(&entry, PLUGIN).expect("instantiate control effect");
    let config = PluginAudioConfiguration {
        sample_rate: 384_000.0,
        min_frames_count: 1,
        max_frames_count: 64,
    };
    let mut subject = subject_instance
        .activate(|shared, _| HostAudioProcessor::new(shared), config)
        .expect("starter response is valid at the maximum rate")
        .start_processing()
        .expect("start subject processing");
    let mut control = control_instance
        .activate(|shared, _| HostAudioProcessor::new(shared), config)
        .expect("starter response is valid at the maximum rate")
        .start_processing()
        .expect("start control processing");

    macro_rules! process_block {
        ($processor:expr, $impulse:expr) => {{
            let mut input = [0.0f32; 64];
            input[0] = if $impulse { 1.0 } else { 0.0 };
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
            let input_events = EventBuffer::new();
            let mut output_events = EventBuffer::new();
            $processor
                .process(
                    &audio_inputs,
                    &mut audio_outputs,
                    &InputEvents::from_buffer(&input_events),
                    &mut OutputEvents::from_buffer(&mut output_events),
                    None,
                    None,
                )
                .expect("process convolution block");
            assert!(output_events.is_empty(), "effect emitted an output event");
            (left, right)
        }};
    }

    assert_eq!(process_block!(subject, true), process_block!(control, true));
    let before = save_state(&mut subject_instance);
    let mut rejected = decode_state(&before);
    rejected["params"]["mix"] = json!({"f32": 0.99});
    rejected["fields"]["preset"] = Value::String(
        serde_json::to_string(&json!({
            "version": 1,
            "loaded": {"name": "Must roll back", "origin": "user"},
            "baseline": {},
            "state_fingerprint": null
        }))
        .expect("encode rejected identity"),
    );
    let mut response: Value = serde_json::from_str(
        rejected["fields"]["response"]
            .as_str()
            .expect("response field"),
    )
    .expect("response JSON");
    response["name"] = json!("Deadline rejection");
    response["sample_rate"] = json!(384_000);
    response["frames"] = json!(384_000);
    response["channels"] = json!(["A".repeat(2_048_000), "A".repeat(2_048_000)]);
    response["preparation"] = json!({
        "onset": 0,
        "extent": 0,
        "reverse": false,
        "time_percent": 100,
        "tail_shape": "natural"
    });
    rejected["fields"]["response"] =
        Value::String(serde_json::to_string(&response).expect("encode response"));
    let bytes = encode_state(&rejected);
    let state: PluginState = subject_instance
        .plugin_shared_handle()
        .get_extension()
        .expect("state extension");
    let mut input = bytes.as_slice();
    assert!(
        state
            .load(&mut subject_instance.plugin_handle(), &mut input)
            .is_err(),
        "a response outside the active deadline envelope must reject reactivation"
    );
    assert_eq!(
        save_state(&mut subject_instance),
        before,
        "an unpreparable unacknowledged field must roll back every parameter and field"
    );

    let mut heard_tail = false;
    for _ in 0..100 {
        let subject_output = process_block!(subject, false);
        let control_output = process_block!(control, false);
        assert_eq!(
            subject_output, control_output,
            "rejection changed the live tail"
        );
        heard_tail |= control_output
            .0
            .iter()
            .chain(&control_output.1)
            .any(|sample| *sample != 0.0);
    }
    assert!(heard_tail, "the control response tail was not observed");

    subject_instance.deactivate(subject.stop_processing());
    control_instance.deactivate(control.stop_processing());
}

#[derive(Copy, Clone)]
enum Act {
    NoteOn,
    NoteOff,
}

fn render(with_effect: bool, bypassed: bool) -> Option<Vec<f32>> {
    let source = mxm_mono_01()?;
    let (_, effect) = bundle()?;
    let chain: Vec<(&Path, &str)> = if with_effect {
        vec![(effect.as_path(), PLUGIN)]
    } else {
        vec![]
    };
    let mut host = Harness::with_fx(&source, SOURCE, 1, &chain).expect("effect chain builds");
    if let Some(effect) = host.fx.first() {
        effect.bypassed.store(bypassed, Ordering::Relaxed);
    }
    let script = [(2, Act::NoteOn), (28, Act::NoteOff)];
    let mut output = Vec::with_capacity(BLOCKS * BLOCK * CHANNELS);
    for block in 0..BLOCKS {
        for (at, action) in script {
            if at != block {
                continue;
            }
            let payload = match action {
                Act::NoteOn => Payload::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 100.0 / 127.0,
                },
                Act::NoteOff => Payload::NoteOff {
                    channel: 0,
                    key: 60,
                    velocity: 0.0,
                },
            };
            assert!(host.push_at(0, 0, payload));
        }
        output.extend_from_slice(host.render(BLOCK));
    }
    host.shutdown();
    Some(output)
}

fn last_sounding_frame(interleaved: &[f32]) -> Option<usize> {
    interleaved
        .chunks(CHANNELS)
        .enumerate()
        .rev()
        .find(|(_, frame)| frame.iter().any(|sample| *sample != 0.0))
        .map(|(frame, _)| frame)
}

#[test]
fn the_player_chain_hears_the_response_tail_and_then_reaches_exact_silence() {
    let Some(dry) = render(false, false) else {
        eprintln!("{SKIP}");
        return;
    };
    let wet = render(true, false).expect("bundle existed above");
    assert_ne!(wet, dry, "the effect must change the real rendered signal");
    let dry_end = last_sounding_frame(&dry).expect("source sounded");
    let wet_end = last_sounding_frame(&wet).expect("effect sounded");
    assert!(
        wet_end > dry_end,
        "wet ended {wet_end}, dry ended {dry_end}"
    );
    assert!(
        wet[wet.len() - 4 * BLOCK * CHANNELS..]
            .iter()
            .all(|sample| *sample == 0.0),
        "the finite response must let the player graph become exactly idle"
    );
    let first = dry
        .chunks(CHANNELS)
        .position(|frame| frame.iter().any(|sample| *sample != 0.0))
        .expect("source sounded");
    assert!(wet[..first * CHANNELS].iter().all(|sample| *sample == 0.0));
}

#[test]
fn player_bypass_restores_the_dry_render_to_the_bit() {
    let Some(dry) = render(false, false) else {
        eprintln!("{SKIP}");
        return;
    };
    let bypassed = render(true, true).expect("bundle existed above");
    assert!(dry.iter().any(|sample| *sample != 0.0));
    assert_eq!(bypassed, dry);
}
