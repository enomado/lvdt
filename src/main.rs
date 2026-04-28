#![cfg_attr(target_arch = "arm", no_std)]
#![cfg_attr(target_arch = "arm", no_main)]

#[cfg(target_arch = "arm")]
use {defmt_rtt as _, panic_probe as _};

#[cfg(target_arch = "arm")]
#[rtic::app(
    device = stm32g4xx_hal::pac,
    peripherals = true,
    dispatchers = [SAI, COMP1_2_3, COMP7]
)]
mod app {
    use lvdt::{
        clocks,
        excitation::{self, Excitation},
        iq,
        sampling::{self, Sampling},
    };
    use stm32g4xx_hal::pac::DMA1;
    use stm32g4xx_hal::prelude::*;

    #[shared]
    struct Shared {}

    #[local]
    struct Local {
        _excitation: Excitation,
        _sampling: Sampling,
        _dma1: DMA1,
        adc_tc_count: u32,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        let mut rcc = clocks::configure(cx.device.RCC, cx.device.PWR);
        defmt::info!("lvdt conditioner hello");
        defmt::info!("clocks: {:?}", rcc.clocks);

        let _gpioa = cx.device.GPIOA.split(&mut rcc); // PA4 stays in default Analog mode
        let mut dma1 = cx.device.DMA1;
        let excitation =
            excitation::configure(cx.device.DAC1, cx.device.TIM6, &mut dma1, &mut rcc);
        let sampling = sampling::configure(
            cx.device.ADC1,
            cx.device.ADC2,
            cx.device.ADC12_COMMON,
            &mut dma1,
            &mut rcc,
        );

        defmt::info!(
            "stage 5: ADC1+ADC2 dual-regular sync, TIM6 TRGO, DMA1 ch2 ← CDR → 64×u32 circular"
        );

        (
            Shared {},
            Local {
                _excitation: excitation,
                _sampling: sampling,
                _dma1: dma1,
                adc_tc_count: 0,
            },
        )
    }

    #[idle]
    fn idle(_cx: idle::Context) -> ! {
        loop {
            cortex_m::asm::wfi();
        }
    }

    // DMA1 channel 2 is wired to ADC1.DR. TC fires every 64 samples ≈ 400 µs.
    #[task(binds = DMA1_CH2, priority = 5, local = [adc_tc_count])]
    fn adc_dma(cx: adc_dma::Context) {
        let (tc, te) = sampling::ack_dma();
        if te {
            defmt::error!("ADC DMA TEIF");
            return;
        }
        if !tc {
            return;
        }
        *cx.local.adc_tc_count = cx.local.adc_tc_count.wrapping_add(1);
        // ~2500 TCs/s → log every 1024 ≈ 0.4 s.
        if *cx.local.adc_tc_count & 0x3ff != 0 {
            return;
        }
        let buf = sampling::snapshot();
        let (mut a_min, mut a_max, mut a_sum) = (u16::MAX, 0u16, 0u32);
        let (mut b_min, mut b_max, mut b_sum) = (u16::MAX, 0u16, 0u32);
        for &packed in &buf {
            let a = (packed & 0x0fff) as u16;
            let b = ((packed >> 16) & 0x0fff) as u16;
            if a < a_min {
                a_min = a;
            }
            if a > a_max {
                a_max = a;
            }
            a_sum += a as u32;
            if b < b_min {
                b_min = b;
            }
            if b > b_max {
                b_max = b;
            }
            b_sum += b as u32;
        }
        let n = buf.len() as u32;
        let demod = iq::demodulate_block(&buf, *cx.local.adc_tc_count);
        let mag_a = demod.a.magnitude();
        let mag_b = demod.b.magnitude();
        defmt::info!(
            "adc tc={=u32} A[mean={=u32} pp={=u16} |IQ|={=f32}] B[mean={=u32} pp={=u16} |IQ|={=f32}]",
            *cx.local.adc_tc_count,
            a_sum / n,
            a_max - a_min,
            mag_a,
            b_sum / n,
            b_max - b_min,
            mag_b,
        );
    }
}

#[cfg(not(target_arch = "arm"))]
fn main() {
    println!(
        "lvdt firmware crate: run `cargo test --target x86_64-unknown-linux-gnu` for host checks"
    );
}
