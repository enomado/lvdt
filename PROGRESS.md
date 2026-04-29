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
- [x] **Stage 6 — детектор качества + AGC через встроенный PGA OPAMP1/2 — host‑тесты зелёные, ждёт bring‑up на железе.** Три **разных** ошибки тракта различаются и видны: символ напротив `A`/`B` в defmt и на OLED — `S`aturated > `L`ow > `H`armonic > `.` ok.
  - **`S` Saturated.** Прямой подсчёт raw отсчётов в зоне `0±2 LSB` или `4095±2 LSB` (`stats.sat_count`). Любой ненулевой = клиппинг где‑то в тракте до ADC, IQ‑magnitude уже не пропорциональна реальной — magnitude нельзя верить.
  - **`L` Low signal.** `|Iq|² < 1%·REFERENCE²`. Обрыв вторички / разъём / нет возбуждения. Прячет distortion: на нулевом сигнале метрика гармоник бессмысленна.
  - **`H` Harmonic.** По теореме Парсеваля: `fund_energy = mag² / (IQ_AMPLITUDE² · LUT_LEN/2)`, сравнивается с `total_energy = Σx²`. Если `(total − fund)/total > 2%` — больше энергии вне основной гармоники, чем шум: мягкий клиппинг, нелинейность фронтенда, посторонняя помеха, повреждённый датчик. Не отличает между ними, но фиксирует «синус не синус».
  - Все три метрики собираются одним проходом по блоку в `iq::demodulate_block`. `Accumulator::push` суммирует `sq_sum`/`abs_sum`/`sat_count` так, что один блок с клиппингом во всём окне 64 блока всё равно флагнет `S` (sat_count — total, не average). Покрыто хост‑тестами, в т.ч. priority `S>L>H>.`.
  - **AGC через OPAMPx PGA.** OPAMP1 (PA3 → ADC1_IN13) и OPAMP2 (PB14 → ADC2_IN16) подняты в PGA‑mode (`VM_SEL=Pga`, `VP_SEL=Vinp1`, `OPAINTOEN=Adcchannel`), стартовый gain ×2. Раз в окно усреднения (`SMOOTHING_BLOCKS=64` блоков ≈ 25.6 мс) `agc::tick`: при `clipping` шаг вниз без оглядки на mag, при `mag<25%` шаг вверх, при `mag>75%` шаг вниз, иначе hold. Между шагами lockout 1 окно. Шкала ×2/×4/×8/×16/×32/×64 — 6 ступеней, всего ×32 динамики; чтобы не зацикливаться, окно 25–75% (3×) шире фактора смены 2×. Decision pure, host‑тестируется без железа.
  - **Pinmap**: переезд с PA0/PA1 (прямой ADC) на PA3/PB14 (через OPAMP). Под loopback‑тест нужно перепаять PA4→PA3 и PA4→PB14; PA0/PA1 остаются свободны.
  - **Гейн на экране**: формат строки `A.x2  100.0%` / `BSx64  3.2%` (label + quality + 'x' + gain + % mag). 12 chars × 10 px = ровно 128 px ширины OLED.
  - **Внимание**: даже gain ×2 на full‑swing loopback (DAC peak‑to‑peak 3.3 В) даст моментальный клиппинг — AGC сразу шагнёт вниз, но `step_down` от ×2 уже упирается в нижнюю рельсу. Loopback теперь имеет смысл только как hot‑signal проверка `S` ⇒ `step_down` (видим тестовый цикл AGC). Реальная работа — на вторичках LVDT с ~50–500 мВ.

