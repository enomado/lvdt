use crate::lut::LUT_LEN;

pub type AdcSampleBuffer = [u32; LUT_LEN];

#[cfg(target_arch = "arm")]
pub use arm::{
    ADC_BUFFER,
    Sampling,
    ack_dma,
    configure,
    snapshot,
};

#[cfg(target_arch = "arm")]
mod arm {
    use core::cell::UnsafeCell;

    use stm32g4xx_hal::pac::{
        self,
        ADC1,
        ADC2,
        ADC12_COMMON,
        DMA1,
        DMAMUX,
    };
    use stm32g4xx_hal::rcc::Rcc;

    use crate::lut::LUT_LEN;

    #[repr(C, align(4))]
    pub struct AdcBuffer(UnsafeCell<[u32; LUT_LEN]>);
    unsafe impl Sync for AdcBuffer {
    }
    impl AdcBuffer {
        const fn new() -> Self {
            Self(UnsafeCell::new([0; LUT_LEN]))
        }
        fn ptr(&self) -> *mut u32 {
            self.0.get() as *mut u32
        }
    }

    pub static ADC_BUFFER: AdcBuffer = AdcBuffer::new();

    pub struct Sampling {
        _adc1: ADC1,
        _adc2: ADC2,
    }

    pub fn configure(
        adc1_own: ADC1,
        adc2_own: ADC2,
        _common: ADC12_COMMON,
        _dma1: &mut DMA1,
        _rcc: &mut Rcc,
    ) -> Sampling {
        let rcc_regs = unsafe { &*pac::RCC::ptr() };
        rcc_regs.ahb2enr().modify(|_, w| w.adc12en().set_bit());
        cortex_m::asm::delay(16);

        // Аналоговые входы PGA — пины подключает `pga::configure` (PA3/PB14).
        // Сами OPAMP'ы заводят выходы на внутренние ADC1_IN13/ADC2_IN16; на
        // внешних пинах ADC ничего нет.

        // Common clock + multi-ADC config — must be written while ADEN=0 on both.
        // CKMODE = HCLK/4 sync (42.5 MHz @ SYSCLK 170).
        // DUAL = regular simultaneous only, MDMA = 12/10-bit, DMACFG = circular.
        let common = unsafe { &*ADC12_COMMON::ptr() };
        common.ccr().modify(|_, w| {
            w.ckmode().sync_div4();
            w.dual().dual_r();
            unsafe { w.mdma().bits(0b10) };
            w.dmacfg().set_bit()
        });

        let adc1 = unsafe { &*ADC1::ptr() };
        let adc2 = unsafe { &*ADC2::ptr() };

        // Bring up regulator on both ADCs.
        adc1.cr().modify(|_, w| w.deeppwd().clear_bit());
        adc2.cr().modify(|_, w| w.deeppwd().clear_bit());
        adc1.cr().modify(|_, w| w.advregen().set_bit());
        adc2.cr().modify(|_, w| w.advregen().set_bit());
        // T_ADCVREG_STUP ≈ 20 µs; pad to ~47 µs at 170 MHz.
        cortex_m::asm::delay(8000);

        // Single-ended calibration on both.
        adc1.cr().modify(|_, w| w.adcaldif().clear_bit());
        adc2.cr().modify(|_, w| w.adcaldif().clear_bit());
        adc1.cr().modify(|_, w| w.adcal().set_bit());
        adc2.cr().modify(|_, w| w.adcal().set_bit());
        while adc1.cr().read().adcal().bit_is_set() {}
        while adc2.cr().read().adcal().bit_is_set() {}
        // RM0440: ≥ 4 ADC clock cycles after ADCAL clears before ADEN.
        cortex_m::asm::delay(64);

        // Enable both.
        adc1.isr().write(|w| w.adrdy().clear_bit_by_one());
        adc2.isr().write(|w| w.adrdy().clear_bit_by_one());
        adc1.cr().modify(|_, w| w.aden().set_bit());
        adc2.cr().modify(|_, w| w.aden().set_bit());
        while adc1.isr().read().adrdy().bit_is_clear() {}
        while adc2.isr().read().adrdy().bit_is_clear() {}
        adc1.isr().write(|w| w.adrdy().clear_bit_by_one());
        adc2.isr().write(|w| w.adrdy().clear_bit_by_one());

        // Regular sequence. Каналы внутренние:
        //   ADC1_IN13 ← OPAMP1_OUT (канал A, вход PA3 через PGA)
        //   ADC2_IN16 ← OPAMP2_OUT (канал B, вход PB14 через PGA)
        // На G474 ADC1/ADC2 шарят нумерацию каналов, но IN13 ≠ IN16 — каждый
        // ADC честно сэмплит свой OPAMP. Каналы 13 и 16 живут в SMPR2.
        // Sample time увеличен до 47.5 ADC cycles (~1.12 µs @ 42.5 МГц) —
        // OPAMP_OUT низкоомный, но запас не повредит, на 160 кГц TRGO влезает
        // с большим люфтом.
        adc1.sqr1().modify(|_, w| unsafe { w.l().bits(0).sq1().bits(13) });
        adc2.sqr1().modify(|_, w| unsafe { w.l().bits(0).sq1().bits(16) });
        adc1.smpr2().modify(|_, w| w.smp13().cycles47_5());
        adc2.smpr2().modify(|_, w| w.smp16().cycles47_5());

        // Master ADC1: TIM6_TRGO rising trigger, 12-bit, single conversion per trigger.
        // Individual DMAEN stays OFF — DMA is driven by ADC12_COMMON.CCR (MDMA).
        adc1.cfgr().modify(|_, w| {
            w.extsel().tim6_trgo();
            w.exten().rising_edge();
            w.dmaen().clear_bit();
            w.dmacfg().clear_bit();
            w.cont().clear_bit();
            w.res().bits12()
        });

        // Slave ADC2: no external trigger (synced with master); same resolution.
        adc2.cfgr().modify(|_, w| {
            w.exten().disabled();
            w.dmaen().clear_bit();
            w.dmacfg().clear_bit();
            w.cont().clear_bit();
            w.res().bits12()
        });

        // DMA1 channel 2 (index 1): ADC12_COMMON.CDR (32-bit) → ADC_BUFFER, circular, word.
        // CDR layout in 12/10-bit dual mode: master in [11:0], slave in [27:16].
        let dma = unsafe { &*DMA1::ptr() };
        let mux = unsafe { &*DMAMUX::ptr() };
        let ch = dma.ch(1);

        ch.cr().modify(|_, w| w.en().clear_bit());
        while ch.cr().read().en().bit_is_set() {}
        dma.ifcr().write(|w| w.cgif(1).set_bit());

        let cdr_addr = common.cdr().as_ptr() as u32;
        let buf_addr = ADC_BUFFER.ptr() as u32;
        ch.par().write(|w| unsafe { w.bits(cdr_addr) });
        ch.mar().write(|w| unsafe { w.bits(buf_addr) });
        ch.ndtr().write(|w| unsafe { w.bits(LUT_LEN as u32) });

        // DMAMUX channel 1 (DMA1 ch2). ADC1 request ID = 5 also serves CDR in MDMA mode.
        mux.ccr(1).write(|w| unsafe { w.dmareq_id().bits(5) });

        // PSIZE = MSIZE = 32-bit (CDR is a word register; halfword access → AHB ERROR).
        ch.cr().write(|w| unsafe {
            w.pl().bits(0b10);
            w.msize().bits(0b10);
            w.psize().bits(0b10);
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

        // Arm master; slave follows via dual-sync.
        adc1.cr().modify(|_, w| w.adstart().set_bit());

        Sampling {
            _adc1: adc1_own,
            _adc2: adc2_own,
        }
    }

    pub fn snapshot() -> [u32; LUT_LEN] {
        let p = ADC_BUFFER.ptr();
        let mut out = [0u32; LUT_LEN];
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
