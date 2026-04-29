use crate::lut::{cos_from_sine_index, ADC_MID_SCALE, IQ_AMPLITUDE, LUT_LEN, SINE_LUT_I16};

/// Допуск к рельсам ADC: значение `<= RAIL_MARGIN` или `>= 4095 − RAIL_MARGIN`
/// считается «прижатым к рельсе» и инкрементит `sat_count`. 2 LSB шире
/// типового шумового пола ADC, чтобы случайный шум на high‑Z пине не давал
/// false‑positive, и при этом любое реальное full‑scale насыщение поймается.
pub const RAIL_MARGIN: u16 = 2;

const ADC_FULL_SCALE: u16 = 4095;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Iq {
    pub i: i32,
    pub q: i32,
}

/// Сопутствующая статистика канала за тот же блок, что и `Iq`. Считается
/// одним проходом в `demodulate_block` и используется `channel_quality` для
/// детекции клиппинга / потери сигнала / гармонических искажений.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChannelStats {
    /// Σ|x_centered| по блоку — линейный детектор амплитуды.
    pub abs_sum: i32,
    /// Σx_centered² — total energy блока. Сравнивается с fund_energy из
    /// (I²+Q²) по теореме Парсеваля: для чистого синуса должно совпадать,
    /// расхождение = энергия гармоник + шум.
    pub sq_sum: i32,
    /// Сколько raw‑отсчётов в блоке коснулись рельс ADC. Любой ненулевой —
    /// клиппинг где‑то в тракте до ADC.
    pub sat_count: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DemodulatedSample {
    pub a: Iq,
    pub b: Iq,
    pub stats_a: ChannelStats,
    pub stats_b: ChannelStats,
    pub sequence: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Quality {
    /// Хотя бы один отсчёт коснулся рельсы ADC: амплитуда вылетела за
    /// full‑scale, IQ‑magnitude уже не пропорциональна реальной.
    pub clipping: bool,
    /// |Iq| < 1% от `REFERENCE_MAGNITUDE`: сигнал пропал (обрыв вторички,
    /// разъём, отсутствие возбуждения). Distortion‑метрика на таком уровне
    /// уже бессмысленна — её прячем приоритетом.
    pub low_signal: bool,
    /// >2% полной энергии блока лежит вне основной гармоники. Мягкий
    /// клиппинг, нелинейность фронтенда, посторонняя помеха или повреждённый
    /// датчик — не отличает между ними, но фиксирует, что синус «не синус».
    pub distortion: bool,
}

impl Quality {
    /// Один символ под канал: `S`aturated > `L`ow > `H`armonic > `.` ok.
    /// Приоритет от грубой ошибки к тонкой: клиппинг прячет honest harmonics,
    /// low‑signal делает distortion бессмысленной.
    pub fn symbol(self) -> char {
        if self.clipping {
            'S'
        } else if self.low_signal {
            'L'
        } else if self.distortion {
            'H'
        } else {
            '.'
        }
    }

    /// То же, что `symbol`, но как `&'static str` — для `defmt::info!("{=str}")`,
    /// который не умеет `{=char}` без extra cost.
    pub fn symbol_str(self) -> &'static str {
        if self.clipping {
            "S"
        } else if self.low_signal {
            "L"
        } else if self.distortion {
            "H"
        } else {
            "."
        }
    }

    pub fn ok(self) -> bool {
        !self.clipping && !self.low_signal && !self.distortion
    }
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

/// Когерентный накопитель I/Q по N подряд идущим блокам.
///
/// `LUT_LEN` сэмплов в одном DMA‑блоке = ровно один период возбуждения, так
/// что фаза LUT в начале каждого блока одна и та же. Сложение I/Q между
/// блоками это эквивалент демодуляции единого окна длиной N·LUT_LEN с тем же
/// LUT, повторённым N раз — то есть когерентное усреднение, сужающее полосу
/// детектора в N раз. После N блоков `drain_average` возвращает усреднённую
/// `DemodulatedSample` (в той же шкале, что и одиночный блок) и сбрасывается.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Accumulator {
    a_i: i64,
    a_q: i64,
    b_i: i64,
    b_q: i64,
    a_abs: i64,
    a_sq: i64,
    b_abs: i64,
    b_sq: i64,
    a_sat: u32,
    b_sat: u32,
    count: u32,
}

