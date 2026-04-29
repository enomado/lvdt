use core::fmt::Write as _;

use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{ascii::FONT_10X20, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::Point,
    text::{Baseline, Text},
    Drawable,
};

use crate::iq::DemodulatedSample;

pub fn render<D>(display: &mut D, demod: &DemodulatedSample) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On)
        .build();

    let a = demod.a.deviation();
    let b = demod.b.deviation();

    let top = format_channel('A', a.mag_pct);
    let bot = format_channel('B', b.mag_pct);

    Text::with_baseline(&top, Point::new(0, -2), style, Baseline::Top).draw(display)?;
    Text::with_baseline(&bot, Point::new(0, 16), style, Baseline::Top).draw(display)?;
    Ok(())
}

fn format_channel(label: char, mag_pct: f32) -> heapless::String<16> {
    let mut buf = heapless::String::new();
    let _ = write!(&mut buf, "{} {:>6.2}%", label, mag_pct);
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
        gpio::{gpiob::{PB8, PB9}, Alternate, OpenDrain},
        i2c::{I2c, I2cExt as _},
        pac::I2C1,
        rcc::Rcc,
        time::RateExtU32 as _,
    };

    pub type Sda = PB9<Alternate<4, OpenDrain>>;
    pub type Scl = PB8<Alternate<4, OpenDrain>>;

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

    #[test]
    fn formats_pct_right_aligned() {
        assert_eq!(format_channel('A', 95.4).as_str(), "A  95.40%");
        assert_eq!(format_channel('B', 0.0).as_str(),  "B   0.00%");
        assert_eq!(format_channel('A', 100.0).as_str(), "A 100.00%");
    }
}
