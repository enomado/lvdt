use core::fmt::Write as _;

use embedded_graphics::Drawable;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::Point;
use embedded_graphics::text::{
    Baseline,
    Text,
};
use profont::PROFONT_14_POINT;

use crate::iq::{
    ChannelStats,
    DemodulatedSample,
    Iq,
    REFERENCE_MAGNITUDE_I64,
    channel_quality,
};
use crate::pga::PgaGain;

/// Режим экрана. Переключается одной USER‑кнопкой по «фонарным» паттернам:
/// short → Status (по умолчанию), double‑short → Phase, long → Position,
/// long+short+short → Raw debug. Маппинг живёт в `main.rs::apply_button`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScreenMode {
    /// Магнитуда + qualifier + gain по каналам — то, что раньше было всегда.
    #[default]
    Status,
    /// Фаза каждого канала в градусах относительно DAC.
    Phase,
    /// Большой `(B−A)/(B+A)` нормированный на gain — центрированная позиция
    /// сердечника LVDT.
    Position,
    /// Sat counters + harmonic distortion fraction для отладки тракта.
    Raw,
}

pub fn render<D>(
    display: &mut D,
    demod: &DemodulatedSample,
    gain_a: PgaGain,
    gain_b: PgaGain,
    mode: ScreenMode,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    // PROFONT_14_POINT — 10×17, ширина та же что у FONT_10X20, но высота
    // 17 → две строки 0+15 укладываются в 32 px без жёсткой обрезки снизу,
    // и нули без слеша (типографически приятнее).
    let style = MonoTextStyleBuilder::new()
        .font(&PROFONT_14_POINT)
        .text_color(BinaryColor::On)
        .build();

    let (top, bot) = match mode {
        ScreenMode::Status => format_status(demod, gain_a, gain_b),
        ScreenMode::Phase => format_phase(demod, gain_a, gain_b),
        ScreenMode::Position => format_position(demod, gain_a, gain_b),
        ScreenMode::Raw => format_raw(demod),
    };

    Text::with_baseline(&top, Point::new(0, 0), style, Baseline::Top).draw(display)?;
    Text::with_baseline(&bot, Point::new(0, 15), style, Baseline::Top).draw(display)?;
    Ok(())
}

type Line = heapless::String<16>;

fn format_status(demod: &DemodulatedSample, gain_a: PgaGain, gain_b: PgaGain) -> (Line, Line) {
    let a = demod.a.deviation();
    let b = demod.b.deviation();
    let qa = channel_quality(demod.a, demod.stats_a);
    let qb = channel_quality(demod.b, demod.stats_b);
    (
        format_channel('A', qa.symbol(), gain_a, a.mag_pct),
        format_channel('B', qb.symbol(), gain_b, b.mag_pct),
    )
}

fn format_channel(label: char, symbol: char, gain: PgaGain, mag_pct: f32) -> Line {
    // 12 cols × 10 px FONT_10X20 ровно влезает в 128px ширину OLED:
    //   "AS x64 100.0%" не лезет, поэтому жертвуем разрядом процента
    //   и выкидываем разделитель: "ASx64 100.0%" = 12 chars exactly.
    // {:<2} right‑pads gain до двух знаков, чтобы '×2' и '×64' выглядели в столбик.
    let mut buf = Line::new();
    let _ = write!(
        &mut buf,
        "{}{}x{:<2}{:>6.1}%",
        label,
        symbol,
        gain.as_num(),
        mag_pct
    );
    buf
}

fn format_phase(demod: &DemodulatedSample, gain_a: PgaGain, gain_b: PgaGain) -> (Line, Line) {
    let a = demod.a.deviation();
    let b = demod.b.deviation();
    let qa = channel_quality(demod.a, demod.stats_a);
    let qb = channel_quality(demod.b, demod.stats_b);
    (
        format_phase_line('A', qa.symbol(), gain_a, a.phase_mrad),
        format_phase_line('B', qb.symbol(), gain_b, b.phase_mrad),
    )
}

fn format_phase_line(label: char, symbol: char, gain: PgaGain, phase_mrad: f32) -> Line {
    // Градусы влезают компактнее, чем mrad: ±180 — 4 знака со знаком.
    // "A.x64 -180d" = 11 chars; gain x2 → "A.x2  -180d" = 11.
    let phase_deg = phase_mrad * (180.0 / core::f32::consts::PI / 1000.0);
    // Клампим в [-180,180] на случай шумовых atan2 выбросов.
    let phase_deg = phase_deg.clamp(-180.0, 180.0);
    let mut buf = Line::new();
    let _ = write!(
        &mut buf,
        "{}{}x{:<2}{:>+5.0}d",
        label,
        symbol,
        gain.as_num(),
        phase_deg,
    );
    buf
}

