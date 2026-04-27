use crate::lut::LUT_LEN;

pub type AdcDmaBlock = [u32; LUT_LEN];
pub type AdcDmaBuffers = [AdcDmaBlock; 2];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadyBuffer {
    First,
    Second,
}

impl ReadyBuffer {
    pub const fn index(self) -> usize {
        match self {
            ReadyBuffer::First => 0,
            ReadyBuffer::Second => 1,
        }
    }
}

#[cfg(target_arch = "arm")]
pub fn configure() {
    // TODO: configure ADC1+ADC2 dual regular simultaneous mode and DMA from ADC12_CDR.
}
