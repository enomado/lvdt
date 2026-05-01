//! Управление встроенными OPAMP1/OPAMP2 в режиме программируемого усиления.
//!
//! Цель: AGC. Демодулятор живёт в `iq.rs` с фиксированным окном
//! `REFERENCE_MAGNITUDE`; чтобы магнитуда оставалась внутри ±20 dB от этого
//! окна на любых вторичках LVDT, между пином и ADC ставим встроенный PGA с
//! программируемой ступенью ×2/×4/×8/×16/×32/×64.
//!
//! ## Маршрут сигнала
//!
//! - Канал A: PA3 → OPAMP1 (VINP1) → внутренний выход → ADC1_IN13.
//! - Канал B: PB14 → OPAMP2 (VINP1) → внутренний выход → ADC2_IN16.
//!
//! Внешние выходы (PA2/PA6) **не** включаем (`OPAINTOEN=Adcchannel`), чтобы
//! не нагружать PGA внешней цепью и не съедать пины.
//!
//! ## Запись CSR
//!
//! `VM_SEL=Pga` (внутренний резистивный делитель — без внешнего VINM),
//! `VP_SEL=Vinp1` (PA3 / PB14), `PGA_GAIN` ∈ {Gain2..Gain64} = 0..5,
//! `OPAINTOEN=Adcchannel`, `OPAEN=Enabled`. После `OPAEN=1` нужен ~3 µs на
//! settle перед первой регулярной выборкой; `sampling::configure` запускает
//! ADC ощутимо позже, так что отдельной задержки не делаем.
//!
//! ## Смена gain в рантайме
//!
//! ST явно разрешает менять `PGA_GAIN` на лету без переразрешения OPAMP
//! (RM0440 §16.3.5). После записи CSR PGA settles за ~1 µs — в рамках
//! одного DMA‑блока (400 µs) это полностью прячется от демодулятора.

#[cfg(target_arch = "arm")]
pub use arm::{
    Pga,
    configure,
};

/// Шесть ступеней PGA. Числовые `as_num()` совпадают с реальным коэффициентом
/// усиления, `as_bits()` — с кодировкой PGA_GAIN[3:0] для PGA‑mode без
/// внешнего фильтра/смещения.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgaGain {
    X2 = 0,
    X4 = 1,
    X8 = 2,
    X16 = 3,
    X32 = 4,
    X64 = 5,
}

impl PgaGain {
    pub const ALL: [PgaGain; 6] = [
        PgaGain::X2,
        PgaGain::X4,
        PgaGain::X8,
        PgaGain::X16,
        PgaGain::X32,
        PgaGain::X64,
    ];

    pub const fn as_num(self) -> u8 {
        match self {
            PgaGain::X2 => 2,
            PgaGain::X4 => 4,
            PgaGain::X8 => 8,
            PgaGain::X16 => 16,
            PgaGain::X32 => 32,
            PgaGain::X64 => 64,
        }
    }

    pub const fn as_bits(self) -> u8 {
        self as u8
    }

    /// Шаг вверх по шкале с насыщением на ×64.
    pub fn step_up(self) -> PgaGain {
        match self {
            PgaGain::X2 => PgaGain::X4,
            PgaGain::X4 => PgaGain::X8,
            PgaGain::X8 => PgaGain::X16,
            PgaGain::X16 => PgaGain::X32,
            PgaGain::X32 => PgaGain::X64,
            PgaGain::X64 => PgaGain::X64,
        }
    }

    /// Шаг вниз по шкале с насыщением на ×2.
    pub fn step_down(self) -> PgaGain {
        match self {
            PgaGain::X2 => PgaGain::X2,
            PgaGain::X4 => PgaGain::X2,
            PgaGain::X8 => PgaGain::X4,
            PgaGain::X16 => PgaGain::X8,
            PgaGain::X32 => PgaGain::X16,
            PgaGain::X64 => PgaGain::X32,
        }
    }
}

#[cfg(target_arch = "arm")]
mod arm {
    use stm32g4xx_hal::pac::{
        self,
        GPIOA,
        GPIOB,
        OPAMP,
    };
    use stm32g4xx_hal::rcc::Rcc;

    use super::PgaGain;

    pub struct Pga {
        _opamp: OPAMP,
        gain_a: PgaGain,
        gain_b: PgaGain,
    }

    impl Pga {
        pub fn gain_a(&self) -> PgaGain {
            self.gain_a
        }
        pub fn gain_b(&self) -> PgaGain {
            self.gain_b
        }

        pub fn set_gain_a(&mut self, gain: PgaGain) {
            if gain == self.gain_a {
                return;
            }
            let opamp = unsafe { &*OPAMP::ptr() };
            opamp.opamp1_csr().modify(|_, w| {
                // PGA_GAIN — единственное, что меняется. VP_SEL/VM_SEL/OPAEN
                // оставляем как настроили в configure.
                unsafe { w.pga_gain().bits(gain.as_bits()) }
            });
            self.gain_a = gain;
        }

        pub fn set_gain_b(&mut self, gain: PgaGain) {
            if gain == self.gain_b {
                return;
            }
            let opamp = unsafe { &*OPAMP::ptr() };
            opamp
                .opamp2_csr()
                .modify(|_, w| unsafe { w.pga_gain().bits(gain.as_bits()) });
            self.gain_b = gain;
        }
    }

    pub fn configure(opamp_own: OPAMP, _rcc: &mut Rcc) -> Pga {
        let initial = PgaGain::X2;
        enable_opamp_clock();
        configure_pga_input_pins();
        configure_opamps(initial);

        Pga {
            _opamp: opamp_own,
            gain_a: initial,
            gain_b: initial,
        }
    }

    fn enable_opamp_clock() {
        // OPAMP делит шину APB2 с SYSCFG; SYSCFGEN зажигает оба, в RM0440
        // отдельного OPAMPEN бита нет. HAL обычно поднимает SYSCFGEN сам,
        // но проверим явно.
        let rcc_regs = unsafe { &*pac::RCC::ptr() };
        rcc_regs.apb2enr().modify(|_, w| w.syscfgen().set_bit());
        cortex_m::asm::delay(16);
    }

    fn configure_pga_input_pins() {
        // Аналоговые входы PGA: PA3 (OPAMP1_VINP1) и PB14 (OPAMP2_VINP1).
        let gpioa = unsafe { &*GPIOA::ptr() };
        gpioa.moder().modify(|_, w| unsafe { w.moder3().bits(0b11) });
        let gpiob = unsafe { &*GPIOB::ptr() };
        gpiob.moder().modify(|_, w| unsafe { w.moder14().bits(0b11) });
    }

    fn configure_opamps(initial: PgaGain) {
        let opamp = unsafe { &*OPAMP::ptr() };

        opamp.opamp1_csr().write(|w| {
            w.vp_sel().vinp1();
            w.vm_sel().pga();
            unsafe { w.pga_gain().bits(initial.as_bits()) };
            w.opaintoen().adcchannel();
            w.opaen().enabled()
        });

        opamp.opamp2_csr().write(|w| {
            w.vp_sel().vinp1();
            w.vm_sel().pga();
            unsafe { w.pga_gain().bits(initial.as_bits()) };
            w.opaintoen().adcchannel();
            w.opaen().enabled()
        });

        // OPAMP startup ≤ 6 µs (DS12288). При SYSCLK 170 МГц — ~1000 тактов.
        cortex_m::asm::delay(2_000);
    }
}
