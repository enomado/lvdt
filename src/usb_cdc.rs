use crate::iq::DemodulatedSample;

pub type IqQueue<const N: usize> = heapless::spsc::Queue<DemodulatedSample, N>;

pub fn format_sample<const N: usize>(
    sample: DemodulatedSample,
    out: &mut heapless::String<N>,
) -> Result<(), core::fmt::Error> {
    use core::fmt::Write;

    write!(
        out,
        "{} {} {} {} {}\r\n",
        sample.sequence, sample.a.i, sample.a.q, sample.b.i, sample.b.q
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::Iq;

    #[test]
    fn formats_usb_line() {
        let sample = DemodulatedSample {
            sequence: 3,
            a: Iq { i: 10, q: -20 },
            b: Iq { i: 30, q: -40 },
        };
        let mut line = heapless::String::<64>::new();

        format_sample(sample, &mut line).unwrap();

        assert_eq!(line.as_str(), "3 10 -20 30 -40\r\n");
    }
}
