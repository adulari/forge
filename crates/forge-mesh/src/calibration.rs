//! Local outcome calibration: a bounded tie-break from Forge's own success and latency record.

/// Bounded, aggregate runtime evidence supplied by Forge's local outcome ledger.  Static
/// benchmarks remain dominant; this only breaks close calls after a meaningful sample size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeCalibration {
    pub samples: u32,
    pub success_rate: f64,
    pub mean_latency_ms: f64,
}

pub(crate) const CALIBRATION_MIN_SAMPLES: u32 = 20;

pub(crate) fn calibration_adjustment(calibration: Option<&RuntimeCalibration>) -> f64 {
    let Some(c) = calibration.filter(|c| c.samples >= CALIBRATION_MIN_SAMPLES) else {
        return 0.0;
    };
    // Failure-rate evidence carries most of the (still bounded) signal.  Latency only separates
    // otherwise-close choices and is logarithmic so a slow long-context model cannot be punished
    // into the wrong tier.  Result is capped at ±1.25 points against static benchmark scores.
    let reliability = (c.success_rate.clamp(0.0, 1.0) - 0.90) * 5.0;
    let latency = if c.mean_latency_ms.is_finite() && c.mean_latency_ms > 0.0 {
        (4_000.0 / c.mean_latency_ms).ln().clamp(-0.25, 0.25)
    } else {
        0.0
    };
    (reliability + latency).clamp(-1.25, 1.25)
}
