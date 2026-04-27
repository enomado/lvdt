use crate::lut::{ACTUAL_EXCITATION_HZ, ACTUAL_SAMPLE_HZ, SYSCLK_HZ, TIM6_ARR};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockPlan {
    pub sysclk_hz: u32,
    pub tim6_arr: u16,
    pub actual_sample_hz: f32,
    pub actual_excitation_hz: f32,
}

pub const CLOCK_PLAN: ClockPlan = ClockPlan {
    sysclk_hz: SYSCLK_HZ,
    tim6_arr: TIM6_ARR,
    actual_sample_hz: ACTUAL_SAMPLE_HZ,
    actual_excitation_hz: ACTUAL_EXCITATION_HZ,
};

#[cfg(target_arch = "arm")]
use stm32g4xx_hal::{
    pac,
    pwr::{PwrExt, VoltageScale},
    rcc::{Config, PllConfig, PllMDiv, PllNMul, PllQDiv, PllRDiv, PllSrc, Prescaler, Rcc, RccExt},
    time::RateExtU32,
};

#[cfg(target_arch = "arm")]
pub fn config_hse8_170mhz() -> Config {
    Config::pll()
        .pll_cfg(PllConfig {
            mux: PllSrc::HSE(8.MHz()),
            m: PllMDiv::DIV_2,
            n: PllNMul::MUL_85,
            r: Some(PllRDiv::DIV_2),
            q: Some(PllQDiv::DIV_8),
            p: None,
        })
        .ahb_psc(Prescaler::NotDivided)
        .apb1_psc(Prescaler::NotDivided)
        .apb2_psc(Prescaler::NotDivided)
}

#[cfg(target_arch = "arm")]
pub fn configure(rcc: pac::RCC, pwr: pac::PWR) -> Rcc {
    // Range1 Boost обязателен для SYSCLK > 150 МГц; без него rcc.freeze панует
    // на configure_wait_states. Config::boost(...) в hal 0.1.0 — no-op.
    let pwr = pwr
        .constrain()
        .vos(VoltageScale::Range1 { enable_boost: true })
        .freeze();
    let rcc = rcc.freeze(config_hse8_170mhz(), pwr);
    rcc.enable_hsi48();
    rcc
}
