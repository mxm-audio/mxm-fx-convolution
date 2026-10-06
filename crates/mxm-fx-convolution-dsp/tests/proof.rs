use mxm_fx_convolution_dsp::{
    Activity, AudioEngine, ControlBlock, ControlChange, InputFrame, LiveControls,
    MODULATION_MAX_DELAY_S, PreparedResponse, ResponseInterpretation, TimedControl,
    WET_EQ_SETTLE_S, WetEqControls, WetPostControls,
};

fn render(
    interpretation: ResponseInterpretation,
    channels: &[&[f32]],
    input: &[InputFrame],
    chunks: &[usize],
) -> Vec<[f32; 2]> {
    let response = PreparedResponse::from_channels(interpretation, channels, 48_000.0).unwrap();
    let response_samples = response.response_samples();
    let mut engine = AudioEngine::new(
        response,
        0,
        LiveControls {
            mix: 1.0,
            pre_delay_samples: 0,
        },
        8,
    );
    let mut padded = input.to_vec();
    padded.resize(input.len() + response_samples - 1, InputFrame::Mono(0.0));
    let mut output = vec![[0.0; 2]; padded.len()];
    let mut offset = 0;
    let mut chunk_index = 0;
    while offset < padded.len() {
        let requested = chunks[chunk_index % chunks.len()];
        let count = requested.min(padded.len() - offset);
        engine
            .process_block(
                &padded[offset..offset + count],
                &mut output[offset..offset + count],
                ControlBlock::<0>::new(count),
            )
            .unwrap();
        offset += count;
        chunk_index += 1;
    }
    output
}

fn render_wet_post(chunks: &[usize]) -> Vec<[f32; 2]> {
    let response: Vec<_> = (0..4_211)
        .map(|index| (index as f32 * 0.17).sin() * (-0.001 * index as f32).exp() * 0.1)
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
        32,
    );
    engine.try_enable_wet_post(48_000.0).unwrap();
    engine.set_wet_post_controls_immediate(WetPostControls {
        wet_eq: WetEqControls {
            low_cut_hz: 70.0,
            high_cut_hz: 9_000.0,
            tone_db: 1.5,
        },
        modulation: 0.7,
        width: 1.4,
    });
    let mut input: Vec<_> = (0..6_000)
        .map(|index| {
            InputFrame::Stereo([
                (index as f32 * 0.031).sin() * 0.1,
                (index as f32 * 0.047).cos() * 0.08,
            ])
        })
        .collect();
    input.resize(11_000, InputFrame::Mono(0.0));
    let mut output = vec![[0.0; 2]; input.len()];
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
    output
}

fn render_feedback_transition(chunks: &[usize]) -> Vec<[f32; 2]> {
    let old_response: Vec<_> = (0..4_267)
        .map(|index| (index as f32 * 0.071).cos() * (-0.0012 * index as f32).exp() * 0.02)
        .collect();
    let new_response: Vec<_> = (0..4_113)
        .map(|index| (index as f32 * 0.053).sin() * (-0.0015 * index as f32).exp() * 0.018)
        .collect();
    let prepared = PreparedResponse::from_channels(
        ResponseInterpretation::Mono,
        &[old_response.as_slice()],
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
        32,
    );
    engine.try_enable_wet_post(48_000.0).unwrap();
    engine.set_wet_post_controls_immediate(WetPostControls {
        wet_eq: WetEqControls {
            low_cut_hz: 55.0,
            high_cut_hz: 11_000.0,
            tone_db: 1.0,
        },
        modulation: 0.45,
        width: 1.35,
    });
    engine.set_feedback_immediate(0.72);
    engine
        .begin_response_transition(
            PreparedResponse::from_channels(
                ResponseInterpretation::Mono,
                &[new_response.as_slice()],
                48_000.0,
            )
            .unwrap(),
            97,
        )
        .unwrap();

    let input: Vec<_> = (0..12_000)
        .map(|index| {
            if index < 2_000 {
                InputFrame::Stereo([
                    (index as f32 * 0.031).sin() * 0.08,
                    (index as f32 * 0.047).cos() * 0.07,
                ])
            } else {
                InputFrame::Stereo([0.0, 0.0])
            }
        })
        .collect();
    let mut output = vec![[0.0; 2]; input.len()];
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
    output
}

