use crate::lut::LUT_LEN;

pub type AdcSampleBuffer = [u16; LUT_LEN];

#[cfg(target_arch = "arm")]
pub use arm::{ack_dma, configure, snapshot, Sampling, ADC_BUFFER};

#[cfg(target_arch = "arm")]
mod arm {
    use crate::lut::LUT_LEN;
    use core::cell::UnsafeCell;
    use stm32g4xx_hal::{
        pac::{self, ADC1, ADC12_COMMON, DMA1, DMAMUX, GPIOA},
        rcc::Rcc,
    };

    #[repr(C, align(4))]
    pub struct AdcBuffer(UnsafeCell<[u16; LUT_LEN]>);
    unsafe impl Sync for AdcBuffer {}
    impl AdcBuffer {
        const fn new() -> Self {
            Self(UnsafeCell::new([0; LUT_LEN]))
        }
        fn ptr(&self) -> *mut u16 {
            self.0.get() as *mut u16
        }
    }

    pub static ADC_BUFFER: AdcBuffer = AdcBuffer::new();

    pub struct Sampling {
        _adc: ADC1,
    }

    pub fn configure(
        adc1_own: ADC1,
        _common: ADC12_COMMON,
        _dma1: &mut DMA1,
        _rcc: &mut Rcc,
    ) -> Sampling {
        // PAC pointers — we already own ADC1 and have &mut DMA1.
        let rcc_regs = unsafe { &*pac::RCC::ptr() };
        rcc_regs.ahb2enr().modify(|_, w| w.adc12en().set_bit());
        cortex_m::asm::delay(16);

        // PA0 → analog mode (ADC1_IN1). User wires PA4 → PA0 for DAC loopback.
        let gpioa = unsafe { &*GPIOA::ptr() };
        gpioa
            .moder()
            .modify(|_, w| unsafe { w.moder0().bits(0b11) });

        // ADC clock: HCLK/4 sync = 42.5 MHz at SYSCLK 170 MHz (within 60 MHz spec).
        let common = unsafe { &*ADC12_COMMON::ptr() };
        common.ccr().modify(|_, w| w.ckmode().sync_div4());

        let adc = unsafe { &*ADC1::ptr() };

        // Exit deep-power-down, then enable internal voltage regulator.
        adc.cr().modify(|_, w| w.deeppwd().clear_bit());
        adc.cr().modify(|_, w| w.advregen().set_bit());
        // T_ADCVREG_STUP ≈ 20 µs (RM0440). At 170 MHz: 3400 cycles. Pad to 8000.
        cortex_m::asm::delay(8000);

        // Single-ended calibration.
        adc.cr().modify(|_, w| w.adcaldif().clear_bit());
        adc.cr().modify(|_, w| w.adcal().set_bit());
        while adc.cr().read().adcal().bit_is_set() {}
        // RM0440: wait at least 4 ADC clock cycles after ADCAL clears.
        cortex_m::asm::delay(64);

        // Enable.
        adc.isr().write(|w| w.adrdy().clear_bit_by_one());
        adc.cr().modify(|_, w| w.aden().set_bit());
        while adc.isr().read().adrdy().bit_is_clear() {}
        adc.isr().write(|w| w.adrdy().clear_bit_by_one());

        // Single regular conversion: SQ1 = IN1.
        adc.sqr1()
            .modify(|_, w| unsafe { w.l().bits(0).sq1().bits(1) });
        // Sample time IN1: 24.5 ADC cycles ≈ 870 ns at 42.5 MHz.
        adc.smpr1().modify(|_, w| w.smp1().cycles24_5());

        // External trigger TIM6_TRGO rising; DMA circular; 12-bit; one-shot per trigger.
        adc.cfgr().modify(|_, w| {
            w.extsel().tim6_trgo();
            w.exten().rising_edge();
            w.dmaen().set_bit();
            w.dmacfg().circular();
            w.cont().clear_bit();
            w.res().bits12()
        });

        // DMA1 channel 2 (index 1): ADC1.DR → ADC_BUFFER, circular, halfword.
        let dma = unsafe { &*DMA1::ptr() };
        let mux = unsafe { &*DMAMUX::ptr() };
        let ch = dma.ch(1);

        ch.cr().modify(|_, w| w.en().clear_bit());
        while ch.cr().read().en().bit_is_set() {}
        dma.ifcr().write(|w| w.cgif(1).set_bit());

        let dr_addr = adc.dr().as_ptr() as u32;
        let buf_addr = ADC_BUFFER.ptr() as u32;
        ch.par().write(|w| unsafe { w.bits(dr_addr) });
        ch.mar().write(|w| unsafe { w.bits(buf_addr) });
        ch.ndtr().write(|w| unsafe { w.bits(LUT_LEN as u32) });

        // DMAMUX channel 1 (DMA1 ch2). ADC1 request ID = 5 (RM0440 Tbl 91).
        mux.ccr(1).write(|w| unsafe { w.dmareq_id().bits(5) });

        ch.cr().write(|w| unsafe {
            w.pl().bits(0b10);
            w.msize().bits(0b01);
            w.psize().bits(0b01);
            w.minc().set_bit();
            w.pinc().clear_bit();
            w.circ().set_bit();
            w.dir().clear_bit();
            w.mem2mem().clear_bit();
            w.tcie().set_bit();
            w.htie().clear_bit();
            w.teie().set_bit();
            w.en().clear_bit()
        });
        ch.cr().modify(|_, w| w.en().set_bit());

        // Arm ADC for next external trigger.
        adc.cr().modify(|_, w| w.adstart().set_bit());

        Sampling { _adc: adc1_own }
    }

    pub fn snapshot() -> [u16; LUT_LEN] {
        let p = ADC_BUFFER.ptr();
        let mut out = [0u16; LUT_LEN];
        for i in 0..LUT_LEN {
            out[i] = unsafe { core::ptr::read_volatile(p.add(i)) };
        }
        out
    }

    /// Acknowledge DMA1 channel 2 flags. Returns (transfer_complete, transfer_error).
    pub fn ack_dma() -> (bool, bool) {
        let dma = unsafe { &*DMA1::ptr() };
        let isr = dma.isr().read();
        let tc = isr.tcif(1).bit_is_set();
        let te = isr.teif(1).bit_is_set();
        dma.ifcr()
            .write(|w| w.ctcif(1).set_bit().cteif(1).set_bit().chtif(1).set_bit());
        (tc, te)
    }
}
