//! Однокнопочный декодер «как у фонариков»: распознаёт паттерны S, L, SS, LSS.
//!
//! Логика чисто host‑testable: `ButtonFsm::step(now_ms, raw_pressed)` принимает
//! текущее время в миллисекундах и сырое состояние пина, возвращает `Some(action)`
//! на финализированном паттерне. Никаких таймеров, никакой работы с железом —
//! железо живёт в `main.rs`, тут только FSM.
//!
//! ## Идея
//!
//! Один клик ≠ паттерн. После каждого release мы ждём `INTERCLICK_GAP_MS`
//! — если за это время не пришёл новый press, серия закрывается и
//! декодируется. Иначе продолжаем накапливать клики (S/L) в буфер.
//!
//! - press длительности < `LONG_PRESS_MS` → `S`
//! - press длительности ≥ `LONG_PRESS_MS` → `L`
//! - дебаунс: изменение raw‑состояния игнорируется, пока не продержится
//!   стабильно `DEBOUNCE_MS`
//!
//! Паттерны:
//!
//! | sequence | action            |
//! |----------|-------------------|
//! | `S`      | Short             |
//! | `L`      | Long              |
//! | `SS`     | DoubleShort       |
//! | `LSS`    | LongShortShort    |
//!
//! Нераспознанные последовательности (например `SSS`, `LL`, `LS`) тихо
//! отбрасываются после gap timeout — это даёт пользователю «escape», когда он
//! передумал.