- [x] **Stage 5 — dual simultaneous ADC + первичный IQ‑demod — проверено на железе 2026‑04‑28.** ADC1 (master, IN1=PA0) + ADC2 (slave, IN2=PA1), regular simultaneous, DMA1 ch2 ← `ADC12_COMMON.CDR` → `[u32; 64]` circular. На замкнутых PA4→PA0 и PA4→PA1: `A[|IQ|] ≈ B[|IQ|] ≈ 1.34e8` (теоретический предел `64·2047²/2`), расхождение A−B стабильно ~80 ppm — это разница калибровок двух физических ADC. На PA0→GND, PA1 свободен: `A[|IQ|] = 0`, `B[|IQ|] ≈ 10⁴…7·10⁵` от наводки — динамика когерентного детектора 4–5 порядков, A и B полностью независимы.
  - `ADC12_COMMON.CCR`: `CKMODE=sync_div4`, `DUAL=dual_r` (regular only = 0b00110), `MDMA=0b10` (12/10‑bit), `DMACFG=1` (circular). Все четыре поля выставлены **до** `ADEN=1` — после ADEN их менять нельзя (RM0440).
  - Каждый ADC калибруется отдельно: `DEEPPWD→0`, `ADVREGEN→1`, delay, `ADCAL` single‑ended, `ADRDY` ack, `ADEN`.
  - Master CFGR: `EXTSEL=Tim6Trgo`, `EXTEN=rising`, `RES=12`, `CONT=0`, **`DMAEN=0`** (индивидуальный DMA выключен — данные текут через `CCR.MDMA` из общего CDR). Slave CFGR: `EXTEN=disabled`, остальное идентично.
  - DMA1 ch2: `PAR = ADC12_COMMON.cdr().as_ptr()`, NDTR=64, `PSIZE=MSIZE=0b10` (32‑бит; CDR — word‑регистр, halfword‑access = AHB ERROR, тот же квирк, что DAC.DHR12Rx). DMAMUX req id=5 (ADC1 — дёргает и CDR в MDMA‑режиме). Старт `ADC1.CR.ADSTART=1`, slave следует через dual‑sync.
  - CDR layout 12/10‑bit: master в [11:0], slave в [27:16] — точно тот формат, который ждёт `iq::unpack_dual_adc`. Лог в main также прогоняет `iq::demodulate_block` и печатает `|Iq.a|`, `|Iq.b|` рядом с min/max/pp.
  - **Главный квирк, на котором поймались:** на STM32G4 нумерация каналов ADC12 *общая*: `ADC12_IN1 = PA0`, `ADC12_IN2 = PA1`, … — то есть IN1 на ADC1 и IN1 на ADC2 указывают на один и тот же пин (PA0). Изначально я поставил `SQR1.SQ1 = 1` на оба ADC — оба честно сэмплили PA0, отчего magnitudes совпадали до 0.02% и не реагировали на отключение PA1. Чтобы читать разные пины, master нужно `SQ1=1` (PA0), slave — `SQ1=2` (PA1) и `SMPR1.SMP2` (а не `SMP1`). Тот же принцип будет на третьем/четвёртом канале.
  - Закрыл «открытый вопрос №3»: hal 0.1.0 для dual ADC не использован вовсе, всё через PAC; зато весь init умещается в одну функцию ~80 строк.

## В работе

- [ ] **Bring‑up Stage 6 на железе.** Перепайка PA4 → PA3 (канал A через OPAMP1) и PA4 → PB14 (канал B через OPAMP2) для тестового loopback. На full‑swing ожидаем сценарий `S` → AGC step_down (зацикливается на ×2). Чтобы увидеть полный цикл AGC, нужен делитель PA4 → ~10 мВ или реальные вторички. Проверить: символ `S` пропадает после нескольких step_down’ов, мерим magnitude после каждой смены gain — должен расти/падать ровно в 2×.

## Дальше (по плану)
- [ ] **Stage 7 — USB CDC live stream.** `usb-device` + `usbd-serial`, USB_LP IRQ task для poll, `usb_writer` task дренит очередь.
- [ ] **Stage 8 — реальный LVDT / RC dry‑test.** PA4 → делитель → PA3/PB14, потом подключить мост / реальный датчик.
- [ ] **AGC: связка A/B по max gain.** Для LVDT обе вторички должны видеть одинаковый gain, иначе позиция `(B−A)/(B+A)` смещается. Сейчас AGC решает по каналам независимо. После реального датчика добавить «slave‑lock»: на смену gain на любом канале — синхронно крутить второй до той же ступени.

## Открытые вопросы (из плана)

