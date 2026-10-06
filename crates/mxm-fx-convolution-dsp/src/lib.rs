//! Framework-free DSP foundations for `mxm-fx-convolution`.
//!
//! The effect is an original implementation of the public long-FIR method documented in
//! `research:effects/convolution-reverb.md`. No existing convolver implementation was opened.
//! Gardner's zero-delay partitioned method is implemented as a direct onset followed by distributed
//! early/late FFT tiers. Prepared spectra and runtime storage are built off the audio thread;
//! the sample path is allocation-free and independent of host block partitioning.
//!
//! This effect deliberately has no note input, performance sources, envelopes, pulsers, random
//! sources, modulation matrix, or sequencer. A prepared impulse response remains fixed; Revision 6
//! adds one deterministic LFO only inside a bounded post-convolution fractional delay, so engaged
//! movement is linear time-varying without animating coefficients. Revision 7 adds the bounded,
//! response-relative raw-response Feedback loop; Freeze remains absent.

#![forbid(unsafe_code)]

pub mod activity;
pub mod control;
pub mod convolver;
mod delay;
pub mod engine;
mod feedback;
mod fft;
mod post;
pub mod request;
pub mod routing;

pub use activity::{Activity, ActivityTracker, RecursiveActivity, TailBounds};
pub use control::{
    ControlBlock, ControlChange, ControlQueueError, FeedbackControlFrame, FeedbackControlState,
    GateAction, GateTick, LiveControls, MixGate, TimedControl, WetEqControls, WetPostControlFrame,
    WetPostControlState, WetPostControls, WetPostTailConfig,
};
pub use convolver::{
    LATE_PARTITION_SAMPLES, PARTITION_SAMPLES, PrepareError, PreparedResponse, ResponseBounds,
    StereoConvolver,
};
pub use engine::{AudioEngine, ProcessError, RetiredResponse};
pub use feedback::FEEDBACK_SATURATION_SCALE;
pub use post::{
    MODULATION_MAX_DELAY_S, MODULATION_MIN_DELAY_S, MODULATION_RATE_HZ, WET_EQ_SETTLE_S,
    WetPostPathError,
};
pub use request::{RequestId, RequestOrder, RequestOrderError};
pub use routing::{
    InputFrame, ResponseInterpretation, ResponseLayoutError, StereoFrame, dry_output,
    wet_excitation,
};
