# LVDT Conditioner — прогресс milestone 1

План: [/home/sc/.claude/plans/vectorized-booping-snowflake.md](/home/sc/.claude/plans/vectorized-booping-snowflake.md)

Цель milestone 1: DAC sine → dual ADC → IQ → USB CDC. Без калибровки, без CAN, без температуры.

## Сделано

- [x] **Скелет проекта.** `Cargo.toml`, `memory.x`, `.cargo/config.toml`, `probe-rs` runner, `defmt-rtt`, RTIC v2 app с `init`/`idle`. `lib.rs` + `main.rs` сплит для host‑тестов.
- [x] **Часы 170 МГц + HSI48 — проверено на железе 2026‑04‑28.** `clocks::configure`: HSE 8 → PLL M=2 N=85 R=2 → SYSCLK 170, AHB/APB1/APB2 = NotDivided, PWR в `Range1 { enable_boost: true }` до `rcc.freeze()` (иначе паника в `configure_wait_states`, т.к. `Config::boost(...)` в hal 0.1.0 — no-op), HSI48 включён. Кварц подтверждён 8 МГц.
- [x] **LUT и IQ‑математика.** `lut.rs`: `SINE_LUT_I16` (знаковая ±2047), `DAC_SINE_LUT` (u16 mid‑scale 2048), `cos_from_sine_index`. `iq.rs`: `pack/unpack_dual_adc`, `demodulate_block`, `Iq::magnitude`. Хост‑тесты зелёные (5 passed).
- [x] **DAC sine excitation — проверено осциллографом 2026‑04‑28.** DAC1_CH1 на PA4, TIM6 TRGO @ ~160 кГц (PSC=0, ARR=1061, MMS=Update), циркулярный DMA1 ch1 mem→DHR12R1, DMAMUX request 6, HFSEL=More160MHz. Синус 2.5 кГц, размах 0…3.3 В.
  - **G4‑квирк, на котором споткнулись:** `DAC.DHR12Rx` не принимает 16‑битные шинные доступы — halfword‑access даёт AHB ERROR → DMA TEIF → канал самоотключается → DAC латчит DMAUDR1. Лечится `PSIZE=32, MSIZE=16`. DMA читает u16 из LUT и пишет 32‑бит в DAC, старшие 4 бита DAC игнорирует.
  - DMA настроен через PAC напрямую — `stm32g4xx-hal 0.1.0` `into_memory_to_peripheral_transfer` принудительно ставит PSIZE=MSIZE.
  - Также: `EGR.UG` убран (генерил спурьезный TRGO до готовности DMA), DAC.DMAEN1 поднимается **после** DMA channel EN, DAC.SR.DMAUDR1 явно сбрасывается до взведения DMAEN.
- [x] **USB CDC форматтер.** `usb_cdc::format_sample` пишет строку `seq I_a Q_a I_b Q_b\r\n`. Тест есть.
- [x] **DBGMCU keep‑alive.** В `clocks::configure` ставим `DBGMCU.CR.DBG_SLEEP/STOP/STANDBY = 1`, иначе после первой прошивки idle WFI глушит SWD и STLink V2 без NRST не может прицепиться (приходится BOOT0+reset). После патча — `cargo run` без танцев.
- [x] **Stage 4 — один ADC1 по триггеру TIM6 — проверено на железе 2026‑04‑28.** ADC1 IN1 (PA0) с замыканием PA4→PA0; CKMODE=HCLK/4 sync (42.5 МГц), `EXTSEL=Tim6Trgo` (= 13), `EXTEN=rising`, single conversion, sample time 24.5 циклов. DMA1 ch2 → `static ADC_BUFFER: [u16; 64]` circular halfword, DMAMUX request 5. HW task `DMA1_CH2` дренит TC и каждые 1024 буфера лоgает min/max/mean/pp.
  - **Результат**: `mean ≈ 2045` (midscale 2048), `pp ≈ 4021` (full scale 4095), TC ≈ 2355/с при расчётных 2500 — синус DAC проходит через ADC до краёв шкалы.
  - **Sequence**: `DEEPPWD=0` → `ADVREGEN=1` → delay ~50 мкс → single‑ended `ADCAL` → wait clear → `ADRDY` clear (write‑1) → `ADEN=1` → wait `ADRDY` → SQR1/SMPR1/CFGR → DMA arm → `ADSTART=1`.
  - **Quirk svd2rust 0.16**: `ADRDY` это rc_w1, у врайтера нет `set_bit()` — `write(|w| w.adrdy().clear_bit_by_one())`.
  - Закрыл «открытый вопрос №2»: TIM6_TRGO на G474 — прямой regular `EXTSEL` для ADC1/2, мост через TIM1/TIM2 не нужен.

## В работе

ничего

## Дальше (по плану)
- [ ] **Stage 5 — dual simultaneous ADC.** ADC1 (master) + ADC2 (slave), `ADC12_CCR.DUAL = 0b00110`, DMA из `ADC12_CDR` (32‑bit) в double‑buffer `[[u32; 64]; 2]`, half/complete IRQ. Имплементация в `sampling::configure`.
- [ ] **Stage 6 — RTIC‑интеграция IQ.** HW task на DMA half/complete IRQ → spawn `iq_demod` async task → пуш в `heapless::spsc::Queue`.
- [ ] **Stage 7 — USB CDC live stream.** `usb-device` + `usbd-serial`, USB_LP IRQ task для poll, `usb_writer` task дренит очередь.
- [ ] **Stage 8 — реальный LVDT / RC dry‑test.** Замкнуть PA4 → ADC через делитель, потом подключить мост / реальный датчик.

## Открытые вопросы (из плана)

1. ~~Точная частота кварца WeAct‑модуля (8 vs 25 МГц) — проверить при первом запуске.~~ Подтверждён 8 МГц.
2. ~~Регулярный ADC trigger из TIM6 TRGO на G474~~ — подтверждён по PAC `stm32g4 0.16.0`: `EXTSEL::Tim6Trgo = 13` для ADC1/2, прямой триггер, мост не нужен.
3. API `stm32g4xx-hal 0.1.0` для dual ADC — местами придётся лезть в PAC напрямую (как уже сделано для DAC).

## Известные риски

- ~~DMAMUX request line `DAC1_CH1 = 6` — первое подозрение, если DAC молчит.~~ Подтверждён, не он.
- HFSEL=More160MHz при HCLK=170 — формально верно; на железе работает.
- Для ADC DMA на G4 учесть тот же квирк что и у DAC: размер шинного доступа peripheral‑регистра должен соответствовать его реальной accessibility, иначе AHB ERROR. Для ADC1/2 dual `CDR` это 32‑bit регистр — PSIZE=32.

## Как обновлять

- Закончил подэтап → перетащи строку из «Дальше» в «Сделано», коротко описав что именно работает.
- Возникла проверенная гипотеза/риск → допиши в «Известные риски».
- Решился открытый вопрос — вычеркни.