1. ~~Точная частота кварца WeAct‑модуля (8 vs 25 МГц) — проверить при первом запуске.~~ Подтверждён 8 МГц.
2. ~~Регулярный ADC trigger из TIM6 TRGO на G474~~ — подтверждён по PAC `stm32g4 0.16.0`: `EXTSEL::Tim6Trgo = 13` для ADC1/2, прямой триггер, мост не нужен.
3. ~~API `stm32g4xx-hal 0.1.0` для dual ADC~~ — не использовали hal вовсе, всё в PAC; работает.

## Известные риски

- ~~DMAMUX request line `DAC1_CH1 = 6` — первое подозрение, если DAC молчит.~~ Подтверждён, не он.
- HFSEL=More160MHz при HCLK=170 — формально верно; на железе работает.
- ~~Для ADC DMA на G4 учесть тот же квирк что и у DAC: размер шинного доступа peripheral‑регистра должен соответствовать его реальной accessibility, иначе AHB ERROR. Для ADC1/2 dual `CDR` это 32‑bit регистр — PSIZE=32.~~ Подтверждено на железе: PSIZE=MSIZE=32 на CDR работает.
- Нумерация ADC1/ADC2 каналов на G4 общая (`ADC12_INx`), а не «своя на каждый ADC». При добавлении третьего/четвёртого канала или ADC3/4/5 ставить SQR/SMPR на разные `INx` — иначе будем сэмплить один пин на оба ADC.
- ADC measurement через min/max/pp на 64‑точечном блоке *не различает* сигнал и наводку на high‑Z пине: оба дают full‑scale. Когерентный `iq::demodulate_block` различает (динамика 4–5 порядков). Любая дальнейшая «есть/нет сигнала» проверка должна идти через magnitude, не через pp.
- **PGA `step_down` upper bound на ×2.** Когда вход уже горячий и AGC просит step_down с ×2, упирается в нижнюю ступень — клиппинг остаётся, символ `S` не уходит. Это ожидаемо: ниже ×2 PGA OPAMP'а не делает (только Follower mode = unity, но он не PGA). Если такое стабильно на реальном датчике — нужен входной делитель в железе (или включать OPAMP в Follower вместо PGA, но это уже не AGC). На loopback‑тесте PA4→PA3 это будет именно такой сценарий.
- **A vs B при разных gain.** AGC решает per‑channel; если канал A на ×8, а B на ×16, magnitudes уже не сравнимы напрямую — формула позиции `(B−A)/(B+A)` сместится. Для LVDT в production это надо нормировать (`B/gain_b` vs `A/gain_a`) или принудительно сводить gain к одной ступени. Пока — задача.
- **PB8 на STM32G474 LQFP48 — НЕ ИСПОЛЬЗОВАТЬ.** Это BOOT0 sample pin при дефолтных option bytes (`nSWBOOT0=1`). Любая внешняя подтяжка к VDD на PB8 (типичный pull‑up I2C SCL на дисплейном модуле) держит BOOT0 high при reset → чип уходит в системный bootloader (PC=`0x1FFF0xxx`), наша прошивка не стартует, RTT/defmt молчит, plata выглядит «мёртвой». Поймали на стадии добавления SSD1306: `display::init` стандартно вешали SCL на PB8 — после первой пайки чип перестал бутиться. Лечение: SCL переехал на PA15 (PB9 для SDA остался). Альтернативное лечение — сбросить `nSWBOOT0=0` в FLASH_OPTR (бит 25), тогда BOOT0 берётся из `nBOOT0`, но это лезть в OB. Проверка диагноза без перепайки: `probe-rs read --chip ... b32 0xE000EDF0 1` и halt через DCRSR/DCRDR — если PC в `0x1FFF0000…0x1FFF7FFF`, это bootloader. И/или `probe-rs read ... b32 0x40022020 1` → бит 25 в FLASH_OPTR.

## Как обновлять

- Закончил подэтап → перетащи строку из «Дальше» в «Сделано», коротко описав что именно работает.
- Возникла проверенная гипотеза/риск → допиши в «Известные риски».
- Решился открытый вопрос — вычеркни.
