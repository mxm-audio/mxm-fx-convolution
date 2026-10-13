//! Lock-free, lossy audio-to-editor telemetry. This effect has no MIDI developer channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResponseRejection {
    UnsupportedRate(f32),
    Deadline(f32),
}

impl ResponseRejection {
    pub fn message(self) -> String {
        match self {
            Self::UnsupportedRate(rate) => format!(
                "Off at {rate:.0} Hz: the reverb runs from 8 to 384 kHz. The dry sound passes through."
            ),
            Self::Deadline(rate) => {
                let maximum = crate::response::maximum_processing_samples(rate).unwrap_or_default();
                format!(
                    "Off at {rate:.0} Hz: the response is too long to run here (at most {:.1} s). The dry sound passes through.",
                    maximum as f64 / rate as f64
                )
            }
        }
    }
}

#[derive(Debug)]
pub struct Telemetry {
    /// The output level and its clip latch: the collection's one meter policy
    /// (`mxm_telemetry::PeakMeter`).
    pub meter: mxm_telemetry::PeakMeter,
    tail_samples: AtomicU64,
    numeric_fault: AtomicBool,
    response_rejection: AtomicU32,
    rejected_rate: AtomicU32,
    /// The host tempo in force, so a synced pre-delay reads its division.
    pub tempo: mxm_tempo::TempoCell,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl Telemetry {
    pub const fn new() -> Self {
        Self {
            meter: mxm_telemetry::PeakMeter::new(),
            tail_samples: AtomicU64::new(0),
            numeric_fault: AtomicBool::new(false),
            response_rejection: AtomicU32::new(0),
            rejected_rate: AtomicU32::new(0),
            tempo: mxm_tempo::TempoCell::new(),
        }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Publish once per host callback. Peaks are max-combined so a skipped editor frame cannot hide
    /// a transient; clip and numeric-fault reports latch until explicitly acknowledged.
    pub fn publish(&self, peak: f32, tail_samples: u64, clipped: bool, numeric_fault: bool) {
        self.meter.publish(peak);
        self.tail_samples.store(tail_samples, Ordering::Relaxed);
        if clipped {
            self.meter.latch_clip();
        }
        if numeric_fault {
            self.numeric_fault.store(true, Ordering::Relaxed);
        }
    }

    pub fn tail_samples(&self) -> u64 {
        self.tail_samples.load(Ordering::Relaxed)
    }

    pub fn numeric_fault(&self) -> bool {
        self.numeric_fault.load(Ordering::Relaxed)
    }

    pub fn clear_numeric_fault(&self) {
        self.numeric_fault.store(false, Ordering::Relaxed);
    }

    pub fn reject_response(&self, rejection: ResponseRejection) {
        let (kind, rate) = match rejection {
            ResponseRejection::UnsupportedRate(rate) => (1, rate),
            ResponseRejection::Deadline(rate) => (2, rate),
        };
        self.rejected_rate.store(rate.to_bits(), Ordering::Relaxed);
        self.response_rejection.store(kind, Ordering::Release);
    }

    pub fn clear_response_rejection(&self) {
        self.response_rejection.store(0, Ordering::Release);
    }

    pub fn response_rejection(&self) -> Option<ResponseRejection> {
        let kind = self.response_rejection.load(Ordering::Acquire);
        let rate = f32::from_bits(self.rejected_rate.load(Ordering::Relaxed));
        match kind {
            1 => Some(ResponseRejection::UnsupportedRate(rate)),
            2 => Some(ResponseRejection::Deadline(rate)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_and_fault_latch_until_acknowledged() {
        let telemetry = Telemetry::new();
        telemetry.publish(1.2, 0, true, true);
        telemetry.publish(0.1, 0, false, false);
        assert!(telemetry.meter.clipped());
        assert!(telemetry.numeric_fault());
        telemetry.meter.clear_clip();
        telemetry.clear_numeric_fault();
        assert!(!telemetry.meter.clipped());
        assert!(!telemetry.numeric_fault());
    }

    #[test]
    fn response_rejection_remains_visible_until_supported_preparation_clears_it() {
        let telemetry = Telemetry::new();
        telemetry.reject_response(ResponseRejection::UnsupportedRate(768_000.0));
        let rejection = telemetry.response_rejection().unwrap();
        assert_eq!(rejection, ResponseRejection::UnsupportedRate(768_000.0));
        assert!(rejection.message().contains("runs from 8 to 384 kHz"));
        assert!(
            rejection
                .message()
                .contains("The dry sound passes through.")
        );

        telemetry.publish(0.25, 0, false, false);
        assert_eq!(telemetry.response_rejection(), Some(rejection));
        telemetry.clear_response_rejection();
        assert_eq!(telemetry.response_rejection(), None);
    }
}
