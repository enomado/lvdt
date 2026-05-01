use crate::lut::{DAC_SINE_LUT, LUT_LEN, TIM6_ARR};

pub struct ExcitationPlan {
    pub tim6_arr: u16,
    pub lut: &'static [u16; LUT_LEN],
}

pub const PLAN: ExcitationPlan = ExcitationPlan {
    tim6_arr: TIM6_ARR,
    lut: &DAC_SINE_LUT,
};

#[cfg(target_arch = "arm")]
pub use arm::{configure, Excitation};

#[cfg(target_arch = "arm")]
mod arm {
    use super::DAC_SINE_LUT;
    use crate::lut::{LUT_LEN, TIM6_ARR};
    use stm32g4xx_hal::{
        pac::{self, DAC1, DMA1, DMAMUX, TIM6},
        rcc::Rcc,
        stm32::dac1::mcr::HFSEL,
    };

    pub struct Excitation {
        _tim6: TIM6,
    }

    pub fn configure(_dac1: DAC1, tim6_own: TIM6, _dma1: &mut DMA1, _rcc: &mut Rcc) -> Excitation {
        let tim6 = &tim6_own;
        let rcc_regs = unsafe { &*pac::RCC::ptr() };
        rcc_regs.ahb1enr().modify(|_, w| {
            w.dma1en().set_bit();
            w.dmamux1en().set_bit()
        });
        rcc_regs.ahb2enr().modify(|_, w| w.dac1en().set_bit());
        rcc_regs.apb1enr1().modify(|_, w| w.tim6en().set_bit());

        let dac = unsafe { &*DAC1::ptr() };
        dac.mcr().modify(|_, w| unsafe {
            w.hfsel().variant(HFSEL::More160mhz);
            w.mode(0).bits(0b000)
        });
        dac.cr().modify(|_, w| {
            w.tsel1().tim6trgo();
            w.ten1().set_bit();
            w.en1().set_bit()
        });

        tim6.psc().write(|w| unsafe { w.psc().bits(0) });
        tim6.arr().write(|w| unsafe { w.bits(TIM6_ARR as u32) });
        tim6.cr2().modify(|_, w| w.mms().update());
        tim6.sr().write(|w| w.uif().clear_bit());

        // Manual DMA1 channel 1 setup, no hal in the loop.
        let dma = unsafe { &*DMA1::ptr() };
        let mux = unsafe { &*DMAMUX::ptr() };
        let ch = dma.ch(0);

        // Make sure channel is disabled before reconfiguring.
        ch.cr().modify(|_, w| w.en().clear_bit());
        while ch.cr().read().en().bit_is_set() {}

        // Clear all flags for channel 1 (CGIF1 clears all of GIF/TCIF/HTIF/TEIF).
        dma.ifcr().write(|w| w.cgif(0).set_bit());

        let lut_addr = (&DAC_SINE_LUT as *const [u16; LUT_LEN]) as u32;
        let dhr_addr = unsafe { (*DAC1::ptr()).dhr12r(0).as_ptr() as u32 };

        ch.par().write(|w| unsafe { w.bits(dhr_addr) });
        ch.mar().write(|w| unsafe { w.bits(lut_addr) });
        ch.ndtr().write(|w| unsafe { w.bits(LUT_LEN as u32) });

        // DMAMUX channel 0 is wired to DMA1 channel 1.
        mux.ccr(0).write(|w| unsafe { w.dmareq_id().bits(6) }); // DAC1_CH1

        // PSIZE=32 is required: DAC.DHR12Rx on G4 only accepts 32-bit AHB
        // writes, a 16-bit halfword write returns ERROR → DMA TEIF, channel
        // halts and DAC latches DMAUDR1. MSIZE stays 16 (LUT is u16); DMA
        // zero-extends each sample to 32 bits, top 4 bits are ignored by DAC.
        ch.cr().write(|w| unsafe {
            w.pl().bits(0b10);
            w.msize().bits(0b01);
            w.psize().bits(0b10);
            w.minc().set_bit();
            w.pinc().clear_bit();
            w.circ().set_bit();
            w.dir().set_bit();
            w.mem2mem().clear_bit();
            w.tcie().clear_bit();
            w.htie().clear_bit();
            w.teie().clear_bit();
            w.en().clear_bit()
        });

        ch.cr().modify(|_, w| w.en().set_bit());

        dac.sr().write(|w| w.dmaudr1().set_bit());
        dac.cr().modify(|_, w| w.dmaen1().set_bit());

        tim6.cr1().modify(|_, w| w.cen().set_bit());

        Excitation { _tim6: tim6_own }
    }
}
