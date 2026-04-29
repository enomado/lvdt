//! AGC: подбираем gain PGA так, чтобы магнитуда I/Q жила в окне 25–75% от
//! REFERENCE. Раз в окно усреднения (`SMOOTHING_BLOCKS` блоков ≈ 25.6 мс)
//! делаем по одному решению на канал: при клиппинге сразу шаг вниз, при
//! слабом сигнале шаг вверх, при «горячем» — шаг вниз. Между ступенями
//! lockout `LOCKOUT_WINDOWS`, чтобы не зацикливаться: каждый шаг это
//! фактор 2× по магнитуде, окно 3× даёт запас.
//!
//! Ширина шагов и пороги выбраны под фиксированные ступени PGA ×2/×4/.../×64
//! ([RM0440 §16.3.5][RM]). Шаг 6 ступеней = 2⁶ = 64×, что покрывает
//! динамику типичных вторичек LVDT (~50 мВ … 3.3 В) одной ручкой.
//!
//! [RM]: https://www.st.com/resource/en/reference_manual/rm0440-stm32g4xx-stmicroelectronics.pdf

use crate::iq::{channel_quality, ChannelStats, Iq, REFERENCE_MAGNITUDE_I64};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AgcAction {
    Hold,
    StepUp,
    StepDown,
}

/// |Iq|² < (25%)² · REFERENCE² ⇔ mag_sq · 16 < ref_sq · 1.
const LOW_NUM: i64 = 1;
const LOW_DEN: i64 = 16;
/// |Iq|² > (75%)² · REFERENCE² ⇔ mag_sq · 16 > ref_sq · 9.
const HIGH_NUM: i64 = 9;
const HIGH_DEN: i64 = 16;
/// После каждой смены gain пропускаем столько окон усреднения, прежде чем
/// принимать следующее решение по этому каналу. 1 окно достаточно: PGA
/// settles за ~1 µs (RM0440), но нам нужно полностью «новых» N блоков
/// в аккумуляторе, чтобы магнитуда успела отразить новый gain.
const LOCKOUT_WINDOWS: u8 = 1;

/// Решение AGC по одному каналу. Не зависит от текущего gain — `Pga` сам
/// насыщается на крайних ступенях. Хост‑тестируемо.
pub fn decide(iq: Iq, stats: ChannelStats) -> AgcAction {
    let q = channel_quality(iq, stats);
    if q.clipping {
        // Клиппинг — самый громкий сигнал «опускай gain», игнорируем магнитуду:
        // она при клиппинге занижена и может ввести в заблуждение.
        return AgcAction::StepDown;
    }
    let i = iq.i as i64;
    let qq = iq.q as i64;
    let mag_sq = i * i + qq * qq;
    let ref_sq = REFERENCE_MAGNITUDE_I64 * REFERENCE_MAGNITUDE_I64;
    if mag_sq.saturating_mul(LOW_DEN) < ref_sq.saturating_mul(LOW_NUM) {
        return AgcAction::StepUp;
    }
    if mag_sq.saturating_mul(HIGH_DEN) > ref_sq.saturating_mul(HIGH_NUM) {
        return AgcAction::StepDown;
    }
    AgcAction::Hold
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Agc {
    lock_a: u8,
    lock_b: u8,
}

impl Agc {
    pub const fn new() -> Self {
        Self { lock_a: 0, lock_b: 0 }
    }
}

#[cfg(target_arch = "arm")]
pub use arm::tick;

#[cfg(target_arch = "arm")]
mod arm {
    use super::{decide, Agc, AgcAction, LOCKOUT_WINDOWS};
    use crate::iq::DemodulatedSample;
    use crate::pga::Pga;

    /// Один шаг AGC по обоим каналам. Возвращает `(changed_a, changed_b)` —
    /// для дополнительного логирования из вызывающего кода.
    pub fn tick(state: &mut Agc, sample: &DemodulatedSample, pga: &mut Pga) -> (bool, bool) {
        let mut ca = false;
        let mut cb = false;

        if state.lock_a > 0 {
            state.lock_a -= 1;
        } else {
            let new_gain = match decide(sample.a, sample.stats_a) {
                AgcAction::StepUp => pga.gain_a().step_up(),
                AgcAction::StepDown => pga.gain_a().step_down(),
                AgcAction::Hold => pga.gain_a(),
            };
            if new_gain != pga.gain_a() {
                pga.set_gain_a(new_gain);
                state.lock_a = LOCKOUT_WINDOWS;
                ca = true;
            }
        }

        if state.lock_b > 0 {
            state.lock_b -= 1;
        } else {
            let new_gain = match decide(sample.b, sample.stats_b) {
                AgcAction::StepUp => pga.gain_b().step_up(),
                AgcAction::StepDown => pga.gain_b().step_down(),
                AgcAction::Hold => pga.gain_b(),
            };
            if new_gain != pga.gain_b() {
                pga.set_gain_b(new_gain);
                state.lock_b = LOCKOUT_WINDOWS;
                cb = true;
            }
        }

        (ca, cb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{demodulate_block, pack_dual_adc};
    use crate::lut::{ADC_MID_SCALE, DAC_SINE_LUT, LUT_LEN, SINE_LUT_I16};

    #[test]
    fn clipped_signal_steps_down() {
        // DAC LUT доходит до рельс → sat_count > 0 → клиппинг.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }
        let demod = demodulate_block(&block, 0);
        assert_eq!(decide(demod.a, demod.stats_a), AgcAction::StepDown);
    }

    #[test]
    fn weak_signal_steps_up() {
        // 10% full-scale ⇒ |Iq| ≈ 10% от REFERENCE ⇒ mag² ≈ 1% < 25%·25% = 6.25%.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            let centered = (SINE_LUT_I16[k] as i32 / 10 + ADC_MID_SCALE) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        let demod = demodulate_block(&block, 0);
        assert_eq!(decide(demod.a, demod.stats_a), AgcAction::StepUp);
    }

    #[test]
    fn medium_signal_holds() {
        // 50% full-scale: mag² = 25% от ref² = 0.25 ∈ [0.0625, 0.5625]. Hold.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            let centered = (SINE_LUT_I16[k] as i32 / 2 + ADC_MID_SCALE) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        let demod = demodulate_block(&block, 0);
        assert_eq!(decide(demod.a, demod.stats_a), AgcAction::Hold);
    }

    #[test]
    fn hot_signal_steps_down() {
        // 80% full-scale: mag² = 64% > 75%·75% = 56.25% ⇒ StepDown. Без клиппинга.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            let centered = (SINE_LUT_I16[k] as i32 * 4 / 5 + ADC_MID_SCALE) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        let demod = demodulate_block(&block, 0);
        assert!(demod.stats_a.sat_count == 0, "shouldn't clip at 80%");
        assert_eq!(decide(demod.a, demod.stats_a), AgcAction::StepDown);
    }
}