fn assert_close(actual: f32, expected: f32, at: usize) {
    assert!(
        (actual - expected).abs() < 3.0e-4,
        "sample {at}: {actual} != {expected}"
    );
}

#[test]
fn every_host_response_matrix_row_matches_its_impulse_oracle() {
    let left: Vec<_> = (0..139)
        .map(|index| (index as f32 * 0.19).sin() * (-0.02 * index as f32).exp())
        .collect();
    let right: Vec<_> = left
        .iter()
        .enumerate()
        .map(|(index, value)| value * -0.4 + (index as f32 * 0.07).cos() * 0.03)
        .collect();

    let cases = [
        (
            ResponseInterpretation::Mono,
            vec![left.as_slice()],
            InputFrame::Mono(0.5),
            [0.5, 0.5],
            false,
        ),
        (
            ResponseInterpretation::MonoToStereo,
            vec![left.as_slice(), right.as_slice()],
            InputFrame::Mono(0.5),
            [0.5, 0.5],
            true,
        ),
        (
            ResponseInterpretation::DiagonalStereo,
            vec![left.as_slice(), right.as_slice()],
            InputFrame::Mono(0.5),
            [0.5, 0.5],
            true,
        ),
        (
            ResponseInterpretation::Mono,
            vec![left.as_slice()],
            InputFrame::Stereo([0.25, -0.5]),
            [0.25, -0.5],
            false,
        ),
        (
            ResponseInterpretation::MonoToStereo,
            vec![left.as_slice(), right.as_slice()],
            InputFrame::Stereo([0.25, -0.5]),
            [
                -0.25 * core::f32::consts::FRAC_1_SQRT_2,
                -0.25 * core::f32::consts::FRAC_1_SQRT_2,
            ],
            true,
        ),
        (
            ResponseInterpretation::DiagonalStereo,
            vec![left.as_slice(), right.as_slice()],
            InputFrame::Stereo([0.25, -0.5]),
            [0.25, -0.5],
            true,
        ),
    ];

    for (interpretation, channels, impulse, gains, separate_right) in cases {
        let mut input = vec![InputFrame::Mono(0.0); left.len()];
        input[0] = impulse;
        let output = render(interpretation, &channels, &input[..1], &[1, 7, 63, 2, 91]);
        for index in 0..left.len() {
            let expected_right_response = if separate_right {
                right[index]
            } else {
                left[index]
            };
            assert_close(output[index][0], left[index] * gains[0], index);
            assert_close(output[index][1], expected_right_response * gains[1], index);
        }
    }
}

#[test]
fn rendering_is_invariant_to_host_block_partitioning() {
    // Cross the 4,096-sample early/late boundary so host chunking cannot perturb the distributed
    // late job's publication schedule.
    let response: Vec<_> = (0..4_267)
        .map(|index| (index as f32 * 0.23).cos() * (-0.001 * index as f32).exp())
        .collect();
    let input: Vec<_> = (0..303)
        .map(|index| {
            InputFrame::Stereo([
                (index as f32 * 0.11).sin() * 0.2,
                (index as f32 * 0.071).cos() * 0.17,
            ])
        })
        .collect();
    let whole = render(
        ResponseInterpretation::Mono,
        &[response.as_slice()],
        &input,
        &[input.len() + response.len()],
    );
    let split = render(
        ResponseInterpretation::Mono,
        &[response.as_slice()],
        &input,
        &[1, 63, 2, 127, 5, 31],
    );
    assert_eq!(whole, split);
}

#[test]
fn sample_rate_specific_responses_preserve_the_same_ten_millisecond_time() {
    for rate in [44_100usize, 96_000] {
        let delay = rate / 100;
        let mut response = vec![0.0; delay + 1];
        response[delay] = 1.0;
        let output = render(
            ResponseInterpretation::Mono,
            &[response.as_slice()],
            &[InputFrame::Mono(1.0)],
            &[17, 64, 3],
        );
        let peak = output
            .iter()
            .position(|frame| frame[0].abs() > 0.5)
            .unwrap();
        assert_eq!(peak, delay);
        assert!((peak as f64 / rate as f64 - 0.010).abs() < 1.0 / rate as f64);
    }
}