const DEBOUNCE_MS: u32 = 15;
const LONG_PRESS_MS: u32 = 500;
const INTERCLICK_GAP_MS: u32 = 350;
const MAX_PATTERN: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickKind {
    Short,
    Long,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonAction {
    Short,
    Long,
    DoubleShort,
    LongShortShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Кнопка отпущена и нет активной серии — ждём первый press.
    Idle,
    /// Кнопка нажата (после дебаунса). `since_ms` — момент признания нажатия.
    Pressed { since_ms: u32 },
    /// Кнопка отпущена внутри серии. Если до `last_release_ms +
    /// INTERCLICK_GAP_MS` придёт новый press — продолжаем серию, иначе —
    /// финализируем паттерн.
    Released { last_release_ms: u32 },
}

pub struct ButtonFsm {
    state:          State,
    pattern:        heapless::Vec<ClickKind, MAX_PATTERN>,
    /// Последнее принятое (после дебаунса) raw‑значение.
    last_raw:       bool,
    /// Момент последнего наблюдённого изменения raw — точка отсчёта дебаунса.
    last_change_ms: u32,
}

impl Default for ButtonFsm {
    fn default() -> Self {
        Self::new()
    }
}

impl ButtonFsm {
    pub const fn new() -> Self {
        Self {
            state:          State::Idle,
            pattern:        heapless::Vec::new(),
            last_raw:       false,
            last_change_ms: 0,
        }
    }

    /// Один тик опроса. Идемпотентно: вызывать с типичным шагом 5–10 мс.
    pub fn step(&mut self, now_ms: u32, raw_pressed: bool) -> Option<ButtonAction> {
        if raw_pressed != self.last_raw {
            self.last_raw = raw_pressed;
            self.last_change_ms = now_ms;
            return None;
        }
        let stable_ms = now_ms.wrapping_sub(self.last_change_ms);

        match self.state {
            State::Idle => {
                if raw_pressed && stable_ms >= DEBOUNCE_MS {
                    self.state = State::Pressed { since_ms: now_ms };
                }
                None
            }
            State::Pressed { since_ms } => {
                if !raw_pressed && stable_ms >= DEBOUNCE_MS {
                    let press_dur = now_ms.wrapping_sub(since_ms);
                    let kind = if press_dur >= LONG_PRESS_MS {
                        ClickKind::Long
                    } else {
                        ClickKind::Short
                    };
                    // Если буфер переполнен — паттерн всё равно нераспознанный,
                    // дальше его всё равно отбросит decode(); просто перестаём
                    // копить, чтобы не паниковать.
                    let _ = self.pattern.push(kind);
                    self.state = State::Released {
                        last_release_ms: now_ms,
                    };
                }
                None
            }
            State::Released { last_release_ms } => {
                if raw_pressed && stable_ms >= DEBOUNCE_MS {
                    self.state = State::Pressed { since_ms: now_ms };
                    None
                } else if !raw_pressed && now_ms.wrapping_sub(last_release_ms) >= INTERCLICK_GAP_MS {
                    let action = decode(&self.pattern);
                    self.pattern.clear();
                    self.state = State::Idle;
                    action
                } else {
                    None
                }
            }
        }
    }
}

fn decode(p: &[ClickKind]) -> Option<ButtonAction> {
    use ClickKind::*;
    match p {
        [Short] => Some(ButtonAction::Short),
        [Long] => Some(ButtonAction::Long),
        [Short, Short] => Some(ButtonAction::DoubleShort),
        [Long, Short, Short] => Some(ButtonAction::LongShortShort),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Прогоняет FSM по интервалам `(start_ms, end_ms, pressed)` с шагом
    /// `STEP_MS` и собирает все возвращённые actions.
    fn drive(intervals: &[(u32, u32, bool)]) -> heapless::Vec<ButtonAction, 8> {
        const STEP_MS: u32 = 5;
        let mut fsm = ButtonFsm::new();
        let mut out = heapless::Vec::new();
        let total = intervals.last().map(|i| i.1).unwrap_or(0) + 2_000;
        let mut t = 0u32;
        while t <= total {
            let pressed = intervals
                .iter()
                .find(|(s, e, _)| t >= *s && t < *e)
                .map(|(_, _, p)| *p)
                .unwrap_or(false);
            if let Some(a) = fsm.step(t, pressed) {
                let _ = out.push(a);
            }
            t += STEP_MS;
        }
        out
    }

    #[test]
    fn single_short_press() {
        let actions = drive(&[(100, 200, true)]);
        assert_eq!(&actions[..], &[ButtonAction::Short]);
    }

    #[test]
    fn single_long_press() {
        // 600 ms ≥ LONG_PRESS_MS (500)
        let actions = drive(&[(100, 700, true)]);
        assert_eq!(&actions[..], &[ButtonAction::Long]);
    }

    #[test]
    fn long_press_just_below_threshold_is_short() {
        // 480 ms < 500 ms threshold → S
        let actions = drive(&[(100, 580, true)]);
        assert_eq!(&actions[..], &[ButtonAction::Short]);
    }

    #[test]
    fn double_short_press() {
        // press 100 ms, gap 150 ms, press 100 ms — оба внутри INTERCLICK_GAP
        let actions = drive(&[(100, 200, true), (350, 450, true)]);
        assert_eq!(&actions[..], &[ButtonAction::DoubleShort]);
    }

    #[test]
    fn long_short_short() {
        let actions = drive(&[
            (100, 700, true),   // L (600 ms)
            (850, 950, true),   // S
            (1100, 1200, true), // S
        ]);
        assert_eq!(&actions[..], &[ButtonAction::LongShortShort]);
    }

    #[test]
    fn triple_short_is_unrecognized_and_consumed() {
        // SSS не в таблице → возвращаем None и сбрасываем буфер. Следующий
        // одиночный S после gap должен снова распознаться как Short — это и
        // подтверждает, что предыдущая серия не залипла.
        let actions = drive(&[
            (100, 200, true), // S
            (350, 450, true), // S
            (600, 700, true), // S → SSS, отброшен после gap
            // gap > INTERCLICK_GAP, новая серия:
            (1500, 1600, true), // S
        ]);
        assert_eq!(&actions[..], &[ButtonAction::Short]);
    }

    #[test]
    fn two_separate_short_presses_with_long_gap_are_two_actions() {
        // Между нажатиями > INTERCLICK_GAP_MS → две отдельные серии.
        let actions = drive(&[
            (100, 200, true),   // S
            (1000, 1100, true), // S — отдельная серия, gap > 350
        ]);
        assert_eq!(&actions[..], &[ButtonAction::Short, ButtonAction::Short]);
    }

    #[test]
    fn bounce_shorter_than_debounce_is_ignored() {
        // 5 ms нажатия — короче DEBOUNCE_MS=15 → состояние стабильным не стало,
        // в Pressed не уходим.
        let actions = drive(&[(100, 105, true)]);
        assert!(actions.is_empty());
    }

    #[test]
    fn pattern_buffer_overflow_drops_silently() {
        // 5 коротких кликов подряд — `pattern` забит до MAX_PATTERN=4, push
        // пятого молча игнорируется, decode([S,S,S,S]) → None. Главное — не
        // паникуем и FSM возвращается в Idle.
        let mut intervals: heapless::Vec<(u32, u32, bool), 8> = heapless::Vec::new();
        for i in 0..5 {
            let s = 100 + i as u32 * 250;
            let _ = intervals.push((s, s + 100, true));
        }
        let actions = drive(&intervals);
        assert!(actions.is_empty(), "got {:?}", actions);
    }
}
