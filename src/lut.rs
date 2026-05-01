pub const LUT_LEN: usize = 128;
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
    0, 100, 201, 300, 399, 497, 594, 690, 783, 875, 965, 1052, 1137, 1219, 1299, 1375, 1447, 1517, 1582,
    1644, 1702, 1756, 1805, 1850, 1891, 1927, 1959, 1986, 2008, 2025, 2037, 2045, 2047, 2045, 2037, 2025,
    2008, 1986, 1959, 1927, 1891, 1850, 1805, 1756, 1702, 1644, 1582, 1517, 1447, 1375, 1299, 1219, 1137,
    1052, 965, 875, 783, 690, 594, 497, 399, 300, 201, 100, 0, -100, -201, -300, -399, -497, -594, -690,
    -783, -875, -965, -1052, -1137, -1219, -1299, -1375, -1447, -1517, -1582, -1644, -1702, -1756, -1805,
    -1850, -1891, -1927, -1959, -1986, -2008, -2025, -2037, -2045, -2047, -2045, -2037, -2025, -2008, -1986,
    -1959, -1927, -1891, -1850, -1805, -1756, -1702, -1644, -1582, -1517, -1447, -1375, -1299, -1219, -1137,
    -1052, -965, -875, -783, -690, -594, -497, -399, -300, -201, -100,
];

pub const DAC_SINE_LUT: [u16; LUT_LEN] = [
    2048, 2148, 2249, 2348, 2447, 2545, 2642, 2738, 2831, 2923, 3013, 3100, 3185, 3267, 3347, 3423, 3495,
    3565, 3630, 3692, 3750, 3804, 3853, 3898, 3939, 3975, 4007, 4034, 4056, 4073, 4085, 4093, 4095, 4093,
    4085, 4073, 4056, 4034, 4007, 3975, 3939, 3898, 3853, 3804, 3750, 3692, 3630, 3565, 3495, 3423, 3347,
    3267, 3185, 3100, 3013, 2923, 2831, 2738, 2642, 2545, 2447, 2348, 2249, 2148, 2048, 1948, 1847, 1748,
    1649, 1551, 1454, 1358, 1265, 1173, 1083, 996, 911, 829, 749, 673, 601, 531, 466, 404, 346, 292, 243,
    198, 157, 121, 89, 62, 40, 23, 11, 3, 1, 3, 11, 23, 40, 62, 89, 121, 157, 198, 243, 292, 346, 404, 466,
    531, 601, 673, 749, 829, 911, 996, 1083, 1173, 1265, 1358, 1454, 1551, 1649, 1748, 1847, 1948,
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
        assert_eq!(DAC_SINE_LUT[32], 4095);
        assert_eq!(DAC_SINE_LUT[64], DAC_MID_SCALE);
        assert_eq!(DAC_SINE_LUT[96], 1);
    }

    #[test]
    fn cosine_is_quarter_cycle_shift() {
        assert_eq!(cos_from_sine_index(0), 32);
        assert_eq!(cos_from_sine_index(96), 0);
        assert_eq!(SINE_LUT_I16[cos_from_sine_index(0)], IQ_AMPLITUDE);
    }
}
