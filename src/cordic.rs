#[cfg(target_arch = "arm")]
pub use arm::{
    CordicHw,
    configure,
    deviation,
};

#[cfg(target_arch = "arm")]
mod arm {
    use stm32g4xx_hal::cordic::op::ATan2Magnitude;
    use stm32g4xx_hal::cordic::prec::P60;
    use stm32g4xx_hal::cordic::types::{
        I1F31,
        Q31,
    };
    use stm32g4xx_hal::cordic::{
        Cordic,
        Ext as _,
    };
    use stm32g4xx_hal::pac::CORDIC;
    use stm32g4xx_hal::rcc::Rcc;

    use crate::iq::{
        Deviation,
        Iq,
        REFERENCE_MAGNITUDE,
    };

    pub type CordicHw = Cordic<Q31, Q31, P60, ATan2Magnitude>;

    pub fn configure(cordic: CORDIC, rcc: &mut Rcc) -> CordicHw {
        cordic.constrain(rcc).freeze::<Q31, Q31, P60, ATan2Magnitude>()
    }

    /// Magnitude scale: CORDIC returns √(x²+y²)/2³¹ in Q1.31; multiplying by
    /// 2³¹ recovers the raw magnitude, then `100 / REFERENCE_MAGNITUDE`
    /// converts to percent of full-loopback amplitude.
    const MAG_SCALE: f32 = (1u64 << 31) as f32 * 100.0 / REFERENCE_MAGNITUDE;
    /// CORDIC returns phase / π in Q1.31; multiply by π·1000 to get mrad.
    const PHASE_SCALE: f32 = core::f32::consts::PI * 1000.0;

    pub fn deviation(cordic: &mut CordicHw, iq: Iq) -> Deviation {
        cordic.start((I1F31::from_bits(iq.i), I1F31::from_bits(iq.q)));
        let (phase_q31, mag_q31) = cordic.result();
        Deviation {
            mag_pct:    mag_q31.to_num::<f32>() * MAG_SCALE,
            phase_mrad: phase_q31.to_num::<f32>() * PHASE_SCALE,
        }
    }
}
