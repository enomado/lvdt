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
        let demod = iq::demodulate_block(&buf, *cx.local.adc_tc_count);
        let a = demod.a.deviation();
        let b = demod.b.deviation();
        // mag in % of ideal full-swing DAC→ADC loopback (REFERENCE_MAGNITUDE),
        // phase in milliradians relative to the DAC excitation (0 = in phase).
        defmt::info!(
            "tc={=u32} A[M={=f32}% P={=f32}mr] B[M={=f32}% P={=f32}mr] ΔM={=f32}% ΔP={=f32}mr",
            *cx.local.adc_tc_count,
            a.mag_pct,
            a.phase_mrad,
            b.mag_pct,
            b.phase_mrad,
            a.mag_pct - b.mag_pct,
            a.phase_mrad - b.phase_mrad,
        );
    }
}

#[cfg(not(target_arch = "arm"))]
fn main() {
    println!(
        "lvdt firmware crate: run `cargo test --target x86_64-unknown-linux-gnu` for host checks"
    );
}