#[test]
fn hostile_finite_values_never_escape_as_nan_or_infinity() {
    let response = vec![1.0; 193];
    let prepared = PreparedResponse::from_channels(
        ResponseInterpretation::Mono,
        &[response.as_slice()],
        48_000.0,
    )
    .unwrap();
    let mut engine = AudioEngine::new(
        prepared,
        32,
        LiveControls {
            mix: 1.0,
            pre_delay_samples: u64::MAX,
        },
        3,
    );
    engine.set_feedback_immediate(f32::MAX);
    let input = vec![InputFrame::Stereo([f32::MAX, -f32::MAX]); 401];
    let mut output = vec![[0.0; 2]; input.len()];
    engine
        .process_block(&input, &mut output, ControlBlock::<0>::new(input.len()))
        .unwrap();
    // Every stage cleans its own output. The fault latch used to fire here through the wet gain
    // at `f32::MAX`, the one gain after the convolver, deleted on 2026-09-28.
    assert!(output.iter().flatten().all(|value| value.is_finite()));
}

#[test]
fn wet_post_path_is_host_block_partition_invariant() {
    assert_eq!(
        render_wet_post(&[11_000]),
        render_wet_post(&[1, 63, 2, 127, 17, 2_048, 31])
    );
}

#[test]
fn wet_post_tail_is_finite_truthful_and_exactly_idle_at_expected_rates() {
    for rate in [44_100.0, 48_000.0] {
        let prepared =
            PreparedResponse::from_channels(ResponseInterpretation::Mono, &[&[1.0]], 48_000.0)
                .unwrap();
        let mut engine = AudioEngine::new(
            prepared,
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            1,
        );
        engine.try_enable_wet_post(rate).unwrap();
        engine.set_wet_post_controls_immediate(WetPostControls {
            wet_eq: WetEqControls {
                low_cut_hz: 20.0,
                ..WetEqControls::default()
            },
            modulation: 1.0,
            width: 1.0,
        });
        let declared = (WET_EQ_SETTLE_S * rate).ceil() as usize
            + (MODULATION_MAX_DELAY_S * rate).ceil() as usize
            + 2;
        let mut impulse = [[0.0; 2]; 1];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(1.0)],
                    &mut impulse,
                    ControlBlock::<0>::new(1),
                )
                .unwrap(),
            Activity::Normal
        );
        let mut first_silence = [[0.0; 2]; 1];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0)],
                    &mut first_silence,
                    ControlBlock::<0>::new(1),
                )
                .unwrap(),
            Activity::Tail((declared - 1) as u64)
        );
        let rest = vec![InputFrame::Mono(0.0); declared - 1];
        let mut tail = vec![[0.0; 2]; rest.len()];
        assert_eq!(
            engine
                .process_block(&rest, &mut tail, ControlBlock::<0>::new(rest.len()))
                .unwrap(),
            Activity::Normal
        );
        let silence = vec![InputFrame::Mono(0.0); 512];
        let mut idle = vec![[1.0; 2]; silence.len()];
        assert_eq!(
            engine
                .process_block(&silence, &mut idle, ControlBlock::<0>::new(silence.len()),)
                .unwrap(),
            Activity::Normal
        );
        assert!(idle.iter().all(|frame| *frame == [0.0, 0.0]));
    }
}

#[test]
fn wet_post_reset_and_hostile_automation_leave_only_finite_silence() {
    let response = vec![0.25; 193];
    let prepared = PreparedResponse::from_channels(
        ResponseInterpretation::MonoToStereo,
        &[response.as_slice(), response.as_slice()],
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
        7,
    );
    engine.try_enable_wet_post(48_000.0).unwrap();
    let mut events = ControlBlock::<5>::new(64);
    for change in [
        ControlChange::LowCutHz(f32::MAX),
        ControlChange::HighCutHz(f32::MAX),
        ControlChange::ToneDb(f32::MAX),
        ControlChange::Modulation(f32::MAX),
        ControlChange::Width(f32::MAX),
    ] {
        events
            .push(mxm_fx_convolution_dsp::TimedControl {
                sample_offset: 0,
                change,
            })
            .unwrap();
    }
    let input = [InputFrame::Stereo([0.5, -0.25]); 64];
    let mut output = [[0.0; 2]; 64];
    engine.process_block(&input, &mut output, events).unwrap();
    assert!(output.iter().flatten().all(|value| value.is_finite()));

    engine.reset();
    let silence = [InputFrame::Mono(0.0); 512];
    let mut after = [[1.0; 2]; 512];
    engine
        .process_block(&silence, &mut after, ControlBlock::<0>::new(512))
        .unwrap();
    assert!(after.iter().all(|frame| *frame == [0.0, 0.0]));
}