/// Возвращает амплитуду канала, скорректированную на текущий PGA gain — то
/// есть «как если бы PGA не было». Это позволяет считать `(B−A)/(B+A)` в
/// сравнимых единицах даже когда AGC поставил каналы на разные ступени.
fn normalized_mag(iq: Iq, gain: PgaGain) -> f32 {
    iq.magnitude() / gain.as_num() as f32
}

fn format_position(demod: &DemodulatedSample, gain_a: PgaGain, gain_b: PgaGain) -> (Line, Line) {
    let ma = normalized_mag(demod.a, gain_a);
    let mb = normalized_mag(demod.b, gain_b);
    let qa = channel_quality(demod.a, demod.stats_a);
    let qb = channel_quality(demod.b, demod.stats_b);

    // Если суммарная амплитуда слишком мала — не делим на ~ноль, рисуем «--».
    // Порог: каждая из mag_a/mag_b в норме ~ 1e8 при full‑scale, 1e6 — это
    // уже глубокий low‑signal, считать положение бессмысленно.
    let sum = ma + mb;
    let denom_ok = sum > 1.0e6;
    let pos_pct = if denom_ok { (mb - ma) / sum * 100.0 } else { 0.0 };

    let mut top = Line::new();
    if denom_ok {
        // "POS +12.34%" = 11 chars, "POS -100.0%" = 11. Берём 4 знач. знака.
        let _ = write!(&mut top, "POS{:>+7.2}%", pos_pct.clamp(-100.0, 100.0));
    } else {
        let _ = write!(&mut top, "POS    --  ");
    }

    // Bot: краткий статус каналов — qualifier + gain без процента, чтобы
    // понимать, валидна ли позиция вообще.
    let mut bot = Line::new();
    let _ = write!(
        &mut bot,
        "A{}x{:<2} B{}x{:<2}",
        qa.symbol(),
        gain_a.as_num(),
        qb.symbol(),
        gain_b.as_num(),
    );
    (top, bot)
}

fn format_raw(demod: &DemodulatedSample) -> (Line, Line) {
    (
        format_raw_line('A', demod.a, demod.stats_a),
        format_raw_line('B', demod.b, demod.stats_b),
    )
}

fn format_raw_line(label: char, iq: Iq, stats: ChannelStats) -> Line {
    // sat_count трёхзначно при сильном клиппинге + sine purity (~ глиф):
    // fund_energy / total_energy в %. Чистый синус → 99.9, полный square
    // wave → ~81 (8/π² от total в первой гармонике), DC‑offset / асимметричный
    // hard clip → ближе к нулю. Формат: "AS000 ~99.9%" = 12 chars, либо
    // "AS000 ~  --%" при low signal.
    //
    // Почему purity, а не доля гармоник: при обрыве/жёстком клиппинге
    // отношение (total−fund)/total легко уходит к 99.9 и теряет различимость
    // между «много гармоник» и «фундаментала вообще нет». Прямой fund/total
    // монотонно отражает «насколько форма похожа на синус».
    let mag_sq = (iq.i as i64) * (iq.i as i64) + (iq.q as i64) * (iq.q as i64);
    let sat = stats.sat_count.min(999);
    let mut buf = Line::new();

    // При обрыве вторички / отсутствии возбуждения mag_sq → 0, а stats.sq_sum
    // = energy ADC‑шума → не ноль. fund/total → 0, формально валидно, но
    // визуально нет смысла отличать «обрыв» от «жёсткий DC clip» — оба ≈ 0%.
    // Тот же порог 1% от REFERENCE, что и в channel_quality::low_signal.
    let ref_sq = REFERENCE_MAGNITUDE_I64 * REFERENCE_MAGNITUDE_I64;
    if mag_sq < ref_sq / 10_000 {
        let _ = write!(&mut buf, "{}S{:0>3} ~  --%", label, sat);
        return buf;
    }

    let total = stats.sq_sum.max(0) as f32;
    // fund_energy = mag_sq / (IQ_AMPLITUDE^2 · LUT_LEN/2) — тот же делитель,
    // что в channel_quality. Для идеального loopback fund немного >total из‑за
    // округления LUT, поэтому clamp до 99.9 (4 chars width).
    let fund_denom = (crate::lut::IQ_AMPLITUDE as i64).pow(2) * (crate::lut::LUT_LEN as i64 / 2);
    let fund = (mag_sq / fund_denom.max(1)) as f32;
    let sine_pct = if total > 0.0 {
        (fund / total * 100.0).clamp(0.0, 99.9)
    } else {
        0.0
    };
    let _ = write!(&mut buf, "{}S{:0>3} ~{:>4.1}%", label, sat, sine_pct);
    buf
}

#[cfg(target_arch = "arm")]
pub use arm::{
    MyDisplay,
    init,
};

