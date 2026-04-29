#![cfg_attr(target_arch = "arm", no_std)]
#![cfg_attr(target_arch = "arm", no_main)]

#[cfg(target_arch = "arm")]
use {defmt_rtt as _, panic_probe as _};

#[cfg(target_arch = "arm")]
rtic_monotonics::systick_monotonic!(Mono, 1_000);

#[cfg(target_arch = "arm")]
#[rtic::app(
    device = stm32g4xx_hal::pac,
    peripherals = true,
    dispatchers = [SAI, COMP1_2_3, COMP7]
)]
mod app {
    use lvdt::{
        clocks,
        cordic::{self, CordicHw},
        display::{self, MyDisplay},
        excitation::{self, Excitation},
        iq::{self, DemodulatedSample},
        sampling::{self, Sampling},
    };
    use rtic_monotonics::systick::prelude::*;
    use stm32g4xx_hal::pac::DMA1;
    use stm32g4xx_hal::prelude::*;

    use crate::Mono;

    #[shared]
    struct Shared {
        latest: Option<DemodulatedSample>,
    }

    #[local]
    struct Local {
        _excitation: Excitation,
        _sampling: Sampling,
        _dma1: DMA1,
        adc_tc_count: u32,
        display: MyDisplay,
        cordic: CordicHw,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        let mut rcc = clocks::configure(cx.device.RCC, cx.device.PWR);
        defmt::info!("lvdt conditioner hello");
        defmt::info!("clocks: {:?}", rcc.clocks);

        Mono::start(cx.core.SYST, clocks::CLOCK_PLAN.sysclk_hz);

        let gpioa = cx.device.GPIOA.split(&mut rcc);
        let gpiob = cx.device.GPIOB.split(&mut rcc);

        let sda = gpiob.pb9.into_alternate_open_drain();
        let scl = gpioa.pa15.into_alternate_open_drain();
        let display = display::init(cx.device.I2C1, sda, scl, &mut rcc);

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
        let cordic = cordic::configure(cx.device.CORDIC, &mut rcc);

        defmt::info!(
            "stage 5 + display: ADC dual + SSD1306 128x32 @ I2C1 PB8/PB9"
        );

        refresh_display::spawn().ok();

        (
            Shared { latest: None },
            Local {
                _excitation: excitation,
                _sampling: sampling,
                _dma1: dma1,
                adc_tc_count: 0,
                display,
                cordic,
            },
        )
    }

    #[idle]
    fn idle(_cx: idle::Context) -> ! {
        loop {
            cortex_m::asm::wfi();
        }
    }

    #[task(binds = DMA1_CH2, priority = 5, local = [adc_tc_count, cordic], shared = [latest])]
    fn adc_dma(mut cx: adc_dma::Context) {
        let (tc, te) = sampling::ack_dma();
        if te {
            defmt::error!("ADC DMA TEIF");
            return;
        }
        if !tc {
            return;
        }
        *cx.local.adc_tc_count = cx.local.adc_tc_count.wrapping_add(1);
        if *cx.local.adc_tc_count & 0x3ff != 0 {
            return;
        }
        let buf = sampling::snapshot();
        let demod = iq::demodulate_block(&buf, *cx.local.adc_tc_count);
        let a = cordic::deviation(cx.local.cordic, demod.a);
        let b = cordic::deviation(cx.local.cordic, demod.b);
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
        cx.shared.latest.lock(|l| *l = Some(demod));
    }

    #[task(priority = 1, local = [display], shared = [latest])]
    async fn refresh_display(mut cx: refresh_display::Context) {
        loop {
            let snapshot = cx.shared.latest.lock(|l| *l);
            if let Some(d) = snapshot {
                cx.local.display.clear_buffer();
                let _ = display::render(cx.local.display, &d);
                let _ = cx.local.display.flush();
            }
            Mono::delay(50.millis()).await;
        }
    }
}

#[cfg(not(target_arch = "arm"))]
fn main() {
    println!(
        "lvdt firmware crate: run `cargo test --target x86_64-unknown-linux-gnu` for host checks"
    );
}