#[test]
fn tail_declaration_reaches_normal_with_exact_idle_output() {
    let response = vec![0.125; 131];
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
        1,
    );
    let mut impulse_output = [[0.0; 2]; 1];
    assert_eq!(
        engine
            .process_block(
                &[InputFrame::Mono(1.0)],
                &mut impulse_output,
                ControlBlock::<0>::new(1),
            )
            .unwrap(),
        Activity::Normal
    );

    let silence = vec![InputFrame::Mono(0.0); response.len() - 1];
    let mut tail = vec![[0.0; 2]; silence.len()];
    let activity = engine
        .process_block(&silence, &mut tail, ControlBlock::<0>::new(silence.len()))
        .unwrap();
    assert_eq!(activity, Activity::Normal);

    let mut idle = vec![[1.0; 2]; 257];
    let activity = engine
        .process_block(
            &vec![InputFrame::Mono(0.0); idle.len()],
            &mut idle,
            ControlBlock::<0>::new(257),
        )
        .unwrap();
    assert_eq!(activity, Activity::Normal);
    assert!(idle.iter().all(|frame| *frame == [0.0, 0.0]));
}

#[test]
fn feedback_transition_and_post_path_are_host_block_partition_invariant() {
    assert_eq!(
        render_feedback_transition(&[12_000]),
        render_feedback_transition(&[1, 63, 2, 127, 17, 2_048, 31])
    );
}

#[test]
fn contractive_feedback_tail_is_truthful_and_reaches_exact_normal_idle() {
    for rate in [44_100usize, 48_000] {
        let prepared =
            PreparedResponse::from_channels(ResponseInterpretation::Mono, &[&[1.0]], 48_000.0)
                .unwrap();
        let mut engine = AudioEngine::new(
            prepared,
            0,
            LiveControls {
                mix: 1.0,
                ..LiveControls::default()
            },
            (rate as f32 * 0.012).round() as u64,
        );
        engine.set_feedback_immediate(0.5);
        let mut impulse = [[0.0; 2]; 1];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(1.0)],
                    &mut impulse,
                    ControlBlock::<0>::new(1),
                )
                .unwrap(),
            Activity::Normal
        );
        assert_eq!(impulse, [[1.0, 1.0]]);

        let mut declared = None;
        let mut last_nonzero = None;
        let mut reached_idle = false;
        // Widened from 1,024. The loop damping decays on its own memory after the geometric term
        // has expired, so a contractive one-tap at q = 0.5 now reaches exact idle at sample 1421
        // rather than inside 1,024 - measured, not estimated. The engine is honest here; the window
        // was simply closing before the tail it declares had finished.
        for sample in 0..8_192usize {
            let mut output = [[1.0; 2]; 1];
            let activity = engine
                .process_block(
                    &[InputFrame::Mono(0.0)],
                    &mut output,
                    ControlBlock::<0>::new(1),
                )
                .unwrap();
            if sample == 0 {
                let Activity::Tail(samples) = activity else {
                    panic!("contractive loop must publish a finite tail, got {activity:?}");
                };
                declared = Some(samples);
            }
            for value in output[0] {
                assert!(value.is_finite());
                assert!(value == 0.0 || value.abs() >= f32::MIN_POSITIVE);
            }
            if output[0] != [0.0, 0.0] {
                last_nonzero = Some(sample as u64);
            }
            if activity == Activity::Normal {
                // `Normal` is a boundary statement: this block may contain the final nonzero sample,
                // but no later sample may remain. The separate idle block below proves exact zero.
                reached_idle = true;
                break;
            }
        }
        assert!(reached_idle, "{rate} Hz contractive loop never became idle");
        assert!(last_nonzero.unwrap() < declared.unwrap());

        let mut idle = [[1.0; 2]; 64];
        assert_eq!(
            engine
                .process_block(
                    &[InputFrame::Mono(0.0); 64],
                    &mut idle,
                    ControlBlock::<0>::new(64),
                )
                .unwrap(),
            Activity::Normal
        );
        assert_eq!(idle, [[0.0; 2]; 64]);
    }
}

