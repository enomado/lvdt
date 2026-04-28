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
- [x] **Stage 5 — dual simultaneous ADC + первичный IQ‑demod — проверено на железе 2026‑04‑28.** ADC1 (master, IN1=PA0) + ADC2 (slave, IN2=PA1), regular simultaneous, DMA1 ch2 ← `ADC12_COMMON.CDR` → `[u32; 64]` circular. На замкнутых PA4→PA0 и PA4→PA1: `A[|IQ|] ≈ B[|IQ|] ≈ 1.34e8` (теоретический предел `64·2047²/2`), расхождение A−B стабильно ~80 ppm — это разница калибровок двух физических ADC. На PA0→GND, PA1 свободен: `A[|IQ|] = 0`, `B[|IQ|] ≈ 10⁴…7·10⁵` от наводки — динамика когерентного детектора 4–5 порядков, A и B полностью независимы.
  - `ADC12_COMMON.CCR`: `CKMODE=sync_div4`, `DUAL=dual_r` (regular only = 0b00110), `MDMA=0b10` (12/10‑bit), `DMACFG=1` (circular). Все четыре поля выставлены **до** `ADEN=1` — после ADEN их менять нельзя (RM0440).
  - Каждый ADC калибруется отдельно: `DEEPPWD→0`, `ADVREGEN→1`, delay, `ADCAL` single‑ended, `ADRDY` ack, `ADEN`.
  - Master CFGR: `EXTSEL=Tim6Trgo`, `EXTEN=rising`, `RES=12`, `CONT=0`, **`DMAEN=0`** (индивидуальный DMA выключен — данные текут через `CCR.MDMA` из общего CDR). Slave CFGR: `EXTEN=disabled`, остальное идентично.
  - DMA1 ch2: `PAR = ADC12_COMMON.cdr().as_ptr()`, NDTR=64, `PSIZE=MSIZE=0b10` (32‑бит; CDR — word‑регистр, halfword‑access = AHB ERROR, тот же квирк, что DAC.DHR12Rx). DMAMUX req id=5 (ADC1 — дёргает и CDR в MDMA‑режиме). Старт `ADC1.CR.ADSTART=1`, slave следует через dual‑sync.
  - CDR layout 12/10‑bit: master в [11:0], slave в [27:16] — точно тот формат, который ждёт `iq::unpack_dual_adc`. Лог в main также прогоняет `iq::demodulate_block` и печатает `|Iq.a|`, `|Iq.b|` рядом с min/max/pp.
  - **Главный квирк, на котором поймались:** на STM32G4 нумерация каналов ADC12 *общая*: `ADC12_IN1 = PA0`, `ADC12_IN2 = PA1`, … — то есть IN1 на ADC1 и IN1 на ADC2 указывают на один и тот же пин (PA0). Изначально я поставил `SQR1.SQ1 = 1` на оба ADC — оба честно сэмплили PA0, отчего magnitudes совпадали до 0.02% и не реагировали на отключение PA1. Чтобы читать разные пины, master нужно `SQ1=1` (PA0), slave — `SQ1=2` (PA1) и `SMPR1.SMP2` (а не `SMP1`). Тот же принцип будет на третьем/четвёртом канале.
  - Закрыл «открытый вопрос №3»: hal 0.1.0 для dual ADC не использован вовсе, всё через PAC; зато весь init умещается в одну функцию ~80 строк.

## В работе

ничего

## Дальше (по плану)
- [ ] **Stage 6 — RTIC‑интеграция IQ.** HW task на DMA half/complete IRQ → spawn `iq_demod` async task → пуш в `heapless::spsc::Queue`.
- [ ] **Stage 7 — USB CDC live stream.** `usb-device` + `usbd-serial`, USB_LP IRQ task для poll, `usb_writer` task дренит очередь.
- [ ] **Stage 8 — реальный LVDT / RC dry‑test.** Замкнуть PA4 → ADC через делитель, потом подключить мост / реальный датчик.

## Открытые вопросы (из плана)

1. ~~Точная частота кварца WeAct‑модуля (8 vs 25 МГц) — проверить при первом запуске.~~ Подтверждён 8 МГц.
2. ~~Регулярный ADC trigger из TIM6 TRGO на G474~~ — подтверждён по PAC `stm32g4 0.16.0`: `EXTSEL::Tim6Trgo = 13` для ADC1/2, прямой триггер, мост не нужен.
3. ~~API `stm32g4xx-hal 0.1.0` для dual ADC~~ — не использовали hal вовсе, всё в PAC; работает.

## Известные риски

- ~~DMAMUX request line `DAC1_CH1 = 6` — первое подозрение, если DAC молчит.~~ Подтверждён, не он.
- HFSEL=More160MHz при HCLK=170 — формально верно; на железе работает.
- Для ADC DMA на G4 учесть тот же квирк что и у DAC: размер шинного доступа peripheral‑регистра должен соответствовать его реальной accessibility, иначе AHB ERROR. Для ADC1/2 dual `CDR` это 32‑bit регистр — PSIZE=32.

## Как обновлять

- Закончил подэтап → перетащи строку из «Дальше» в «Сделано», коротко описав что именно работает.
- Возникла проверенная гипотеза/риск → допиши в «Известные риски».
- Решился открытый вопрос — вычеркни.
