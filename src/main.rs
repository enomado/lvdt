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
    };
    use stm32g4xx_hal::prelude::*;

    #[shared]
    struct Shared {}

    #[local]
    struct Local {
        _excitation: Excitation,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        let mut rcc = clocks::configure(cx.device.RCC, cx.device.PWR);
        defmt::info!("lvdt conditioner hello");
        defmt::info!("clocks: {:?}", rcc.clocks);

        let _gpioa = cx.device.GPIOA.split(&mut rcc); // PA4 stays in default Analog mode
        let excitation =
            excitation::configure(cx.device.DAC1, cx.device.TIM6, cx.device.DMA1, &mut rcc);

        (
            Shared {},
            Local {
                _excitation: excitation,
            },
        )
    }

    #[idle]
    fn idle(_cx: idle::Context) -> ! {
        loop {
            cortex_m::asm::wfi();
        }
    }
}

#[cfg(not(target_arch = "arm"))]
fn main() {
    println!(
        "lvdt firmware crate: run `cargo test --target x86_64-unknown-linux-gnu` for host checks"
    );
}
