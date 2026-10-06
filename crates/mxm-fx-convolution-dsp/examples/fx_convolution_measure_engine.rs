//! Local callback-cost probe for the current partition geometry.
//!
//! This is evidence for the machine which runs it, not a portable regression oracle. Pass the
//! processing rate and response sample count used by the plugin's measured scheduling envelope.
//! Both steady state and the actual 20 ms old/new engine transition are measured.
//!
//! ```text
//! cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 8000 80000
//! cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 48000 480000 neutral
//! cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 48000 480000 engaged
//! cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 48000 480000 feedback
//! cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 96000 960000
//! cargo run -p mxm-fx-convolution-dsp --release --example fx_convolution_measure_engine -- 384000 4096
//! ```

use mxm_fx_convolution_dsp::{
    AudioEngine, ControlBlock, InputFrame, LATE_PARTITION_SAMPLES, LiveControls, PreparedResponse,
    ResponseInterpretation, WetEqControls, WetPostControls,
};
use std::time::{Duration, Instant};

const BLOCK: usize = 64;
const TRANSITION_SECONDS: f64 = 0.020;

fn build_engine(
    rate: usize,
    response_len: usize,
    phase: f32,
    wet_post: WetPostControls,
    feedback: f32,
) -> AudioEngine {
    let response: Vec<_> = (0..response_len)
        .map(|index| {
            let time = index as f32 / rate as f32;
            (index as f32 * 0.37 + phase).sin() * (-time * 8.0).exp() * 0.04
        })
        .collect();
    let prepared = PreparedResponse::from_channels(
        ResponseInterpretation::Mono,
        &[response.as_slice()],
        48_000.0,
    )
    .unwrap();
    let mut engine = AudioEngine::new(
        prepared,
        0,
        LiveControls {
            mix: 1.0,
            ..LiveControls::default()
        },
        BLOCK as u64,
    );
    engine.try_enable_wet_post(rate as f32).unwrap();
    engine.set_wet_post_controls_immediate(wet_post);
    engine.set_feedback_immediate(feedback);
    engine
}

fn measure(
    engine: &mut AudioEngine,
    input: &[InputFrame; BLOCK],
    output: &mut [[f32; 2]; BLOCK],
    blocks: usize,
) -> (Duration, Duration) {
    let mut total = Duration::ZERO;
    let mut worst = Duration::ZERO;
    for _ in 0..blocks {
        let start = Instant::now();
        engine
            .process_block(input, output, ControlBlock::<0>::new(BLOCK))
            .unwrap();
        let elapsed = start.elapsed();
        total += elapsed;
        worst = worst.max(elapsed);
    }
    (total, worst)
}

fn main() {
    let rate = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(48_000)
        .clamp(8_000, 384_000);
    let response_len = std::env::args()
        .nth(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(480_000)
        .max(1);
    let response_seconds = response_len as f32 / rate as f32;
    let mode = std::env::args().nth(3).unwrap_or_else(|| "engaged".into());
    let wet_post = if mode == "neutral" {
        WetPostControls::default()
    } else {
        WetPostControls {
            wet_eq: WetEqControls {
                low_cut_hz: 20.0,
                high_cut_hz: 1_000.0,
                tone_db: 6.0,
            },
            modulation: 1.0,
            width: 2.0,
        }
    };
    // q=1.25 is a deliberately uncertified stress coordinate, not the final product maximum.
    let feedback = if mode == "feedback" { 1.25 } else { 0.0 };
    let controls = LiveControls {
        mix: 1.0,
        ..LiveControls::default()
    };
    let mut engine = build_engine(rate, response_len, 0.0, wet_post, feedback);
    let input = [InputFrame::Stereo([0.1, -0.1]); BLOCK];
    let mut output = [[0.0; 2]; BLOCK];
    // Fill the old engine's complete frequency-domain history. The replacement engine remains fresh,
    // exactly as a production candidate prepared off audio and transferred into the transition.
    let warmup_blocks = response_len.div_ceil(BLOCK);
    for _ in 0..warmup_blocks {
        engine
            .process_block(&input, &mut output, ControlBlock::<0>::new(BLOCK))
            .unwrap();
    }

    let steady_blocks = rate * 2 / BLOCK;
    let (steady_total, steady_worst) = measure(&mut engine, &input, &mut output, steady_blocks);

    // Start the replacement one callback before the old late tier's heaviest boundary. State loads
    // can occur at any stream position; measuring only the incidental phase after warmup can miss
    // the old engine's FFT spike during a short transition.
    let processed_samples = (warmup_blocks + steady_blocks) * BLOCK;
    let old_phase = processed_samples % LATE_PARTITION_SAMPLES;
    let target_phase = LATE_PARTITION_SAMPLES - BLOCK;
    let alignment_samples =
        (target_phase + LATE_PARTITION_SAMPLES - old_phase) % LATE_PARTITION_SAMPLES;
    for _ in 0..alignment_samples / BLOCK {
        engine
            .process_block(&input, &mut output, ControlBlock::<0>::new(BLOCK))
            .unwrap();
    }

    let candidate = build_engine(rate, response_len, 0.71, wet_post, feedback);
    let transition_samples = (rate as f64 * TRANSITION_SECONDS).round() as u64;
    assert!(
        engine
            .begin_engine_transition(candidate, controls, transition_samples)
            .is_ok(),
        "fresh engine starts the maximum response transition"
    );
    let feedback_transition_samples = if feedback == 0.0 { 0 } else { 2 * BLOCK };
    let transition_blocks =
        (transition_samples as usize + feedback_transition_samples).div_ceil(BLOCK);
    let (transition_total, transition_worst) =
        measure(&mut engine, &input, &mut output, transition_blocks);
    assert!(
        !engine.transition_active(),
        "the measured transition did not complete"
    );

    let budget = Duration::from_secs_f64(BLOCK as f64 / rate as f64);
    println!(
        "mode={mode}, rate={rate} Hz, response={response_seconds:.6}s ({response_len} samples), stereo output"
    );
    println!(
        "profile={}, os={}, arch={}, FTZ=not queried (explicit seam flushing active)",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    println!(
        "steady: blocks={steady_blocks}, average={:?}, worst={steady_worst:?}, callback budget={budget:?}, worst/budget={:.3}",
        steady_total / steady_blocks as u32,
        steady_worst.as_secs_f64() / budget.as_secs_f64(),
    );
    println!(
        "transition: blocks={transition_blocks}, average={:?}, worst={transition_worst:?}, callback budget={budget:?}, worst/budget={:.3}",
        transition_total / transition_blocks as u32,
        transition_worst.as_secs_f64() / budget.as_secs_f64(),
    );
}
