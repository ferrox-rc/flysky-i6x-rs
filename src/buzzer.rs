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

pub struct Buzzer {
    pub enabled: bool,
    remaining_ms: u16,
    pause_ms: u16,
    repeat_count: u8,
    active_freq: u16,
    active_len_ms: u16,
    active_pause_ms: u16,
}

#[allow(dead_code)]
impl Buzzer {
    pub const fn new() -> Self {
        Self {
            enabled: true,
            remaining_ms: 0,
            pause_ms: 0,
            repeat_count: 0,
            active_freq: 0,
            active_len_ms: 0,
            active_pause_ms: 0,
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
        self.play_tone_pattern(freq_hz, duration_ms, 0, 0);
    }

    /// Play a repeating pattern of `freq_hz` for `duration_ms` with `pause_ms` between repeats.
    pub fn play_tone_pattern(&mut self, freq_hz: u16, duration_ms: u16, pause_ms: u16, repeats: u8) {
        self.active_freq = freq_hz;
        self.active_len_ms = duration_ms;
        self.active_pause_ms = pause_ms;
        self.remaining_ms = duration_ms;
        self.pause_ms = pause_ms;
        self.repeat_count = repeats;

        self.hardware_on(freq_hz);
    }

    /// Immediate silence.
    pub fn stop(&mut self) {
        self.remaining_ms = 0;
        self.pause_ms = 0;
        self.repeat_count = 0;
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

    /// Trim position adjustment step. Pitch scales with trim value (-25..+25).
    pub fn trim_step(&mut self, step: i8) {
        let freq = (2000i32 + (step as i32 * 20)).clamp(1500, 2500) as u16;
        self.play_tone(freq, 25);
    }

    /// Trim center reference confirmed. Higher pitch distinct tone.
    pub fn trim_center(&mut self) {
        self.play_tone(BEEP_CENTER_FREQ, 60);
    }

    /// Trim limit reached (cannot increment/decrement further).
    pub fn trim_limit(&mut self) {
        self.play_tone(BEEP_LIMIT_FREQ, 45);
    }

    /// Low battery double-chirp warning alert.
    pub fn warn_battery(&mut self) {
        self.play_tone_pattern(2400, 70, 50, 1);
    }

    /// Pre-flight throttle / switch startup safety alarm (urgent alert).
    pub fn warn_preflight(&mut self) {
        self.play_tone_pattern(2600, 80, 50, 2);
    }

    /// Radio inactivity idle alarm (gentle reminder chirp).
    pub fn warn_inactivity(&mut self) {
        self.play_tone(1800, 250);
    }

    /// Telemetry RSSI low warning alert (range warning < 40%).
    pub fn warn_rssi_low(&mut self) {
        self.play_tone(2000, 80);
    }

    /// Telemetry RSSI critical alarm (range critical < 20%).
    pub fn warn_rssi_critical(&mut self) {
        self.play_tone_pattern(2800, 50, 40, 2);
    }
}