#[cfg(target_arch = "arm")]
mod arm {
    use ssd1306::mode::BufferedGraphicsMode;
    use ssd1306::prelude::*;
    use ssd1306::rotation::DisplayRotation;
    use ssd1306::size::DisplaySize128x32;
    use ssd1306::{
        I2CDisplayInterface,
        Ssd1306,
    };
    use stm32g4xx_hal::gpio::gpioa::PA15;
    use stm32g4xx_hal::gpio::gpiob::PB9;
    use stm32g4xx_hal::gpio::{
        Alternate,
        OpenDrain,
    };
    use stm32g4xx_hal::i2c::{
        I2c,
        I2cExt as _,
    };
    use stm32g4xx_hal::pac::I2C1;
    use stm32g4xx_hal::rcc::Rcc;
    use stm32g4xx_hal::time::RateExtU32 as _;

    // SCL=PA15 (НЕ PB8!). PB8 = BOOT0 sample pin на STM32G474 LQFP48 с
    // дефолтными option bytes (nSWBOOT0=1): pull-up на SCL держит BOOT0 high
    // при reset, чип уходит в системный bootloader и наша прошивка не стартует.
    pub type Sda = PB9<Alternate<4, OpenDrain>>;
    pub type Scl = PA15<Alternate<4, OpenDrain>>;

    pub type MyDisplay = Ssd1306<
        I2CInterface<I2c<I2C1, Sda, Scl>>,
        DisplaySize128x32,
        BufferedGraphicsMode<DisplaySize128x32>,
    >;

    pub fn init(i2c1: I2C1, sda: Sda, scl: Scl, rcc: &mut Rcc) -> MyDisplay {
        let i2c = i2c1.i2c((sda, scl), 400_u32.kHz(), rcc);
        let interface = I2CDisplayInterface::new(i2c);
        let mut display = Ssd1306::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display.init().unwrap();
        display.set_brightness(Brightness::DIM).unwrap();
        display
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{
        ChannelStats,
        Iq,
    };
    use crate::lut::{
        ADC_MID_SCALE,
        DAC_SINE_LUT,
        LUT_LEN,
        SINE_LUT_I16,
    };
    use crate::pga::PgaGain;

    #[test]
    fn formats_pct_with_gain() {
        // 12 cols максимум при FONT_10X20 на 128‑pixel OLED. {:<2} пэдит
        // gain до двух знаков пробелом справа, {:>6.1}% даёт 7 chars, префикс
        // "X.xN" — 4 или 5 chars (в зависимости от двузначности gain).
        assert_eq!(
            format_channel('A', '.', PgaGain::X2, 95.4).as_str(),
            "A.x2   95.4%"
        );
        assert_eq!(
            format_channel('B', 'L', PgaGain::X64, 0.0).as_str(),
            "BLx64   0.0%"
        );
        assert_eq!(
            format_channel('A', 'S', PgaGain::X16, 100.0).as_str(),
            "ASx16 100.0%"
        );
    }

    #[test]
    fn phase_line_clamps_and_fits_12_cols() {
        // π rad = 180°. format должен дать ровно ≤12 chars и не вылететь
        // за рельсы из‑за noise atan2.
        let max = format_phase_line('A', '.', PgaGain::X64, 4000.0); // > π
        assert!(max.len() <= 12, "'{}' too wide", max);
        assert_eq!(max.as_str(), "A.x64 +180d");
        let min = format_phase_line('B', 'S', PgaGain::X2, -4000.0);
        assert_eq!(min.as_str(), "BSx2  -180d");
        let zero = format_phase_line('A', '.', PgaGain::X4, 0.0);
        assert_eq!(zero.as_str(), "A.x4    +0d");
    }

    #[test]
    fn position_centred_when_a_eq_b() {
        // Симметрия: оба канала видят полный loopback с одним и тем же
        // gain → POS = 0.00%.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = crate::iq::pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }
        let demod = crate::iq::demodulate_block(&block, 0);
        let (top, bot) = format_position(&demod, PgaGain::X4, PgaGain::X4);
        assert_eq!(top.as_str(), "POS  +0.00%");
        // bot — qualifier+gains; A и B одинаковые, лишь бы влезло.
        assert!(bot.len() <= 12, "'{}' too wide", bot);
    }

    #[test]
    fn position_skewed_when_b_dominates() {
        // A молчит (mid-scale), B даёт полный синус → POS должен быть +100%.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = crate::iq::pack_dual_adc(ADC_MID_SCALE as u16, DAC_SINE_LUT[k]);
        }
        let demod = crate::iq::demodulate_block(&block, 0);
        let (top, _bot) = format_position(&demod, PgaGain::X4, PgaGain::X4);
        // mag_a ≈ 0 → (mb-ma)/(mb+ma) ≈ +1.0 → +100.0%
        assert_eq!(top.as_str(), "POS+100.00%");
    }

