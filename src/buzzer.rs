//! Hardware Piezo Buzzer driver using TIM1 Channel 1 on PA8.
//!
//! Pin PA8 is configured as Alternate Function 2 (TIM1_CH1).
//! TIM1 is clocked at 48 MHz from APB2 with PSC=47 (1 MHz tick).
//! Outputs clean hardware PWM tones with programmable frequency and duration.

use core::ptr;

// RCC registers
const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;

// GPIOA registers (Base 0x4800_0000)
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRH: *mut u32 = 0x4800_0024 as *mut u32;

// TIM1 registers (Base 0x4001_2C00)
const TIM1_CR1: *mut u32 = 0x4001_2C00 as *mut u32;
const TIM1_SR: *mut u32 = 0x4001_2C10 as *mut u32;
const TIM1_CCMR1: *mut u32 = 0x4001_2C18 as *mut u32;
const TIM1_CCER: *mut u32 = 0x4001_2C20 as *mut u32;
const TIM1_CNT: *mut u32 = 0x4001_2C24 as *mut u32;
const TIM1_PSC: *mut u32 = 0x4001_2C28 as *mut u32;
const TIM1_ARR: *mut u32 = 0x4001_2C2C as *mut u32;
const TIM1_CCR1: *mut u32 = 0x4001_2C34 as *mut u32;
const TIM1_BDTR: *mut u32 = 0x4001_2C44 as *mut u32;

pub const BEEP_DEFAULT_FREQ: u16 = 2250;
pub const BEEP_CENTER_FREQ: u16 = 2800;
pub const BEEP_LIMIT_FREQ: u16 = 1100;

/// A single musical note or chime element in a melody sequence.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub freq_hz: u16,
    pub duration_ms: u16,
    pub pause_ms: u16,
}

impl Note {
    pub const fn new(freq_hz: u16, duration_ms: u16, pause_ms: u16) -> Self {
        Self {
            freq_hz,
            duration_ms,
            pause_ms,
        }
    }
}

const MAX_SEQ_NOTES: usize = 8;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ToneStyle {
    Simple = 0,
    Rich = 1,
}

impl ToneStyle {
    pub const fn from_u8(val: u8) -> Self {
        if val == 0 {
            ToneStyle::Simple
        } else {
            ToneStyle::Rich
        }
    }
}

pub struct Buzzer {
    pub enabled: bool,
    pub tone_style: ToneStyle,
    remaining_ms: u16,
    pause_ms: u16,
    repeat_count: u8,
    active_freq: u16,
    active_len_ms: u16,
    active_pause_ms: u16,
    // Note sequence state machine
    seq_notes: [Note; MAX_SEQ_NOTES],
    seq_len: u8,
    seq_idx: u8,
}

#[allow(dead_code)]
impl Buzzer {
    pub const fn new() -> Self {
        Self {
            enabled: true,
            tone_style: ToneStyle::Rich,
            remaining_ms: 0,
            pause_ms: 0,
            repeat_count: 0,
            active_freq: 0,
            active_len_ms: 0,
            active_pause_ms: 0,
            seq_notes: [Note::new(0, 0, 0); MAX_SEQ_NOTES],
            seq_len: 0,
            seq_idx: 0,
        }
    }

    /// Initialize GPIOA PA8 as AF2 (TIM1_CH1) and configure TIM1 PWM output.
    pub fn init(&mut self) {
        unsafe {
            // 1. Enable GPIOA (bit 17) and TIM1 (bit 11) clocks
            let ahb = ptr::read_volatile(RCC_AHBENR);
            ptr::write_volatile(RCC_AHBENR, ahb | (1 << 17));

            let apb2 = ptr::read_volatile(RCC_APB2ENR);
            ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 11));

            // 2. Configure PA8 as AF mode (MODER[17:16] = 10)
            let moder = ptr::read_volatile(GPIOA_MODER);
            ptr::write_volatile(GPIOA_MODER, (moder & !(3 << 16)) | (2 << 16));

            // 3. Set PA8 Alternate Function to AF2 (TIM1_CH1)
            // AFRH[3:0] corresponding to pin 8 = bits 3:0
            let afrh = ptr::read_volatile(GPIOA_AFRH);
            ptr::write_volatile(GPIOA_AFRH, (afrh & !0x0F) | 0x02);

            // 4. Configure TIM1 for PWM generation:
            // PSC = 47 -> 48 MHz / 48 = 1 MHz (1 µs resolution)
            ptr::write_volatile(TIM1_PSC, 47);

            // Output Compare 1 Mode: PWM mode 1 (active while CNT < CCR1)
            // Bits 6:4 of CCMR1 (OC1M) = 0b110 (PWM Mode 1), Bit 3 (OC1PE) = 1 (Preload)
            ptr::write_volatile(TIM1_CCMR1, (6 << 4) | (1 << 3));