impl Accumulator {
    pub const fn new() -> Self {
        Self {
            a_i: 0,
            a_q: 0,
            b_i: 0,
            b_q: 0,
            a_abs: 0,
            a_sq: 0,
            b_abs: 0,
            b_sq: 0,
            a_sat: 0,
            b_sat: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, sample: &DemodulatedSample) {
        self.a_i += sample.a.i as i64;
        self.a_q += sample.a.q as i64;
        self.b_i += sample.b.i as i64;
        self.b_q += sample.b.q as i64;
        self.a_abs += sample.stats_a.abs_sum as i64;
        self.a_sq += sample.stats_a.sq_sum as i64;
        self.b_abs += sample.stats_b.abs_sum as i64;
        self.b_sq += sample.stats_b.sq_sum as i64;
        // sat_count в окне — это total touch'ей по всем блокам, без деления
        // на N. Любой одиночный block с клиппингом флагнет всё окно.
        self.a_sat = self.a_sat.saturating_add(sample.stats_a.sat_count as u32);
        self.b_sat = self.b_sat.saturating_add(sample.stats_b.sat_count as u32);
        self.count = self.count.wrapping_add(1);
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn drain_average(&mut self, sequence: u32) -> DemodulatedSample {
        let n = self.count.max(1) as i64;
        let out = DemodulatedSample {
            a: Iq { i: (self.a_i / n) as i32, q: (self.a_q / n) as i32 },
            b: Iq { i: (self.b_i / n) as i32, q: (self.b_q / n) as i32 },
            stats_a: ChannelStats {
                abs_sum: (self.a_abs / n) as i32,
                sq_sum: (self.a_sq / n) as i32,
                sat_count: self.a_sat.min(u16::MAX as u32) as u16,
            },
            stats_b: ChannelStats {
                abs_sum: (self.b_abs / n) as i32,
                sq_sum: (self.b_sq / n) as i32,
                sat_count: self.b_sat.min(u16::MAX as u32) as u16,
            },
            sequence,
        };
        *self = Self::new();
        out
    }
}

#[inline]
fn touches_rail(raw: u16) -> bool {
    raw <= RAIL_MARGIN || raw >= ADC_FULL_SCALE - RAIL_MARGIN
}

pub fn demodulate_block(samples: &[u32; LUT_LEN], sequence: u32) -> DemodulatedSample {
    let mut out = DemodulatedSample {
        sequence,
        ..DemodulatedSample::default()
    };

    for (k, packed) in samples.iter().copied().enumerate() {
        let raw_a = (packed & 0x0fff) as u16;
        let raw_b = ((packed >> 16) & 0x0fff) as u16;
        let a = raw_a as i32 - ADC_MID_SCALE;
        let b = raw_b as i32 - ADC_MID_SCALE;
        let s = SINE_LUT_I16[k] as i32;
        let c = SINE_LUT_I16[cos_from_sine_index(k)] as i32;

        out.a.i += a * s;
        out.a.q += a * c;
        out.b.i += b * s;
        out.b.q += b * c;

        out.stats_a.abs_sum += a.unsigned_abs() as i32;
        out.stats_a.sq_sum += a * a;
        out.stats_b.abs_sum += b.unsigned_abs() as i32;
        out.stats_b.sq_sum += b * b;
        if touches_rail(raw_a) {
            out.stats_a.sat_count += 1;
        }
        if touches_rail(raw_b) {
            out.stats_b.sat_count += 1;
        }
    }

    out
}

/// Quality для одного канала. `iq` и `stats` берутся из одной пары
/// `DemodulatedSample::{a,stats_a}` или `{b,stats_b}`. Работает и на
/// одиночном блоке, и на выходе `Accumulator::drain_average` — все три
/// метрики масштабно‑инвариантны относительно деления на N.
pub fn channel_quality(iq: Iq, stats: ChannelStats) -> Quality {
    let i = iq.i as i64;
    let q = iq.q as i64;
    let mag_sq = i * i + q * q;

    let clipping = stats.sat_count > 0;

    // |Iq| < 1% REFERENCE_MAGNITUDE  ⇔  mag_sq · 100² < REFERENCE_MAGNITUDE²
    // Делим, чтобы не переполнить i64: REFERENCE² ≈ 1.8e16 спокойно влезает.
    let ref_sq = REFERENCE_MAGNITUDE_I64 * REFERENCE_MAGNITUDE_I64;
    let low_signal = mag_sq < ref_sq / 10_000;

    // По Парсевалю: для чистого синуса Σx² == fund_energy. Любой избыток
    // total над fund — энергия гармоник (или шума, что в нашем SNR пренебрежимо).
    // fund_energy = mag_sq / (IQ_AMPLITUDE² · LUT_LEN/2).
    let lut_amp_sq = (IQ_AMPLITUDE as i64) * (IQ_AMPLITUDE as i64);
    let denom = lut_amp_sq * (LUT_LEN as i64 / 2);
    let fund_energy = mag_sq / denom;
    let total_energy = stats.sq_sum as i64;
    // Порог 2% — мимо него проходит ADC noise + LUT rounding (<<1%), но
    // ловится 14% harmonic amplitude (3rd на ~14% даёт ~2% по энергии).
    let distortion = !low_signal
        && total_energy > 0
        && fund_energy < total_energy
        && (total_energy - fund_energy) * 50 > total_energy;

    Quality {
        clipping,
        low_signal,
        distortion,
    }
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
    fn accumulator_average_matches_single_block() {
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }
        let single = demodulate_block(&block, 0);

        let mut acc = Accumulator::new();
        for _ in 0..64 {
            acc.push(&single);
        }
        assert_eq!(acc.count(), 64);

        let avg = acc.drain_average(7);
        assert_eq!(acc.count(), 0);
        assert_eq!(avg.sequence, 7);
        // Усреднённый I/Q должен совпасть с одиночным блоком (округление ≤1 LSB).
        assert!((avg.a.i - single.a.i).abs() <= 1);
        assert!((avg.a.q - single.a.q).abs() <= 1);
        assert!((avg.b.i - single.b.i).abs() <= 1);
        assert!((avg.b.q - single.b.q).abs() <= 1);
        let dev = avg.a.deviation();
        assert!((dev.mag_pct - 100.0).abs() < 0.05, "mag_pct={}", dev.mag_pct);
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
    fn quality_clean_loopback_is_ok() {
        // 90% full-scale: peaks ≈ ±1842 → не касаются рельс ±2 LSB.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            let centered = (SINE_LUT_I16[k] as i32 * 9 / 10 + ADC_MID_SCALE) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        let demod = demodulate_block(&block, 0);
        let qa = channel_quality(demod.a, demod.stats_a);
        let qb = channel_quality(demod.b, demod.stats_b);
        assert!(qa.ok(), "qa={:?} stats={:?}", qa, demod.stats_a);
        assert!(qb.ok(), "qb={:?} stats={:?}", qb, demod.stats_b);
        assert_eq!(qa.symbol(), '.');
        assert_eq!(qa.symbol_str(), ".");
    }

    #[test]
    fn quality_silence_flags_low_signal() {
        // Постоянный midscale ⇒ centered=0 повсюду ⇒ I=Q=0, total_energy=0.
        let block = [pack_dual_adc(ADC_MID_SCALE as u16, ADC_MID_SCALE as u16); LUT_LEN];
        let demod = demodulate_block(&block, 0);
        let q = channel_quality(demod.a, demod.stats_a);
        assert!(q.low_signal, "stats={:?}", demod.stats_a);
        assert!(!q.clipping);
        assert!(!q.distortion); // total_energy=0 → distortion guard срабатывает
        assert_eq!(q.symbol(), 'L');
    }

    #[test]
    fn quality_full_swing_dac_lut_flags_clipping() {
        // DAC_SINE_LUT доходит до 4095 и 1 — рельсы ADC. sat_count >= 2.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }
        let demod = demodulate_block(&block, 0);
        let q = channel_quality(demod.a, demod.stats_a);
        assert!(q.clipping, "stats={:?}", demod.stats_a);
        assert!(demod.stats_a.sat_count >= 2);
        assert_eq!(q.symbol(), 'S');
    }

    #[test]
    fn quality_hard_clipped_sine_flags_clipping() {
        // 4× амплитуда, прижатая к рельсам clamp(0,4095) — почти square wave.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            let unclamped = SINE_LUT_I16[k] as i32 * 4;
            let centered = (unclamped + ADC_MID_SCALE).clamp(0, 4095) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        let demod = demodulate_block(&block, 0);
        let q = channel_quality(demod.a, demod.stats_a);
        assert!(q.clipping);
        // Симвoл S имеет приоритет, distortion может быть и true и false тут —
        // не проверяем; важно что мы не молчим.
        assert_eq!(q.symbol(), 'S');
    }

    #[test]
    fn quality_square_wave_flags_distortion_only() {
        // ±1638 ≈ 80% full-scale → за рельсы не уходит, но это square wave,
        // ~19% энергии в гармониках.
        let mut block = [0_u32; LUT_LEN];
        let amp: i32 = 1638;
        for (k, sample) in block.iter_mut().enumerate() {
            let sign: i32 = if SINE_LUT_I16[k] >= 0 { 1 } else { -1 };
            let centered = (amp * sign + ADC_MID_SCALE) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        let demod = demodulate_block(&block, 0);
        let q = channel_quality(demod.a, demod.stats_a);
        assert!(!q.clipping, "stats={:?}", demod.stats_a);
        assert!(!q.low_signal);
        assert!(q.distortion);
        assert_eq!(q.symbol(), 'H');
    }

    #[test]
    fn quality_symbol_priority_is_s_l_h_dot() {
        let q = Quality { clipping: true, low_signal: true, distortion: true };
        assert_eq!(q.symbol(), 'S');
        let q = Quality { clipping: false, low_signal: true, distortion: true };
        assert_eq!(q.symbol(), 'L');
        let q = Quality { clipping: false, low_signal: false, distortion: true };
        assert_eq!(q.symbol(), 'H');
        assert_eq!(Quality::default().symbol(), '.');
        assert!(Quality::default().ok());
    }

    #[test]
    fn accumulator_propagates_clipping_through_window() {
        // Один блок с клиппингом среди 64 чистых — accumulator всё равно
        // флагнет S по сумме sat_count.
        let mut clean = [0_u32; LUT_LEN];
        for (k, sample) in clean.iter_mut().enumerate() {
            let centered = (SINE_LUT_I16[k] as i32 * 9 / 10 + ADC_MID_SCALE) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        let mut dirty = [0_u32; LUT_LEN];
        for (k, sample) in dirty.iter_mut().enumerate() {
            *sample = pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }
        let mut acc = Accumulator::new();
        for _ in 0..63 {
            acc.push(&demodulate_block(&clean, 0));
        }
        acc.push(&demodulate_block(&dirty, 0));
        let avg = acc.drain_average(0);
        let q = channel_quality(avg.a, avg.stats_a);
        assert!(q.clipping, "avg.stats_a={:?}", avg.stats_a);
        assert_eq!(q.symbol(), 'S');
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
