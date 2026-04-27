pub const LUT_LEN: usize = 64;
pub const LUT_QUARTER: usize = LUT_LEN / 4;

pub const ADC_MID_SCALE: i32 = 2048;
pub const DAC_MID_SCALE: u16 = 2048;
pub const DAC_AMPLITUDE: u16 = 2047;
pub const IQ_AMPLITUDE: i16 = 2047;

pub const EXCITATION_HZ: u32 = 2_500;
pub const SAMPLE_HZ: u32 = EXCITATION_HZ * LUT_LEN as u32;

pub const SYSCLK_HZ: u32 = 170_000_000;
pub const TIM6_ARR: u16 = (SYSCLK_HZ / SAMPLE_HZ) as u16 - 1;
pub const ACTUAL_SAMPLE_HZ: f32 = SYSCLK_HZ as f32 / (TIM6_ARR as f32 + 1.0);
pub const ACTUAL_EXCITATION_HZ: f32 = ACTUAL_SAMPLE_HZ / LUT_LEN as f32;

pub const SINE_LUT_I16: [i16; LUT_LEN] = [
    0, 201, 399, 594, 783, 965, 1137, 1298, 1448, 1583, 1703, 1807, 1894, 1962, 2011, 2041, 2047,
    2041, 2011, 1962, 1894, 1807, 1703, 1583, 1448, 1298, 1137, 965, 783, 594, 399, 201, 0, -201,
    -399, -594, -783, -965, -1137, -1298, -1448, -1583, -1703, -1807, -1894, -1962, -2011, -2041,
    -2047, -2041, -2011, -1962, -1894, -1807, -1703, -1583, -1448, -1298, -1137, -965, -783, -594,
    -399, -201,
];

pub const DAC_SINE_LUT: [u16; LUT_LEN] = [
    2048, 2249, 2447, 2642, 2831, 3013, 3185, 3346, 3496, 3631, 3751, 3855, 3942, 4010, 4059, 4089,
    4095, 4089, 4059, 4010, 3942, 3855, 3751, 3631, 3496, 3346, 3185, 3013, 2831, 2642, 2447, 2249,
    2048, 1847, 1649, 1454, 1265, 1083, 911, 750, 600, 465, 345, 241, 154, 86, 37, 7, 1, 7, 37, 86,
    154, 241, 345, 465, 600, 750, 911, 1083, 1265, 1454, 1649, 1847,
];

#[inline]
pub const fn cos_from_sine_index(k: usize) -> usize {
    (k + LUT_QUARTER) & (LUT_LEN - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dac_lut_is_unsigned_midscale_sine() {
        assert_eq!(DAC_SINE_LUT[0], DAC_MID_SCALE);
        assert_eq!(DAC_SINE_LUT[16], 4095);
        assert_eq!(DAC_SINE_LUT[32], DAC_MID_SCALE);
        assert_eq!(DAC_SINE_LUT[48], 1);
    }

    #[test]
    fn cosine_is_quarter_cycle_shift() {
        assert_eq!(cos_from_sine_index(0), 16);
        assert_eq!(cos_from_sine_index(48), 0);
        assert_eq!(SINE_LUT_I16[cos_from_sine_index(0)], IQ_AMPLITUDE);
    }
}