            // Enable Channel 1 output with active low polarity in CCER (CC1E | CC1P)
            ptr::write_volatile(TIM1_CCER, (1 << 1) | (1 << 0));

            // Main Output Enable (MOE) in BDTR (bit 15: MOE) - required for advanced timers like TIM1
            ptr::write_volatile(TIM1_BDTR, 1 << 15);
        }
    }

    /// Turn on the hardware PWM generator at the specified frequency (50% duty cycle).
    fn hardware_on(&self, freq_hz: u16) {
        if !self.enabled || freq_hz < 100 {
            return;
        }

        unsafe {
            // Period in 1 µs ticks = 1_000_000 / freq_hz
            let period = (1_000_000u32 / (freq_hz as u32)).clamp(10, 65535);
            let arr = period.saturating_sub(1);
            let ccr = period / 2; // 50% duty cycle

            ptr::write_volatile(TIM1_ARR, arr);
            ptr::write_volatile(TIM1_CCR1, ccr);
            if ptr::read_volatile(TIM1_CNT) > arr {
                ptr::write_volatile(TIM1_CNT, 0);
            }

            // Start counter: CEN (bit 0) | ARPE (bit 7)
            ptr::write_volatile(TIM1_CR1, (1 << 7) | (1 << 0));
        }
    }

    /// Turn off the hardware PWM generator.
    fn hardware_off(&self) {
        unsafe {
            ptr::write_volatile(TIM1_CR1, 0);
            ptr::write_volatile(TIM1_CNT, 0);
            ptr::write_volatile(TIM1_SR, 0);
        }
    }

    /// Play a single tone of `freq_hz` for `duration_ms`.
    pub fn play_tone(&mut self, freq_hz: u16, duration_ms: u16) {
        self.seq_len = 0;
        self.seq_idx = 0;
        self.play_tone_pattern(freq_hz, duration_ms, 0, 0);
    }

    /// Play a repeating pattern of `freq_hz` for `duration_ms` with `pause_ms` between repeats.
    pub fn play_tone_pattern(&mut self, freq_hz: u16, duration_ms: u16, pause_ms: u16, repeats: u8) {
        self.seq_len = 0;
        self.seq_idx = 0;
        self.active_freq = freq_hz;
        self.active_len_ms = duration_ms;
        self.active_pause_ms = pause_ms;
        self.remaining_ms = duration_ms;
        self.pause_ms = pause_ms;
        self.repeat_count = repeats;

        self.hardware_on(freq_hz);
    }

    /// Play an ordered sequence of melodic notes.
    pub fn play_sequence(&mut self, notes: &[Note]) {
        if !self.enabled || notes.is_empty() {
            return;
        }

        let count = notes.len().min(MAX_SEQ_NOTES);
        self.seq_notes[..count].copy_from_slice(&notes[..count]);
        self.seq_len = count as u8;
        self.seq_idx = 0;
        self.repeat_count = 0;

        let first = self.seq_notes[0];
        self.active_freq = first.freq_hz;
        self.active_len_ms = first.duration_ms;
        self.active_pause_ms = first.pause_ms;
        self.remaining_ms = first.duration_ms;
        self.pause_ms = first.pause_ms;

        self.hardware_on(first.freq_hz);
    }

    /// Immediate silence and clear queues.
    pub fn stop(&mut self) {
        self.remaining_ms = 0;
        self.pause_ms = 0;
        self.repeat_count = 0;
        self.seq_len = 0;
        self.seq_idx = 0;
        self.hardware_off();
    }

    /// Periodic update advancing active tone playback (called from main loop).
    pub fn tick(&mut self, elapsed_ms: u16) {
        if self.remaining_ms > 0 {
            if self.remaining_ms <= elapsed_ms {
                self.remaining_ms = 0;
                self.hardware_off();

                // If repeats remain, start inter-beep pause
                if self.repeat_count > 0 {
                    self.pause_ms = self.active_pause_ms;
                } else if self.seq_len > 0 {
                    // Sequence note completed, pause before next note
                    self.pause_ms = self.active_pause_ms;
                }
            } else {
                self.remaining_ms -= elapsed_ms;
            }
        } else if self.pause_ms > 0 {
            if self.pause_ms <= elapsed_ms {
                self.pause_ms = 0;
                if self.repeat_count > 0 {
                    self.repeat_count -= 1;
                    self.remaining_ms = self.active_len_ms;
                    self.hardware_on(self.active_freq);
                } else if self.seq_len > 0 {
                    // Advance to next note in sequence
                    self.seq_idx += 1;
                    if self.seq_idx < self.seq_len {
                        let note = self.seq_notes[self.seq_idx as usize];
                        self.active_freq = note.freq_hz;
                        self.active_len_ms = note.duration_ms;
                        self.active_pause_ms = note.pause_ms;
                        self.remaining_ms = note.duration_ms;
                        self.pause_ms = note.pause_ms;
                        self.hardware_on(note.freq_hz);
                    } else {
                        // Sequence completed
                        self.seq_len = 0;
                        self.seq_idx = 0;
                    }
                }
            } else {
                self.pause_ms -= elapsed_ms;
            }
        }
    }

    // --- Sound presets ---

    /// Short tactile button click.
    pub fn click(&mut self) {
        self.play_tone(BEEP_DEFAULT_FREQ, 15);
    }

    /// Power-on welcome chime: pleasant 4-note ascending fanfare (C6 -> E6 -> G6 -> C7).
    pub fn chime_welcome(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 4] = [
                Note::new(1046, 110, 25), // C6
                Note::new(1318, 110, 25), // E6
                Note::new(1568, 120, 30), // G6
                Note::new(2093, 240, 0),  // C7 (drawn-out final resolving note)
            ];
            self.play_sequence(&MELODY);
        } else {
            self.click();
        }
    }

    /// Arming confirmation: crisp 2-note rising chirp.
    pub fn chime_armed(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 2] = [
                Note::new(1800, 45, 10),
                Note::new(2600, 75, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(2400, 80);
        }
    }

    /// Disarming confirmation: crisp 2-note falling chirp.
    pub fn chime_disarmed(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 2] = [
                Note::new(2600, 45, 10),
                Note::new(1800, 75, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(1800, 80);
        }
    }

    /// Trim position adjustment step. Pitch scales with trim value (-25..+25).
    pub fn trim_step(&mut self, step: i8) {
        let freq = (2000i32 + (step as i32 * 20)).clamp(1500, 2500) as u16;
        self.play_tone(freq, 25);
    }

    /// Trim center reference confirmed. Higher pitch distinct tone.
    pub fn trim_center(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 2] = [
                Note::new(2400, 30, 10),
                Note::new(3000, 50, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(BEEP_CENTER_FREQ, 60);
        }
    }

    /// Trim limit reached (cannot increment/decrement further).
    pub fn trim_limit(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 2] = [
                Note::new(1100, 40, 15),
                Note::new(900, 60, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(BEEP_LIMIT_FREQ, 45);
        }
    }

    /// Low battery warning alert: melodic 3-tone escalating chirp.
    pub fn warn_battery(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 3] = [
                Note::new(2000, 60, 30),
                Note::new(2300, 60, 30),
                Note::new(2600, 90, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone_pattern(2400, 70, 50, 1);
        }
    }

    /// Critical battery / failsafe urgent 2-tone siren.
    pub fn warn_critical(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 4] = [
                Note::new(2800, 70, 20),
                Note::new(1800, 70, 20),
                Note::new(2800, 70, 20),
                Note::new(1800, 70, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone_pattern(2800, 80, 40, 2);
        }
    }

    /// Pre-flight throttle / switch startup safety alarm (urgent alert).
    pub fn warn_preflight(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 3] = [
                Note::new(2600, 70, 25),
                Note::new(2600, 70, 25),
                Note::new(2900, 90, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone_pattern(2600, 80, 50, 2);
        }
    }

    /// Radio inactivity idle alarm (gentle reminder chirp).
    pub fn warn_inactivity(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 2] = [
                Note::new(1600, 70, 30),
                Note::new(2000, 100, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(1800, 250);
        }
    }

    /// Telemetry RSSI low warning alert (range warning < 40%).
    pub fn warn_rssi_low(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 2] = [
                Note::new(1900, 50, 20),
                Note::new(1700, 70, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(2000, 80);
        }
    }

    /// Telemetry RSSI critical alarm (range critical < 20%).
    pub fn warn_rssi_critical(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 4] = [
                Note::new(2600, 40, 20),
                Note::new(1800, 40, 20),
                Note::new(2600, 40, 20),
                Note::new(1800, 60, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone_pattern(2800, 50, 40, 2);
        }
    }

    /// Calibration wizard start confirmation chirp.
    pub fn chime_calib_start(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 2] = [
                Note::new(2000, 50, 15),
                Note::new(2500, 80, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(2200, 100);
        }
    }

    /// Calibration wizard success fanfare.
    pub fn chime_calib_success(&mut self) {
        if self.tone_style == ToneStyle::Rich {
            const MELODY: [Note; 3] = [
                Note::new(2000, 40, 15),
                Note::new(2500, 40, 15),
                Note::new(3100, 100, 0),
            ];
            self.play_sequence(&MELODY);
        } else {
            self.play_tone(2800, 150);
        }
    }
}
