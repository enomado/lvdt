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
    use embedded_hal::digital::InputPin as _;
    use lvdt::{
        agc::{self, Agc},
        button::{ButtonAction, ButtonFsm},
        clocks,
        cordic::{self, CordicHw},
        display::{self, MyDisplay, ScreenMode},
        excitation::{self, Excitation},
        iq::{self, DemodulatedSample},
        pga::{self, Pga, PgaGain},
        sampling::{self, Sampling},
    };
    use rtic_monotonics::systick::prelude::*;
    use stm32g4xx_hal::gpio::{gpioc::PC13, Input};
    use stm32g4xx_hal::pac::DMA1;
    use stm32g4xx_hal::prelude::*;

    use crate::Mono;

    /// Окно когерентного усреднения I/Q в DMA‑блоках. При 2.5 кГц блоков 64
    /// блока = 25.6 мс. Полоса детектора сужается в √N раз → шум ‑18 dB
    /// относительно одиночного блока. Экран читает `latest` каждые 50 мс.
    const SMOOTHING_BLOCKS: u32 = 64;

    /// Шаг опроса USER‑кнопки. 8 мс — половина DEBOUNCE_MS=15, ловим
    /// фронты надёжно, при этом CPU‑нагрузка нулевая (priority=1, async).
    const BUTTON_POLL_MS: u32 = 8;

    /// Кнопка USER (K1) на WeAct STM32G47xCxTx Core Board V1.0 — PC13.
    /// Активная **HIGH** (нестандартно для blackpill‑клонов!): по схеме
    /// `PC13 — R4(330Ω) — K1 — 3.3V`, нет внешнего pull‑down. С внутренней
    /// подтяжкой `into_pull_up_input()` пин всегда HIGH и кнопка не
    /// реагирует — нужно `into_pull_down_input()` + `is_high()`.
    /// Если у тебя другой WeAct/плата с active‑low кнопкой, переключи на
    /// `into_pull_up_input()` и `is_low()` ниже в `button_poll`.
    type ButtonPin = PC13<Input>;

    #[shared]
    struct Shared {
        latest: Option<DemodulatedSample>,
        gains: (PgaGain, PgaGain),
        screen_mode: ScreenMode,
    }

    #[local]
    struct Local {
        _excitation: Excitation,
        _sampling: Sampling,
        _dma1: DMA1,
        adc_tc_count: u32,
        display: MyDisplay,
        cordic: CordicHw,
        accum: iq::Accumulator,
        pga: Pga,
        agc: Agc,
        button_pin: ButtonPin,
        button_fsm: ButtonFsm,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        let mut rcc = clocks::configure(cx.device.RCC, cx.device.PWR);
        defmt::info!("lvdt conditioner hello");
        defmt::info!("clocks: {:?}", rcc.clocks);

        Mono::start(cx.core.SYST, clocks::CLOCK_PLAN.sysclk_hz);

        let gpioa = cx.device.GPIOA.split(&mut rcc);
        let gpiob = cx.device.GPIOB.split(&mut rcc);
        let gpioc = cx.device.GPIOC.split(&mut rcc);

        let sda = gpiob.pb9.into_alternate_open_drain();
        let scl = gpioa.pa15.into_alternate_open_drain();
        let display = display::init(cx.device.I2C1, sda, scl, &mut rcc);

        // USER button K1 = PC13, active HIGH (см. WeAct G474 V1.0 schematic):
        // K1 замыкает PC13 через R4=330Ω на 3.3V, внешнего pull-down нет.
        // Включаем внутренний pull-down, чтобы idle читался как LOW.
        let button_pin: ButtonPin = gpioc.pc13.into_pull_down_input();

        let mut dma1 = cx.device.DMA1;
        let excitation = excitation::configure(cx.device.DAC1, cx.device.TIM6, &mut dma1, &mut rcc);
        // PGA поднимаем ДО ADC: SQR1 у sampling указывает на IN13/IN16
        // (внутренние выходы OPAMP'ов), к моменту первого ADC trigger они уже
        // должны выдавать осмысленный сигнал, иначе в первом блоке будут нули.
        let pga = pga::configure(cx.device.OPAMP, &mut rcc);
        let sampling = sampling::configure(
            cx.device.ADC1,
            cx.device.ADC2,
            cx.device.ADC12_COMMON,
            &mut dma1,
            &mut rcc,
        );
        let cordic = cordic::configure(cx.device.CORDIC, &mut rcc);

        let initial_gains = (pga.gain_a(), pga.gain_b());
        defmt::info!(
            "stage 6 + AGC: OPAMP1/2 PGA on PA3/PB14 → ADC1_IN13/ADC2_IN16, init gain x{=u8}/x{=u8}",
            initial_gains.0.as_num(),
            initial_gains.1.as_num(),
        );

        refresh_display::spawn().ok();
        button_poll::spawn().ok();

        (
            Shared {
                latest: None,
                gains: initial_gains,
                screen_mode: ScreenMode::default(),
            },
            Local {
                _excitation: excitation,
                _sampling: sampling,
                _dma1: dma1,
                adc_tc_count: 0,
                display,
                cordic,
                accum: iq::Accumulator::new(),
                pga,
                agc: Agc::new(),
                button_pin,
                button_fsm: ButtonFsm::new(),
            },
        )
    }

    #[idle]
    fn idle(_cx: idle::Context) -> ! {
        loop {
            cortex_m::asm::wfi();
        }
    }

    #[task(binds = DMA1_CH2, priority = 5, local = [adc_tc_count, cordic, accum, pga, agc], shared = [latest, gains])]
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
        let buf = sampling::snapshot();
        let demod = iq::demodulate_block(&buf, *cx.local.adc_tc_count);
        cx.local.accum.push(&demod);
        if cx.local.accum.count() >= SMOOTHING_BLOCKS {
            let avg = cx.local.accum.drain_average(*cx.local.adc_tc_count);
            // AGC решает по усреднённому окну — там шум на √N меньше,
            // решение стабильнее, чем по одиночному блоку. Меняет gain
            // прямо здесь, до публикации `latest`, чтобы экран и хост
            // видели уже новый PGA вместе с новой магнитудой.
            let (ca, cb) = agc::tick(cx.local.agc, &avg, cx.local.pga);
            let new_gains = (cx.local.pga.gain_a(), cx.local.pga.gain_b());
            (&mut cx.shared.latest, &mut cx.shared.gains).lock(|l, g| {
                *l = Some(avg);
                *g = new_gains;
            });
            if ca || cb {
                defmt::info!(
                    "agc: gain A=x{=u8} B=x{=u8} (changed A={=bool} B={=bool})",
                    new_gains.0.as_num(),
                    new_gains.1.as_num(),
                    ca,
                    cb,
                );
            }
        }
        // Дорогой CORDIC + defmt над RTT остаются децимированными, иначе на
        // 2.5 kHz блоков лог захлебнётся; экран читает `latest` каждые 50 мс.
        if *cx.local.adc_tc_count & 0x3ff != 0 {
            return;
        }
        let qa = iq::channel_quality(demod.a, demod.stats_a);
        let qb = iq::channel_quality(demod.b, demod.stats_b);
        let a = cordic::deviation(cx.local.cordic, demod.a);
        let b = cordic::deviation(cx.local.cordic, demod.b);
        defmt::info!(
            "tc={=u32} A[{=str} x{=u8} M={=f32}% P={=f32}mr sat={=u16}] B[{=str} x{=u8} M={=f32}% P={=f32}mr sat={=u16}] ΔM={=f32}% ΔP={=f32}mr",
            *cx.local.adc_tc_count,
            qa.symbol_str(),
            cx.local.pga.gain_a().as_num(),
            a.mag_pct,
            a.phase_mrad,
            demod.stats_a.sat_count,
            qb.symbol_str(),
            cx.local.pga.gain_b().as_num(),
            b.mag_pct,
            b.phase_mrad,
            demod.stats_b.sat_count,
            a.mag_pct - b.mag_pct,
            a.phase_mrad - b.phase_mrad,
        );
    }

    #[task(priority = 1, local = [display], shared = [latest, gains, screen_mode])]
    async fn refresh_display(mut cx: refresh_display::Context) {
        loop {
            let (snapshot, gains, mode) = (
                &mut cx.shared.latest,
                &mut cx.shared.gains,
                &mut cx.shared.screen_mode,
            )
                .lock(|l, g, m| (*l, *g, *m));
            if let Some(d) = snapshot {
                cx.local.display.clear_buffer();
                let _ = display::render(cx.local.display, &d, gains.0, gains.1, mode);
                let _ = cx.local.display.flush();
            }
            Mono::delay(50.millis()).await;
        }
    }

    /// Опрашивает USER‑кнопку, прогоняет её через `ButtonFsm` и на каждом
    /// распознанном паттерне переключает `screen_mode`. Логика паттернов:
    ///
    /// | паттерн | режим          |
    /// |---------|----------------|
    /// | S       | Status (default) |
    /// | SS      | Phase           |
    /// | L       | Position        |
    /// | LSS     | Raw debug       |
    ///
    /// Любой нераспознанный набор кликов (SSS, LL, ...) тихо отбрасывается
    /// `ButtonFsm::decode` после `INTERCLICK_GAP_MS` без нового нажатия.
    #[task(priority = 1, local = [button_pin, button_fsm], shared = [screen_mode])]
    async fn button_poll(mut cx: button_poll::Context) {
        loop {
            let now_ms = Mono::now().duration_since_epoch().to_millis() as u32;
            // Active HIGH (см. doc на ButtonPin): K1 даёт PC13 = 3.3V,
            // pull-down держит idle LOW. `is_high` берёт &mut self в
            // embedded-hal 1.0 — `local` уже даёт &mut.
            let pressed = cx.local.button_pin.is_high().unwrap_or(false);
            if let Some(action) = cx.local.button_fsm.step(now_ms, pressed) {
                let new_mode = mode_for_action(action);
                cx.shared.screen_mode.lock(|m| *m = new_mode);
                defmt::info!(
                    "button: {=str} → screen={=str}",
                    action_str(action),
                    screen_str(new_mode),
                );
            }
            Mono::delay(BUTTON_POLL_MS.millis()).await;
        }
    }

    fn mode_for_action(a: ButtonAction) -> ScreenMode {
        match a {
            ButtonAction::Short => ScreenMode::Status,
            ButtonAction::DoubleShort => ScreenMode::Phase,
            ButtonAction::Long => ScreenMode::Position,
            ButtonAction::LongShortShort => ScreenMode::Raw,
        }
    }

    fn action_str(a: ButtonAction) -> &'static str {
        match a {
            ButtonAction::Short => "S",
            ButtonAction::Long => "L",
            ButtonAction::DoubleShort => "SS",
            ButtonAction::LongShortShort => "LSS",
        }
    }

    fn screen_str(m: ScreenMode) -> &'static str {
        match m {
            ScreenMode::Status => "status",
            ScreenMode::Phase => "phase",
            ScreenMode::Position => "position",
            ScreenMode::Raw => "raw",
        }
    }
}

#[cfg(not(target_arch = "arm"))]
fn main() {
    println!(
        "lvdt firmware crate: run `cargo test --target x86_64-unknown-linux-gnu` for host checks"
    );
}