    #[test]
    fn position_normalises_by_gain() {
        // Если A на ×4 и B на ×16, и обе вторички видят одинаковую амплитуду
        // на входе, то raw mag_b будет в 4× больше. После нормализации
        // на gain — обе равны, POS должна быть 0.
        // Эмулируем: оба ADC видят DAC sine, gain "виртуально" разные.
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            *sample = crate::iq::pack_dual_adc(DAC_SINE_LUT[k], DAC_SINE_LUT[k]);
        }
        let demod = crate::iq::demodulate_block(&block, 0);
        // Подменяем mag_b в 4× больше, чтобы сэмулировать «B на gain×4
        // относительно A». normalised_mag должна вернуть равные.
        // Прямо в этом тесте мы передаём демод как есть, но gain_a=X4,
        // gain_b=X16 — на пользовательском уровне это ситуация, где AGC
        // поставил каналы по-разному. Мы хотим проверить, что в случае
        // _одинаковых raw mag_ POS не равна нулю (gain факторит).
        // Здесь raw равны → нормализованные становятся неравными:
        // POS = (mb/16 - ma/4)/(mb/16 + ma/4) при ma=mb даёт
        // (1/16 - 1/4)/(1/16 + 1/4) = (1-4)/(1+4) = -3/5 = -60.00%.
        let (top, _) = format_position(&demod, PgaGain::X4, PgaGain::X16);
        assert_eq!(top.as_str(), "POS -60.00%");
    }

    #[test]
    fn position_silence_renders_dashes() {
        let demod = crate::iq::DemodulatedSample::default();
        let (top, _) = format_position(&demod, PgaGain::X2, PgaGain::X2);
        assert_eq!(top.as_str(), "POS    --  ");
    }

    #[test]
    fn raw_line_clean_sine_is_close_to_100pct() {
        // 90% loopback — без рельс, форма практически чистая. sine purity
        // должна упереться в clamp 99.9. (Именно этот сценарий пользователь
        // видит после init.)
        let mut block = [0_u32; LUT_LEN];
        for (k, sample) in block.iter_mut().enumerate() {
            let centred = (SINE_LUT_I16[k] as i32 * 9 / 10 + ADC_MID_SCALE) as u16;
            *sample = crate::iq::pack_dual_adc(centred, centred);
        }
        let demod = crate::iq::demodulate_block(&block, 0);
        let s = format_raw_line('B', demod.b, demod.stats_b);
        assert_eq!(s.len(), 12, "'{}' wrong width", s);
        assert_eq!(s.as_str(), "BS000 ~99.9%");
    }

    #[test]
    fn raw_line_square_wave_is_about_81pct() {
        // ±80% full-scale square (как в quality_square_wave_flags_distortion_only):
        // fund/total = 8/π² ≈ 0.811 для классического square. Видим ~81%,
        // не 99 — теперь сразу понятно «это не синус».
        let mut block = [0_u32; LUT_LEN];
        let amp: i32 = 1638;
        for (k, sample) in block.iter_mut().enumerate() {
            let sign: i32 = if SINE_LUT_I16[k] >= 0 { 1 } else { -1 };
            let centred = (amp * sign + ADC_MID_SCALE) as u16;
            *sample = crate::iq::pack_dual_adc(centred, centred);
        }
        let demod = crate::iq::demodulate_block(&block, 0);
        let s = format_raw_line('A', demod.a, demod.stats_a);
        assert_eq!(s.len(), 12, "'{}' wrong width", s);
        // Приёмочно: 70..90 %. Точное значение зависит от LUT‑квантизации.
        let pct_str = &s[7..11]; // "AS000 ~XX.X%"  → срез "XX.X"
        let pct: f32 = pct_str.trim().parse().unwrap();
        assert!((70.0..90.0).contains(&pct), "expected ~81%, got '{}'", s);
        assert_eq!(demod.stats_a.sat_count, 0); // именно square, не clipping
    }

    #[test]
    fn raw_line_low_signal_shows_dashes() {
        // Обрыв вторички: mag ≈ 0, но stats.sq_sum ≠ 0 от ADC noise.
        // Раньше показывало 99.9% как «вся энергия в гармониках» — теперь
        // прочерки, чтобы не путать с реальным «грязным» сигналом.
        let stats = ChannelStats {
            abs_sum:   0,
            sq_sum:    1_000_000,
            sat_count: 0,
        };
        let iq = Iq { i: 100, q: 0 };
        let s = format_raw_line('A', iq, stats);
        assert_eq!(s.len(), 12, "'{}' wrong width", s);
        assert_eq!(s.as_str(), "AS000 ~  --%");
    }
}
