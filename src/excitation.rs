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
        dma::{
            channel::{DMAExt, C},
            config::{DmaConfig, Priority},
            traits::TargetAddress,
            transfer::ConstTransfer,
            MemoryToPeripheral, Transfer, TransferExt,
        },
        pac::{self, DAC1, DMA1, TIM6},
        rcc::Rcc,
        stm32::dac1::mcr::HFSEL,
    };

    pub struct DacCh1Dma;

    unsafe impl TargetAddress<MemoryToPeripheral> for DacCh1Dma {
        type MemSize = u16;
        fn address(&self) -> u32 {
            unsafe { (*DAC1::ptr()).dhr12r(0).as_ptr() as u32 }
        }
        // DMAMUX request line for DAC1_CH1 (RM0440, table for DMAMUX1 inputs).
        const REQUEST_LINE: Option<u8> = Some(6);
    }

    pub type Excitation = Transfer<
        C<DMA1, 0>,
        DacCh1Dma,
        MemoryToPeripheral,
        &'static [u16; LUT_LEN],
        ConstTransfer,
    >;

    pub fn configure(_dac1: DAC1, tim6: TIM6, dma1: DMA1, rcc: &mut Rcc) -> Excitation {
        let rcc_regs = unsafe { &*pac::RCC::ptr() };
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
            w.dmaen1().set_bit();
            w.en1().set_bit()
        });

        tim6.psc().write(|w| unsafe { w.psc().bits(0) });
        tim6.arr().write(|w| unsafe { w.bits(TIM6_ARR as u32) });
        tim6.cr2().modify(|_, w| w.mms().update());
        tim6.egr().write(|w| w.ug().set_bit());
        tim6.sr().write(|w| w.uif().clear_bit());

        let channels = dma1.split(rcc);
        let config = DmaConfig::default()
            .priority(Priority::High)
            .circular_buffer(true)
            .memory_increment(true)
            .peripheral_increment(false)
            .transfer_complete_interrupt(false)
            .half_transfer_interrupt(false);

        let mut transfer = channels
            .ch1
            .into_memory_to_peripheral_transfer(DacCh1Dma, &DAC_SINE_LUT, config);
        transfer.start(|_| {});

        tim6.cr1().modify(|_, w| w.cen().set_bit());

        transfer
    }
}
