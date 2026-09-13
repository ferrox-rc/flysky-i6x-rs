//! Digital Trim controller for FlySky FS-i6X.
//!
//! Manages 4 trim axes (Roll, Pitch, Throttle, Yaw) mapped to the 8 trim rocker keys.
//! Provides single-press edge detection, auto-repeat when held, audio feedback via Buzzer,
//! and range limiting to -25..+25 steps (±100 µs authority).

use crate::boot;
use crate::buzzer::Buzzer;

pub const TRIM_MIN: i8 = -25;
pub const TRIM_MAX: i8 = 25;
pub const TRIM_STEP_US: i16 = 4; // 4 µs per step -> ±100 µs authority

/// Trim values for all 4 primary flight control channels.
#[derive(Copy, Clone, Debug, Default)]
pub struct TrimValues {
    pub roll: i8,     // CH1 (AIL)
    pub pitch: i8,    // CH2 (ELE)
    pub throttle: i8, // CH3 (THR)
    pub yaw: i8,      // CH4 (RUD)
}

/// Identifies which trim was last modified (for temporary UI callout).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ActiveTrim {
    None,
    Roll,
    Pitch,
    Throttle,
    Yaw,
}

pub struct TrimController {
    pub values: TrimValues,
    pub last_active: ActiveTrim,
    pub active_timer_ms: u16,
    pub throttle_enabled: bool,
    prev_trim_keys: u8,
    hold_time_ms: u16,
    repeat_time_ms: u16,
}

impl TrimController {
    pub const fn new() -> Self {
        Self {
            values: TrimValues {
                roll: 0,
                pitch: 0,
                throttle: 0,
                yaw: 0,
            },
            last_active: ActiveTrim::None,
            active_timer_ms: 0,
            throttle_enabled: false, // Option 2: locked/disabled for FC safety
            prev_trim_keys: 0,
            hold_time_ms: 0,
            repeat_time_ms: 0,
        }
    }

    /// Update trims based on raw 16-bit key matrix scan.
    /// Handles single clicks, auto-repeat, DFU lockout, and buzzer tones.
    pub fn update(&mut self, keys: u16, dt_ms: u16, buzzer: &mut Buzzer) {
        // Decrease active display timer
        if self.active_timer_ms > 0 {
            self.active_timer_ms = self.active_timer_ms.saturating_sub(dt_ms);
            if self.active_timer_ms == 0 {
                self.last_active = ActiveTrim::None;
            }
        }

        // Lock out trim processing if the DFU combination is pressed
        if boot::is_dfu_requested(keys) {
            self.prev_trim_keys = 0;
            self.hold_time_ms = 0;
            self.repeat_time_ms = 0;
            return;
        }

        // Trim keys occupy bits 0..7 of the key matrix
        let trim_keys = (keys & 0x00FF) as u8;

        if trim_keys == 0 {
            self.prev_trim_keys = 0;
            self.hold_time_ms = 0;
            self.repeat_time_ms = 0;
            return;
        }

        let mut trigger_key: Option<u8> = None;

        if trim_keys != self.prev_trim_keys {
            // New key press detected (edge trigger)
            let newly_pressed = trim_keys & !self.prev_trim_keys;
            if newly_pressed != 0 {
                // Pick lowest active bit
                let key_idx = newly_pressed.trailing_zeros() as u8;
                trigger_key = Some(key_idx);
            }
            self.prev_trim_keys = trim_keys;
            self.hold_time_ms = 0;
            self.repeat_time_ms = 0;
        } else {
            // Key held down -> auto-repeat
            self.hold_time_ms = self.hold_time_ms.saturating_add(dt_ms);
            if self.hold_time_ms >= 350 {
                self.repeat_time_ms = self.repeat_time_ms.saturating_add(dt_ms);
                if self.repeat_time_ms >= 90 {
                    self.repeat_time_ms = 0;
                    let key_idx = trim_keys.trailing_zeros() as u8;
                    trigger_key = Some(key_idx);
                }
            }
        }

        if let Some(key_bit) = trigger_key {
            self.process_trim_key(key_bit, buzzer);
        }
    }

    /// Process a single trim click for a given key index (0..7).
    fn process_trim_key(&mut self, key: u8, buzzer: &mut Buzzer) {
        match key {
            0 => {
                // Roll R (+1)
                self.step_trim(ActiveTrim::Roll, 1, buzzer);
            }
            1 => {
                // Roll L (-1)
                self.step_trim(ActiveTrim::Roll, -1, buzzer);
            }
            2 => {
                // Pitch U (+1)
                self.step_trim(ActiveTrim::Pitch, 1, buzzer);
            }
            3 => {
                // Pitch D (-1)
                self.step_trim(ActiveTrim::Pitch, -1, buzzer);
            }
            4 => {
                // Throttle U (+1)
                self.step_trim(ActiveTrim::Throttle, 1, buzzer);
            }
            5 => {
                // Throttle D (-1)
                self.step_trim(ActiveTrim::Throttle, -1, buzzer);
            }
            6 => {
                // Yaw R (+1)
                self.step_trim(ActiveTrim::Yaw, 1, buzzer);
            }
            7 => {
                // Yaw L (-1)
                self.step_trim(ActiveTrim::Yaw, -1, buzzer);
            }
            _ => {}
        }
    }

    /// Step a specific trim axis by delta (+1 or -1) and produce appropriate audio.
    fn step_trim(&mut self, axis: ActiveTrim, delta: i8, buzzer: &mut Buzzer) {
        if axis == ActiveTrim::Throttle && !self.throttle_enabled {
            return; // Throttle trim locked/disabled
        }

        let val_ref = match axis {
            ActiveTrim::Roll => &mut self.values.roll,
            ActiveTrim::Pitch => &mut self.values.pitch,
            ActiveTrim::Throttle => &mut self.values.throttle,
            ActiveTrim::Yaw => &mut self.values.yaw,
            ActiveTrim::None => return,
        };

        let curr = *val_ref;
        if delta > 0 && curr >= TRIM_MAX {
            buzzer.trim_limit();
            return;
        }
        if delta < 0 && curr <= TRIM_MIN {
            buzzer.trim_limit();
            return;
        }

        let new_val = (curr + delta).clamp(TRIM_MIN, TRIM_MAX);
        *val_ref = new_val;

        self.last_active = axis;
        self.active_timer_ms = 1500; // Show trim overlay on UI for 1.5 seconds

        if new_val == 0 {
            buzzer.trim_center();
        } else {
            buzzer.trim_step(new_val);
        }
    }

    /// Apply trim offset to raw channel pulse width (1000..2000 µs).
    #[inline(always)]
    pub fn apply(base_us: u16, trim: i8) -> u16 {
        let offset = trim as i16 * TRIM_STEP_US;
        (base_us as i16 + offset).clamp(1000, 2000) as u16
    }
}