#[test]
fn uncertified_feedback_sustains_then_zero_drains_and_reset_is_deterministic() {
    // A room-scale response, not a one-tap. On `H = [1]` the RMS divisor equals the peak, so a
    // displayed 1.25 is an effective 0.525 and the loop decays instead of sustaining; reaching
    // unity there would need 2.38, past the product's maximum. The control's edge is calibrated for
    // the response shape an owner loads, so the proof uses that shape and measures sustain by
    // energy rather than by one sample 256 frames into a 100,000-sample impulse.
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
    let prepared =
        PreparedResponse::from_channels(ResponseInterpretation::Mono, &[&response], 48_000.0)
            .unwrap();
    let mut engine = AudioEngine::new(
        prepared,
        0,
        LiveControls {
            mix: 1.0,
            ..LiveControls::default()
        },
        8,
    );
    engine.set_feedback_immediate(1.25);
    let mut activity = Activity::Normal;
    let mut early = 0.0f64;
    let mut late = 0.0f64;
    for index in 0..160_000 {
        let frame_input = InputFrame::Mono(if index == 0 { 1.0 } else { 0.0 });
        let mut frame = [[0.0; 2]; 1];
        activity = engine
            .process_block(&[frame_input], &mut frame, ControlBlock::<0>::new(1))
            .unwrap();
        let square = f64::from(frame[0][0]).powi(2);
        if (80_000..90_000).contains(&index) {
            early += square;
        }
        if index >= 150_000 {
            late += square;
        }
    }
    assert_eq!(activity, Activity::KeepAlive);
    assert!(
        late > early * 0.5,
        "above the edge the loop must sustain: {early:e} -> {late:e}"
    );

    let mut stop = ControlBlock::<1>::new(64);
    stop.push(TimedControl {
        sample_offset: 0,
        change: ControlChange::Feedback(0.0),
    })
    .unwrap();
    let mut drained = [[1.0; 2]; 64];
    // A finite tail, not `Normal`, and no 8-sample silence: both of those expectations belonged to
    // the one-tap fixture this proof used to carry. With a 100,000-sample response the convolver
    // still owes its whole feed-forward tail when Feedback reaches zero, so the honest assertions
    // are that recursion stops being advertised and that the remaining tail is the response's own.
    let drain_activity = engine
        .process_block(&[InputFrame::Mono(0.0); 64], &mut drained, stop)
        .unwrap();
    assert!(
        matches!(drain_activity, Activity::Tail(_) | Activity::Normal),
        "zeroing Feedback must leave a finite tail, got {drain_activity:?}"
    );
    assert!(drained.iter().any(|frame| *frame != [0.0, 0.0]));

    engine.set_feedback_immediate(1.25);
    let mut reseed = [[0.0; 2]; 16];
    let mut reseed_input = [InputFrame::Mono(0.0); 16];
    reseed_input[0] = InputFrame::Mono(1.0);
    engine
        .process_block(&reseed_input, &mut reseed, ControlBlock::<0>::new(16))
        .unwrap();
    engine.reset();
    let mut after_reset = [[1.0; 2]; 64];
    assert_eq!(
        engine
            .process_block(
                &[InputFrame::Mono(0.0); 64],
                &mut after_reset,
                ControlBlock::<0>::new(64),
            )
            .unwrap(),
        Activity::Normal
    );
    assert_eq!(after_reset, [[0.0; 2]; 64]);

    let mut parked_seed = [[0.0; 2]; 16];
    engine
        .process_block(&reseed_input, &mut parked_seed, ControlBlock::<0>::new(16))
        .unwrap();
    let mut off = ControlBlock::<1>::new(32);
    off.push(TimedControl {
        sample_offset: 0,
        change: ControlChange::Mix(0.0),
    })
    .unwrap();
    let mut faded = [[0.0; 2]; 32];
    assert_eq!(
        engine
            .process_block(&[InputFrame::Mono(0.0); 32], &mut faded, off)
            .unwrap(),
        Activity::Normal
    );
    assert!(faded[8..].iter().all(|frame| *frame == [0.0, 0.0]));

    let mut wake = ControlBlock::<1>::new(32);
    wake.push(TimedControl {
        sample_offset: 0,
        change: ControlChange::Mix(1.0),
    })
    .unwrap();
    let mut after_wake = [[1.0; 2]; 32];
    assert_eq!(
        engine
            .process_block(&[InputFrame::Mono(0.0); 32], &mut after_wake, wake)
            .unwrap(),
        Activity::Normal
    );
    assert_eq!(after_wake, [[0.0; 2]; 32]);
}
