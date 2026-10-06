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
    peak: AtomicU32,
    tail_samples: AtomicU64,
    clipped: AtomicBool,
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
            peak: AtomicU32::new(0),
            tail_samples: AtomicU64::new(0),
            clipped: AtomicBool::new(false),
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
        let peak = if peak.is_finite() { peak.abs() } else { 0.0 };
        let mut current = self.peak.load(Ordering::Relaxed);
        while f32::from_bits(current) < peak {
            match self.peak.compare_exchange_weak(
                current,
                peak.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(seen) => current = seen,
            }
        }
        self.tail_samples.store(tail_samples, Ordering::Relaxed);
        if clipped || peak > 1.0 {
            self.clipped.store(true, Ordering::Relaxed);
        }
        if numeric_fault {
            self.numeric_fault.store(true, Ordering::Relaxed);
        }
    }

    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }

    pub fn tail_samples(&self) -> u64 {
        self.tail_samples.load(Ordering::Relaxed)
    }

    pub fn clipped(&self) -> bool {
        self.clipped.load(Ordering::Relaxed)
    }

    pub fn clear_clip(&self) {
        self.clipped.store(false, Ordering::Relaxed);
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
    fn peak_is_max_combined_and_reset_on_read() {
        let telemetry = Telemetry::new();
        telemetry.publish(0.4, 9, false, false);
        telemetry.publish(0.8, 7, false, false);
        telemetry.publish(0.2, 5, false, false);
        assert_eq!(telemetry.take_peak(), 0.8);
        assert_eq!(telemetry.take_peak(), 0.0);
        assert_eq!(telemetry.tail_samples(), 5);
    }

    #[test]
    fn clip_and_fault_latch_until_acknowledged() {
        let telemetry = Telemetry::new();
        telemetry.publish(1.2, 0, true, true);
        telemetry.publish(0.1, 0, false, false);
        assert!(telemetry.clipped());
        assert!(telemetry.numeric_fault());
        telemetry.clear_clip();
        telemetry.clear_numeric_fault();
        assert!(!telemetry.clipped());
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
