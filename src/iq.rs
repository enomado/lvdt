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

impl Iq {
    pub fn magnitude(self) -> f32 {
        libm::sqrtf((self.i as f32 * self.i as f32) + (self.q as f32 * self.q as f32))
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
}
