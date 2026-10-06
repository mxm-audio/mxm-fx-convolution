//! Render project-authored convolution examples for the C4 listening gate.
//!
//! ```text
//! cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_render_demo
//! ```
//!
//! Files are generated under `target/convolution-demo/`; no WAV or third-party IR is committed.

use mxm_fx_convolution_dsp::{
    AudioEngine, ControlBlock, InputFrame, LiveControls, PreparedResponse, ResponseInterpretation,
    WetEqControls, WetPostControls,
};

const RATE: usize = 48_000;

fn generated_response(seconds: f32, seed: u32) -> Vec<f32> {
    let len = (seconds * RATE as f32) as usize;
    let mut response = vec![0.0; len.max(1)];
    response[0] = 0.55;
    for (delay_ms, gain) in [(17.0, 0.24), (31.0, -0.17), (47.0, 0.13), (73.0, -0.09)] {
        let at = (delay_ms * RATE as f32 / 1_000.0) as usize;
        if at < response.len() {
            response[at] += gain;
        }
    }
    let mut state = seed;
    for (index, sample) in response.iter_mut().enumerate().skip(1) {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = ((state >> 8) as f32 / 16_777_215.0) * 2.0 - 1.0;
        let time = index as f32 / RATE as f32;
        *sample += noise * (-time * 4.8).exp() * 0.018;
    }
    response
}

fn source(seconds: f32) -> Vec<InputFrame> {
    let len = (seconds * RATE as f32) as usize;
    let mut result = vec![InputFrame::Stereo([0.0, 0.0]); len];
    for beat in 0..8 {
        let start = beat * RATE / 2;
        for index in 0..(RATE / 5) {
            if start + index >= len {
                break;
            }
            let time = index as f32 / RATE as f32;
            let envelope = (-time * 18.0).exp();
            let left = (core::f32::consts::TAU * (180.0 + beat as f32 * 23.0) * time).sin();
            let right = (core::f32::consts::TAU * (270.0 + beat as f32 * 17.0) * time).sin();
            result[start + index] =
                InputFrame::Stereo([left * envelope * 0.22, right * envelope * 0.22]);
        }
    }
    result
}

fn render(
    left: &[f32],
    right: &[f32],
    input: &[InputFrame],
    mix: f32,
    wet_post: WetPostControls,
    feedback: f32,
) -> Vec<[f32; 2]> {
    let prepared = PreparedResponse::from_channels(
        ResponseInterpretation::MonoToStereo,
        &[left, right],
        48_000.0,
    )
    .unwrap();
    let mut engine = AudioEngine::new(
        prepared,
        RATE / 4,
        LiveControls {
            mix,
            pre_delay_samples: (0.020 * RATE as f32) as u64,
        },
        (0.020 * RATE as f32) as u64,
    );
    engine.try_enable_wet_post(RATE as f32).unwrap();
    engine.set_wet_post_controls_immediate(wet_post);
    engine.set_feedback_immediate(feedback);
    let mut padded = input.to_vec();
    padded.resize(input.len() + left.len() + RATE / 4, InputFrame::Mono(0.0));
    let mut output = vec![[0.0; 2]; padded.len()];
    for (input, output) in padded.chunks(127).zip(output.chunks_mut(127)) {
        engine
            .process_block(input, output, ControlBlock::<0>::new(input.len()))
            .unwrap();
    }
    output
}

/// Writes a render as 16-bit PCM through the collection's encoder.
///
/// **A correction, not an equivalence.** The hand-written writer this replaced quantised with
/// `as i16`, which truncates toward zero; the encoder rounds to nearest, so about half the samples
/// move by one LSB, away from zero — the correction `mxm-measure`'s migration measured. Headers,
/// rates and lengths are unchanged, and nothing hashes these files.
fn write_wav(path: &str, audio: &[[f32; 2]]) -> std::io::Result<()> {
    let interleaved: Vec<f32> = audio.iter().flatten().copied().collect();
    mxm_audio_file::write(
        path,
        &interleaved,
        2,
        RATE as u32,
        mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
    )
    .map(|_| ())
    .map_err(std::io::Error::other)
}

fn main() -> std::io::Result<()> {
    let directory = "target/convolution-demo";
    std::fs::create_dir_all(directory)?;
    let input = source(4.0);
    // Chosen short proof response. The playable C4 gate fixes the starter response and accepted
    // source-duration budget after the separate deadline probe has measured them.
    let left = generated_response(0.45, 0x4356_4c31);
    let right = generated_response(0.45, 0x4356_5232);
    write_wav(
        &format!("{directory}/01-generated-room.wav"),
        &render(&left, &right, &input, 0.58, WetPostControls::default(), 0.0),
    )?;

    let mut reverse_left = left.clone();
    let mut reverse_right = right.clone();
    reverse_left.reverse();
    reverse_right.reverse();
    write_wav(
        &format!("{directory}/02-generated-reverse.wav"),
        &render(
            &reverse_left,
            &reverse_right,
            &input,
            0.72,
            WetPostControls::default(),
            0.0,
        ),
    )?;
    write_wav(
        &format!("{directory}/03-shaped-moving-wide.wav"),
        &render(
            &left,
            &right,
            &input,
            0.68,
            WetPostControls {
                wet_eq: WetEqControls {
                    low_cut_hz: 80.0,
                    high_cut_hz: 8_000.0,
                    tone_db: 1.5,
                },
                modulation: 0.65,
                width: 1.5,
            },
            0.0,
        ),
    )?;

    // A sparse project-authored qualifying response makes the above-certificate loop audible
    // without pretending every arbitrary WAV must self-oscillate at the same coordinate.
    let mut feedback_left = vec![0.0; (0.18 * RATE as f32) as usize];
    let mut feedback_right = feedback_left.clone();
    feedback_left[0] = 0.78;
    feedback_right[0] = 0.78;
    feedback_left[(0.007 * RATE as f32) as usize] = 0.15;
    feedback_right[(0.011 * RATE as f32) as usize] = 0.15;
    write_wav(
        &format!("{directory}/04-feedback-recursion.wav"),
        &render(
            &feedback_left,
            &feedback_right,
            &input,
            0.62,
            WetPostControls {
                width: 1.25,
                ..WetPostControls::default()
            },
            1.12,
        ),
    )?;
    println!("Rendered project-authored examples to {directory}");
    Ok(())
}
