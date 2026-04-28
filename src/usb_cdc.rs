use crate::iq::DemodulatedSample;

pub type IqQueue<const N: usize> = heapless::spsc::Queue<DemodulatedSample, N>;

/// `seq A_mag_pct A_phase_mrad B_mag_pct B_phase_mrad\r\n`
///
/// `*_mag_pct` — magnitude as % of the ideal full-swing loopback reference
/// (`iq::REFERENCE_MAGNITUDE`). `*_phase_mrad` — phase in milliradians
/// relative to the DAC excitation (0 ≡ in-phase with the LUT, negative ≡ the
/// sampled signal lags excitation).
pub fn format_sample<const N: usize>(
    sample: DemodulatedSample,
    out: &mut heapless::String<N>,
) -> Result<(), core::fmt::Error> {
    use core::fmt::Write;

    let a = sample.a.deviation();
    let b = sample.b.deviation();
    write!(
        out,
        "{} {:.3} {:.1} {:.3} {:.1}\r\n",
        sample.sequence, a.mag_pct, a.phase_mrad, b.mag_pct, b.phase_mrad,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::Iq;
    use crate::lut::SINE_LUT_I16;

    #[test]
    fn formats_usb_line_full_loopback() {
        // Synthesize an Iq that corresponds to perfect in-phase loopback so
        // we can pin the human-readable values.
        let i: i32 = SINE_LUT_I16.iter().map(|s| (*s as i32).pow(2)).sum();
        let sample = DemodulatedSample {
            sequence: 3,
            a: Iq { i, q: 0 },
            b: Iq { i: i / 2, q: 0 },
        };
        let mut line = heapless::String::<64>::new();
        format_sample(sample, &mut line).unwrap();
        assert_eq!(line.as_str(), "3 100.000 0.0 50.000 0.0\r\n");
    }
}
