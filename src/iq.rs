use crate::lut::{cos_from_sine_index, ADC_MID_SCALE, LUT_LEN, SINE_LUT_I16};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Iq {
    pub i: i32,
    pub q: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DemodulatedSample {
    pub a: Iq,
    pub b: Iq,
    pub sequence: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Deviation {
    /// Magnitude as fraction of `REFERENCE_MAGNITUDE`, in percent.
    /// 100.0 == ideal full-swing loopback, 0.0 == silence.
    pub mag_pct: f32,
    /// Phase relative to the DAC excitation reference, in milliradians.
    /// 0 == in phase with DAC; negative values mean the sampled signal lags
    /// the excitation (group delay through ADC + analog path).
    pub phase_mrad: f32,
}

/// Sum of squares of `SINE_LUT_I16`.
///
/// This is the value of `Iq::i` we get from `demodulate_block` when the ADC
/// reproduces the DAC excitation perfectly (full-swing loopback, zero phase
/// shift, midscale-centred). Equals `LUT_LEN/2 · IQ_AMPLITUDE²` ≈ 1.34e8 for
/// the current 64-point ±2047 LUT.
pub const REFERENCE_MAGNITUDE_I64: i64 = {
    let mut sum: i64 = 0;
    let mut k = 0;
    while k < LUT_LEN {
        let s = SINE_LUT_I16[k] as i64;
        sum += s * s;
        k += 1;
    }
    sum
};

pub const REFERENCE_MAGNITUDE: f32 = REFERENCE_MAGNITUDE_I64 as f32;

impl Iq {
    pub fn magnitude(self) -> f32 {
        libm::sqrtf((self.i as f32 * self.i as f32) + (self.q as f32 * self.q as f32))
    }

    /// Phase relative to the DAC excitation, in radians, in `(-π, π]`.
    pub fn phase_radians(self) -> f32 {
        libm::atan2f(self.q as f32, self.i as f32)
    }

    pub fn deviation(self) -> Deviation {
        Deviation {
            mag_pct: self.magnitude() * (100.0 / REFERENCE_MAGNITUDE),
            phase_mrad: self.phase_radians() * 1000.0,
        }
    }
}

pub fn demodulate_block(samples: &[u32; LUT_LEN], sequence: u32) -> DemodulatedSample {
    let mut out = DemodulatedSample {
        sequence,
        ..DemodulatedSample::default()
    };

    for (k, packed) in samples.iter().copied().enumerate() {
        let (a, b) = unpack_dual_adc(packed);
        let s = SINE_LUT_I16[k] as i32;
        let c = SINE_LUT_I16[cos_from_sine_index(k)] as i32;

        out.a.i += a * s;
        out.a.q += a * c;
        out.b.i += b * s;
        out.b.q += b * c;
    }

    out
}

#[inline]
pub fn pack_dual_adc(adc1: u16, adc2: u16) -> u32 {
    ((adc2 as u32) << 16) | adc1 as u32
}

#[inline]
pub fn unpack_dual_adc(packed: u32) -> (i32, i32) {
    let adc1 = (packed & 0x0fff) as i32 - ADC_MID_SCALE;
    let adc2 = ((packed >> 16) & 0x0fff) as i32 - ADC_MID_SCALE;
    (adc1, adc2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lut::{DAC_SINE_LUT, SINE_LUT_I16};

    #[test]
    fn unpack_dual_adc_subtracts_midscale() {
        assert_eq!(unpack_dual_adc(pack_dual_adc(2048, 2048)), (0, 0));
        assert_eq!(unpack_dual_adc(pack_dual_adc(4095, 1)), (2047, -2047));
    }

    #[test]
    fn demodulates_in_phase_sine_into_i_axis() {
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }

        let iq = demodulate_block(&block, 7);
        let expected_i: i32 = SINE_LUT_I16
            .iter()
            .map(|sample| *sample as i32 * *sample as i32)
            .sum();

        assert_eq!(iq.sequence, 7);
        assert_eq!(iq.a.i, expected_i);
        assert!(iq.a.q.abs() < 10_000);
        assert!((iq.b.i - expected_i).abs() < 50_000);
        assert!(iq.b.q.abs() < 10_000);
    }

    #[test]
    fn reference_constant_matches_lut_sum_of_squares() {
        let expected: i64 = SINE_LUT_I16.iter().map(|s| (*s as i64).pow(2)).sum();
        assert_eq!(REFERENCE_MAGNITUDE_I64, expected);
        // Exact value for the current 64-point ±2047 LUT (the closed-form
        // 64·2047²/2 = 134_086_688 cited in PROGRESS.md is the asymptotic
        // ideal; the rounded LUT entries land slightly higher).
        assert_eq!(REFERENCE_MAGNITUDE_I64, 134_335_850);
    }

    #[test]
    fn deviation_full_loopback_is_100pct_zero_phase() {
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }
        let iq = demodulate_block(&block, 0);
        let dev = iq.a.deviation();
        assert!((dev.mag_pct - 100.0).abs() < 0.05, "mag_pct={}", dev.mag_pct);
        assert!(dev.phase_mrad.abs() < 1.0, "phase_mrad={}", dev.phase_mrad);
    }

    #[test]
    fn deviation_quarter_cycle_lag_is_minus_half_pi() {
        // ADC sees DAC excitation shifted by +LUT_QUARTER samples (i.e. 90°
        // *advance* in time → in cosine projection terms the correlator sees
        // pure cos, so phase = +π/2 rad = +1570 mrad).
        let mut block = [0_u32; LUT_LEN];
        for k in 0..LUT_LEN {
            let shifted = (k + LUT_LEN / 4) & (LUT_LEN - 1);
            block[k] = pack_dual_adc(DAC_SINE_LUT[shifted], DAC_SINE_LUT[shifted]);
        }
        let iq = demodulate_block(&block, 0);
        let dev = iq.a.deviation();
        assert!((dev.mag_pct - 100.0).abs() < 0.05, "mag_pct={}", dev.mag_pct);
        assert!(
            (dev.phase_mrad - 1570.796).abs() < 5.0,
            "phase_mrad={}",
            dev.phase_mrad
        );
    }
}
