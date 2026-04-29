use core::fmt::Write as _;

use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{ascii::FONT_10X20, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::Point,
    text::{Baseline, Text},
    Drawable,
};

use crate::iq::{channel_quality, DemodulatedSample};
use crate::pga::PgaGain;

pub fn render<D>(
    display: &mut D,
    demod: &DemodulatedSample,
    gain_a: PgaGain,
    gain_b: PgaGain,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On)
        .build();

    let a = demod.a.deviation();
    let b = demod.b.deviation();
    let qa = channel_quality(demod.a, demod.stats_a);
    let qb = channel_quality(demod.b, demod.stats_b);

    let top = format_channel('A', qa.symbol(), gain_a, a.mag_pct);
    let bot = format_channel('B', qb.symbol(), gain_b, b.mag_pct);

    Text::with_baseline(&top, Point::new(0, -2), style, Baseline::Top).draw(display)?;
    Text::with_baseline(&bot, Point::new(0, 16), style, Baseline::Top).draw(display)?;
    Ok(())
}

fn format_channel(
    label: char,
    symbol: char,
    gain: PgaGain,
    mag_pct: f32,
) -> heapless::String<16> {
    // 12 cols × 10 px FONT_10X20 ровно влезает в 128px ширину OLED:
    //   "AS x64 100.0%" не лезет, поэтому жертвуем разрядом процента
    //   и выкидываем разделитель: "ASx64 100.0%" = 12 chars exactly.
    // {:<2} right‑pads gain до двух знаков, чтобы '×2' и '×64' выглядели в столбик.
    let mut buf = heapless::String::new();
    let _ = write!(&mut buf, "{}{}x{:<2}{:>6.1}%", label, symbol, gain.as_num(), mag_pct);
    buf
}

#[cfg(target_arch = "arm")]
pub use arm::{init, MyDisplay};

#[cfg(target_arch = "arm")]
mod arm {
    use ssd1306::{
        mode::BufferedGraphicsMode,
        prelude::*,
        rotation::DisplayRotation,
        size::DisplaySize128x32,
        I2CDisplayInterface, Ssd1306,
    };
    use stm32g4xx_hal::{
        gpio::{gpioa::PA15, gpiob::PB9, Alternate, OpenDrain},
        i2c::{I2c, I2cExt as _},
        pac::I2C1,
        rcc::Rcc,
        time::RateExtU32 as _,
    };

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
    use super::format_channel;

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
}
