//! Revision 6 compatibility lock captured from the unchanged Revision 5 DSP at `fda4f94`.
//!
//! The fixture deliberately uses exact binary fractions and impulses aligned to both partition
//! clocks. That keeps the bit lock independent of libm trigonometric differences while crossing the
//! direct/early/late seams, pre-delay, stereo routing and Mix. Revision 6 neutral defaults and
//! Revision 7 Feedback zero must retain this digest exactly.
//!
//! **Re-derived at wet gain one, 2026-09-28**, when the wet gain was deleted (the owner: responses
//! are normalised and Mix is the one level). Revision 5 at `fda4f94` rendered this fixture at its
//! original wet gain of 0.75 to the old digest, `0x0136_0a5e_6b71_a3f9`, which proved the probe
//! was the same render, and at 1.0 to the digest below. Multiplying by one is exact, so the DSP
//! without a wet gain has to reproduce it bit for bit.

use mxm_fx_convolution_dsp::{
    AudioEngine, ControlBlock, InputFrame, LiveControls, PreparedResponse, ResponseInterpretation,
};

const REVISION_5_DSP_DIGEST: u64 = 0x7a88_488e_9df2_f72d;

fn hash_sample(hash: &mut u64, sample: f32) {
    *hash ^= u64::from(sample.to_bits());
    *hash = hash.wrapping_mul(0x100_0000_01b3);
}

#[test]
fn revision_5_neutral_dsp_render_is_bit_locked() {
    let mut left = vec![0.0; 5_121];
    let mut right = vec![0.0; 5_121];
    for (at, left_gain, right_gain) in [
        (0, 0.25, -0.125),
        (63, -0.125, 0.25),
        (64, 0.5, 0.125),
        (4_095, 0.0625, -0.03125),
        (4_096, -0.25, 0.5),
        (5_120, 0.125, 0.0625),
    ] {
        left[at] = left_gain;
        right[at] = right_gain;
    }
    let response = PreparedResponse::from_channels(
        ResponseInterpretation::MonoToStereo,
        &[left.as_slice(), right.as_slice()],
        48_000.0,
    )
    .unwrap();
    let mut engine = AudioEngine::new(
        response,
        64,
        LiveControls {
            mix: 0.625,
            pre_delay_samples: 17,
        },
        11,
    );

    // Revision 6 allocates its post path, while Revision 7 explicitly selects Feedback zero. Every
    // added control remains on its structural neutral branch.
    engine.try_enable_wet_post(48_000.0).unwrap();
    engine.set_feedback_immediate(0.0);
    assert_eq!(engine.feedback(), 0.0);

    let mut input = vec![InputFrame::Stereo([0.0, 0.0]); 14_337];
    input[0] = InputFrame::Stereo([0.5, -0.25]);
    input[8_192] = InputFrame::Stereo([-0.25, 0.125]);
    let mut output = vec![[0.0; 2]; input.len()];
    let chunks = [1, 63, 2, 127, 17, 2_048, 31];
    let mut offset = 0;
    let mut chunk = 0;
    while offset < input.len() {
        let count = chunks[chunk % chunks.len()].min(input.len() - offset);
        engine
            .process_block(
                &input[offset..offset + count],
                &mut output[offset..offset + count],
                ControlBlock::<0>::new(count),
            )
            .unwrap();
        offset += count;
        chunk += 1;
    }

    let mut digest = 0xcbf2_9ce4_8422_2325;
    for frame in output {
        hash_sample(&mut digest, frame[0]);
        hash_sample(&mut digest, frame[1]);
    }
    assert_eq!(
        digest, REVISION_5_DSP_DIGEST,
        "Revision 5 DSP digest changed: {digest:#018x}"
    );
}
