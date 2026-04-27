# LVDT Conditioner

First milestone firmware for an STM32G474CEU6-based LVDT signal conditioner.

The target signal path is:

```text
TIM6 TRGO @ ~160 kHz
  -> DAC1_CH1 PA4 sine excitation, 64 samples per cycle
  -> ADC1 + ADC2 dual regular simultaneous sampling
  -> DMA circular blocks of 64 packed ADC pairs
  -> IQ demodulation
  -> USB CDC text stream
```

Current milestone status:

- Rust/RTIC project skeleton is in place.
- `lut` contains the fixed 64-point sine tables and timing constants.
- `iq` contains host-testable packed dual-ADC unpacking and IQ demodulation.
- `clocks`, `excitation`, `sampling`, and `usb_cdc` define the firmware boundaries for the next hardware pass.

## Build

Install the embedded target once:

```sh
rustup target add thumbv7em-none-eabihf
```

Check the firmware:

```sh
cargo check
```

Run host-side math tests:

```sh
cargo test --target x86_64-unknown-linux-gnu
```

Flash and run with probe-rs:

```sh
cargo run --release
```

The first board bring-up should confirm the WeAct HSE frequency before enabling the full PLL/USB clock tree.
