//! Versioned transfer and working-space contracts for managed video color.

pub const HDR_COLOR_VERSION: u32 = 1;
pub const REFERENCE_WHITE_NITS: f64 = 203.0;

/// Explicit camera gamut and transfer pairs. The project never guesses these
/// from the codec, sample depth or Rec.709 container fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CameraLog {
    SonySLog3Cine,
    SonySLog3,
    SonySLog2,
}

impl CameraLog {
    pub const OPTIONS: [Self; 3] = [Self::SonySLog3Cine, Self::SonySLog3, Self::SonySLog2];

    pub fn label(self) -> &'static str {
        match self {
            Self::SonySLog3Cine => "Sony S-Log3 / S-Gamut3.Cine",
            Self::SonySLog3 => "Sony S-Log3 / S-Gamut3",
            Self::SonySLog2 => "Sony S-Log2 / S-Gamut",
        }
    }

    pub fn index(self) -> usize { self as usize + 1 }
}

const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 32.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 128.0;
const PQ_C3: f64 = 2392.0 / 128.0;
const HLG_A: f64 = 0.17883277;
const HLG_B: f64 = 1.0 - 4.0 * HLG_A;
const HLG_C: f64 = 0.55991073;

/// ST 2084 electro-optical transfer. A code value of one represents 10,000 cd/m².
pub fn pq_code_to_nits(code: f64) -> f64 {
    let v = code.clamp(0.0, 1.0).powf(1.0 / PQ_M2);
    10_000.0 * ((v - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * v)).powf(1.0 / PQ_M1)
}

/// ST 2084 inverse electro-optical transfer for an absolute luminance.
pub fn nits_to_pq_code(nits: f64) -> f64 {
    let v = (nits.clamp(0.0, 10_000.0) / 10_000.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * v) / (1.0 + PQ_C3 * v)).powf(PQ_M2)
}

/// Inverse HLG OETF. This stage returns relative scene light before the OOTF.
pub fn hlg_code_to_scene(code: f64) -> f64 {
    let code = code.clamp(0.0, 1.0);
    if code <= 0.5 {
        code * code / 3.0
    } else {
        (((code - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

/// Sony S-Log3 inverse curve for a normalized 10-bit code value. Gamut is separate.
pub fn slog3_code_to_scene(code: f64) -> f64 {
    let sample = code * 1023.0;
    if sample >= 171.2102946929 {
        0.19 * 10.0_f64.powf((sample - 420.0) / 261.5) - 0.01
    } else {
        (sample - 95.0) * 0.01125 / (171.2102946929 - 95.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!((actual - expected).abs() < tolerance, "expected {expected}, got {actual}");
    }

    #[test]
    fn pq_reference_points_are_absolute_and_reversible() {
        for (code, nits) in [(0.5080784215, 100.0), (0.5806888810, 203.0), (0.7518270962, 1000.0)] {
            close(pq_code_to_nits(code), nits, 0.0001);
            close(nits_to_pq_code(nits), code, 0.000001);
        }
        assert_eq!(pq_code_to_nits(-1.0), 0.0);
        close(pq_code_to_nits(3.0), 10_000.0, 0.0001);
        assert_eq!(nits_to_pq_code(20_000.0), 1.0);
        assert_eq!(nits_to_pq_code(-1.0), nits_to_pq_code(0.0));
    }

    #[test]
    fn hlg_knee_and_sony_exposure_anchors() {
        assert_eq!(hlg_code_to_scene(-0.1), 0.0);
        close(hlg_code_to_scene(0.5), 1.0 / 12.0, 1e-10);
        close(hlg_code_to_scene(0.5 + 1e-8), 1.0 / 12.0, 1e-7);
        close(hlg_code_to_scene(2.0), 1.0, 1e-7);
        close(slog3_code_to_scene(95.0 / 1023.0), 0.0, 1e-12);
        close(slog3_code_to_scene(420.0 / 1023.0), 0.18, 1e-12);
        assert!(slog3_code_to_scene(64.0 / 1023.0) < 0.0);
    }
}
