//! AGC: подбираем gain PGA так, чтобы магнитуда I/Q жила в окне 25–75% от
//! REFERENCE. Раз в окно усреднения (`SMOOTHING_BLOCKS` блоков ≈ 25.6 мс)
//! считаем решение по каждому каналу, но применяем общую ступень к A и B:
//! при клиппинге или «горячем» сигнале любого канала оба идут вниз, вверх
//! оба идут только если оба слабые. Между ступенями lockout
//! `LOCKOUT_WINDOWS`, чтобы не зацикливаться: каждый шаг это фактор 2× по
//! магнитуде, окно 3× даёт запас.
//!
//! Ширина шагов и пороги выбраны под фиксированные ступени PGA ×2/×4/.../×64
//! ([RM0440 §16.3.5][RM]). Шаг 6 ступеней = 2⁶ = 64×, что покрывает
//! динамику типичных вторичек LVDT (~50 мВ … 3.3 В) одной ручкой.
//!
//! [RM]: https://www.st.com/resource/en/reference_manual/rm0440-stm32g4xx-stmicroelectronics.pdf

use crate::iq::{
    ChannelStats,
    DemodulatedSample,
    Iq,
    REFERENCE_MAGNITUDE_I64,
    channel_quality,
};
use crate::pga::PgaGain;

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

pub trait GainControl {
    fn gain_a(&self) -> PgaGain;
    fn gain_b(&self) -> PgaGain;
    fn set_gain_a(&mut self, gain: PgaGain);
    fn set_gain_b(&mut self, gain: PgaGain);
}

#[cfg(target_arch = "arm")]
impl GainControl for crate::pga::Pga {
    fn gain_a(&self) -> PgaGain {
        self.gain_a()
    }

    fn gain_b(&self) -> PgaGain {
        self.gain_b()
    }

    fn set_gain_a(&mut self, gain: PgaGain) {
        self.set_gain_a(gain);
    }

    fn set_gain_b(&mut self, gain: PgaGain) {
        self.set_gain_b(gain);
    }
}

/// Один шаг AGC по обоим каналам. Возвращает `(changed_a, changed_b)` —
/// для дополнительного логирования из вызывающего кода.
///
/// LVDT-пара работает на общей ступени PGA: если любой канал просит вниз,
/// опускаем оба; вверх поднимаем только когда оба канала слабые. Так
/// `(B-A)/(B+A)` не уезжает из-за разного gain, а hot канал не клиппируется
/// ради слабого соседнего.
pub fn tick<G: GainControl>(state: &mut Agc, sample: &DemodulatedSample, pga: &mut G) -> (bool, bool) {
    if state.lock_a > 0 || state.lock_b > 0 {
        state.lock_a = state.lock_a.saturating_sub(1);
        state.lock_b = state.lock_b.saturating_sub(1);
        return (false, false);
    }

    let action = common_action(decide(sample.a, sample.stats_a), decide(sample.b, sample.stats_b));
    let target = common_target_gain(pga.gain_a(), pga.gain_b(), action);
    let ca = pga.gain_a() != target;
    let cb = pga.gain_b() != target;
    if ca {
        pga.set_gain_a(target);
    }
    if cb {
        pga.set_gain_b(target);
    }
    if ca || cb {
        state.lock_a = LOCKOUT_WINDOWS;
        state.lock_b = LOCKOUT_WINDOWS;
    }

    (ca, cb)
}

fn common_action(a: AgcAction, b: AgcAction) -> AgcAction {
    match (a, b) {
        (AgcAction::StepDown, _) | (_, AgcAction::StepDown) => AgcAction::StepDown,
        (AgcAction::StepUp, AgcAction::StepUp) => AgcAction::StepUp,
        _ => AgcAction::Hold,
    }
}

fn common_target_gain(a: PgaGain, b: PgaGain, action: AgcAction) -> PgaGain {
    let common = lower_gain(a, b);
    match action {
        AgcAction::Hold => common,
        AgcAction::StepUp => common.step_up(),
        AgcAction::StepDown => common.step_down(),
    }
}

