# LVDT Conditioner — прогресс milestone 1

План: [/home/sc/.claude/plans/vectorized-booping-snowflake.md](/home/sc/.claude/plans/vectorized-booping-snowflake.md)

Цель milestone 1: DAC sine → dual ADC → IQ → USB CDC. Без калибровки, без CAN, без температуры.

## Сделано

- [x] **Скелет проекта.** `Cargo.toml`, `memory.x`, `.cargo/config.toml`, `probe-rs` runner, `defmt-rtt`, RTIC v2 app с `init`/`idle`. `lib.rs` + `main.rs` сплит для host‑тестов.
- [x] **Часы 170 МГц + HSI48.** `clocks::configure`: HSE 8 → PLL M=2 N=85 R=2 → SYSCLK 170, AHB/APB1/APB2 = NotDivided, PWR в `Range1 { enable_boost: true }` до `rcc.freeze()` (иначе паника в `configure_wait_states`, т.к. `Config::boost(...)` в hal 0.1.0 — no-op), HSI48 включён. (Не верифицировано на железе.)
- [x] **LUT и IQ‑математика.** `lut.rs`: `SINE_LUT_I16` (знаковая ±2047), `DAC_SINE_LUT` (u16 mid‑scale 2048), `cos_from_sine_index`. `iq.rs`: `pack/unpack_dual_adc`, `demodulate_block`, `Iq::magnitude`. Хост‑тесты зелёные (5 passed).
- [x] **DAC sine excitation.** `excitation::configure`: DAC1_CH1 на PA4, TIM6 TRGO @ 160 кГц (PSC=0, ARR=1062, MMS=Update), циркулярный DMA1 ch1 mem→DHR12R1, request line 6 (DAC1_CH1), HFSEL=More160MHz. Подключено в `main.rs::init`. Собирается, на железе не проверено.
- [x] **USB CDC форматтер.** `usb_cdc::format_sample` пишет строку `seq I_a Q_a I_b Q_b\r\n`. Тест есть.

## В работе

ничего

## Дальше (по плану)

- [ ] **Stage 4 — один ADC по триггеру TIM6.** Полировка триггерной цепочки: проверить, что TIM6_TRGO действительно регулярный триггер для ADC1 на G474, или нужен мост через TIM1/TIM2.
- [ ] **Stage 5 — dual simultaneous ADC.** ADC1 (master) + ADC2 (slave), `ADC12_CCR.DUAL = 0b00110`, DMA из `ADC12_CDR` (32‑bit) в double‑buffer `[[u32; 64]; 2]`, half/complete IRQ. Имплементация в `sampling::configure`.
- [ ] **Stage 6 — RTIC‑интеграция IQ.** HW task на DMA half/complete IRQ → spawn `iq_demod` async task → пуш в `heapless::spsc::Queue`.
- [ ] **Stage 7 — USB CDC live stream.** `usb-device` + `usbd-serial`, USB_LP IRQ task для poll, `usb_writer` task дренит очередь.
- [ ] **Stage 8 — реальный LVDT / RC dry‑test.** Замкнуть PA4 → ADC через делитель, потом подключить мост / реальный датчик.

## Открытые вопросы (из плана)

1. Точная частота кварца WeAct‑модуля (8 vs 25 МГц) — проверить при первом запуске.
2. Регулярный ADC trigger из TIM6 TRGO на G474 — подтвердить по RM0440 при коде `sampling.rs`. Fallback: TIM6→TIM1/TIM2 master/slave.
3. API `stm32g4xx-hal 0.1.0` для dual ADC — местами придётся лезть в PAC напрямую (как уже сделано для DAC).

## Известные риски, не проверенные без железа

- DMAMUX request line `DAC1_CH1 = 6` захардкожен (взято из приватного `mux::DmaMuxResources`). Первое подозрение, если DAC молчит.
- HFSEL=More160MHz при HCLK=170 — формально верно, при артефактах попробовать More80mhz.
- Порядок старта `transfer.start()` → `TIM6.CEN=1` — выбран так, чтобы первая точка LUT уже ждала в DMA к первому TRGO. Если первый сэмпл «потеряется» — возможно, надо менять порядок.

## Как обновлять

- Закончил подэтап → перетащи строку из «Дальше» в «Сделано», коротко описав что именно работает.
- Возникла проверенная гипотеза/риск → допиши в «Известные риски».
- Решился открытый вопрос — вычеркни.