fn lower_gain(a: PgaGain, b: PgaGain) -> PgaGain {
    if a.as_num() <= b.as_num() { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{
        DemodulatedSample,
        demodulate_block,
        pack_dual_adc,
    };
    use crate::lut::{
        ADC_MID_SCALE,
        DAC_SINE_LUT,
        LUT_LEN,
        SINE_LUT_I16,
    };

    #[derive(Debug)]
    struct MockPga {
        a: PgaGain,
        b: PgaGain,
    }

    impl MockPga {
        fn new(a: PgaGain, b: PgaGain) -> Self {
            Self { a, b }
        }
    }

    impl GainControl for MockPga {
        fn gain_a(&self) -> PgaGain {
            self.a
        }

        fn gain_b(&self) -> PgaGain {
            self.b
        }

        fn set_gain_a(&mut self, gain: PgaGain) {
            self.a = gain;
        }

        fn set_gain_b(&mut self, gain: PgaGain) {
            self.b = gain;
        }
    }

    fn demod_for_amplitude(num: i32, den: i32) -> DemodulatedSample {
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            let centered = (SINE_LUT_I16[k] as i32 * num / den + ADC_MID_SCALE) as u16;
            *sample = pack_dual_adc(centered, centered);
        }
        demodulate_block(&block, 0)
    }

    fn mixed_sample(a: DemodulatedSample, b: DemodulatedSample) -> DemodulatedSample {
        DemodulatedSample {
            a:        a.a,
            b:        b.b,
            stats_a:  a.stats_a,
            stats_b:  b.stats_b,
            sequence: 0,
        }
    }

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

    #[test]
    fn tick_steps_both_channels_up_when_both_are_weak() {
        let sample = demod_for_amplitude(1, 10);
        let mut agc = Agc::new();
        let mut pga = MockPga::new(PgaGain::X2, PgaGain::X2);

        assert_eq!(tick(&mut agc, &sample, &mut pga), (true, true));
        assert_eq!((pga.gain_a(), pga.gain_b()), (PgaGain::X4, PgaGain::X4));

        assert_eq!(tick(&mut agc, &sample, &mut pga), (false, false));
        assert_eq!((pga.gain_a(), pga.gain_b()), (PgaGain::X4, PgaGain::X4));
    }

    #[test]
    fn tick_holds_common_gain_when_only_one_channel_is_weak() {
        let weak = demod_for_amplitude(1, 10);
        let medium = demod_for_amplitude(1, 2);
        let sample = mixed_sample(weak, medium);
        let mut agc = Agc::new();
        let mut pga = MockPga::new(PgaGain::X8, PgaGain::X8);

        assert_eq!(tick(&mut agc, &sample, &mut pga), (false, false));
        assert_eq!((pga.gain_a(), pga.gain_b()), (PgaGain::X8, PgaGain::X8));
    }

    #[test]
    fn tick_steps_both_channels_down_if_either_channel_is_hot() {
        let hot = demod_for_amplitude(4, 5);
        let weak = demod_for_amplitude(1, 10);
        let sample = mixed_sample(hot, weak);
        let mut agc = Agc::new();
        let mut pga = MockPga::new(PgaGain::X8, PgaGain::X8);

        assert_eq!(tick(&mut agc, &sample, &mut pga), (true, true));
        assert_eq!((pga.gain_a(), pga.gain_b()), (PgaGain::X4, PgaGain::X4));
    }

    #[test]
    fn tick_heals_mismatched_gains_to_lower_common_step() {
        let sample = demod_for_amplitude(1, 2);
        let mut agc = Agc::new();
        let mut pga = MockPga::new(PgaGain::X4, PgaGain::X16);

        assert_eq!(tick(&mut agc, &sample, &mut pga), (false, true));
        assert_eq!((pga.gain_a(), pga.gain_b()), (PgaGain::X4, PgaGain::X4));
    }
}
