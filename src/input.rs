//! Input normalization, stick axis processing, switch decoding, and battery calculation.

use crate::adc;

/// 3-position switch states.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SwitchPos {
    Up,
    Mid,
    Down,
}

impl SwitchPos {
    pub fn as_char(&self) -> char {
        match self {
            SwitchPos::Up => 'U',
            SwitchPos::Mid => 'M',
            SwitchPos::Down => 'D',
        }
    }
}

/// Normalized stick axes (-1000 .. +1000).
#[derive(Copy, Clone, Debug)]
pub struct Sticks {
    pub roll: i16,     // CH1 (AIL): -1000 (left) .. +1000 (right)
    pub pitch: i16,    // CH2 (ELE): -1000 (down) .. +1000 (up)
    pub throttle: i16, // CH3 (THR): -1000 (bottom/0%) .. +1000 (top/100%)
    pub yaw: i16,      // CH4 (RUD): -1000 (left) .. +1000 (right)
}

/// Normalized potentiometer rotary dials (-1000 .. +1000).
#[derive(Copy, Clone, Debug)]
pub struct Pots {
    pub vr1: i16, // VRA
    pub vr2: i16, // VRB
}

/// Physical switch states on the radio.
#[derive(Copy, Clone, Debug)]
pub struct Switches {
    pub sa: SwitchPos, // 2-pos
    pub sb: SwitchPos, // 3-pos
    pub sc: SwitchPos, // 3-pos
    pub sd: SwitchPos, // 2-pos
}

/// Full processed input snapshot.
pub struct InputState {
    pub sticks: Sticks,
    pub pots: Pots,
    pub switches: Switches,
    pub battery_mv: u16,
    #[allow(dead_code)]
    pub raw: [u16; adc::NUM_CHANNELS],
}

/// Calibration data for a single analog axis.
#[derive(Copy, Clone, Debug)]
pub struct AxisCalib {
    pub min: u16,
    pub center: u16,
    pub max: u16,
    pub filtered_raw: u16,
    pub invert: bool,
}

impl AxisCalib {
    pub const fn new(min: u16, center: u16, max: u16, invert: bool) -> Self {
        Self {
            min,
            center,
            max,
            filtered_raw: 0,
            invert,
        }
    }

    /// Normalize raw ADC count (0..4095) piecewise around center point to -1000..+1000.
    pub fn normalize(&mut self, raw: u16) -> i16 {
        // Adaptive jitter filter:
        // Carbon pots on FlySky gimbals exhibit micro-flutter (< 10 ADC counts).
        // If movement is small, apply EMA smoothing; if fast/intentional, pass through with 0 latency.
        if self.filtered_raw == 0 {
            self.filtered_raw = raw;
        }
        let diff = (raw as i32 - self.filtered_raw as i32).abs();
        if diff < 12 {
            self.filtered_raw = (((self.filtered_raw as u32 * 3) + raw as u32) / 4) as u16;
        } else {
            self.filtered_raw = raw;
        }
        let smoothed_raw = self.filtered_raw;

        // Dynamic endpoint expansion when physical stick reaches further than initial default
        if smoothed_raw < self.min && smoothed_raw > 400 {
            self.min = smoothed_raw;
        }
        if smoothed_raw > self.max && smoothed_raw < 3700 {
            self.max = smoothed_raw;
        }

        let val = if smoothed_raw <= self.center {
            let span = (self.center - self.min).max(100) as i32;
            let delta = smoothed_raw as i32 - self.center as i32;
            ((delta * 1000) / span).clamp(-1000, 0)
        } else {
            let span = (self.max - self.center).max(100) as i32;
            let delta = smoothed_raw as i32 - self.center as i32;
            ((delta * 1000) / span).clamp(0, 1000)
        };

        // 1.5% deadband at resting center to eliminate remaining center noise
        let filtered = if val > -15 && val < 15 {
            0
        } else {
            val
        };

        if self.invert {
            -filtered as i16
        } else {
            filtered as i16
        }
    }
}

// Initial default gimbal endpoints based on FlySky FS-i6X mechanical potentiometer throw
// Typical sweep is ~1150 (min), ~2048 (center), ~2950 (max)
static mut ROLL_CALIB: AxisCalib = AxisCalib::new(1180, 2048, 2920, false);
static mut PITCH_CALIB: AxisCalib = AxisCalib::new(1180, 2048, 2920, false);
static mut THROTTLE_CALIB: AxisCalib = AxisCalib::new(1180, 2048, 2920, false);
static mut YAW_CALIB: AxisCalib = AxisCalib::new(1180, 2048, 2920, false);

/// Initialize input subsystem and measure resting center for spring-loaded gimbals.
pub fn init() {
    // Wait for continuous DMA scanner to complete its initial cycle
    adc::wait_first_conversion();

    // Average 16 scans over ~4ms for rock-solid zero reference
    let mut sum_roll = 0u32;
    let mut sum_pitch = 0u32;
    let mut sum_yaw = 0u32;
    const SAMPLES: u32 = 16;

    for _ in 0..SAMPLES {
        let raw = adc::read_raw();
        sum_roll += raw[0] as u32;
        sum_pitch += raw[1] as u32;
        sum_yaw += raw[3] as u32;
        for _ in 0..3_000 {
            cortex_m::asm::nop();
        }
    }

    let avg_roll = (sum_roll / SAMPLES) as u16;
    let avg_pitch = (sum_pitch / SAMPLES) as u16;
    let avg_yaw = (sum_yaw / SAMPLES) as u16;

    unsafe {
        // Roll: PA0 (RH)
        if avg_roll >= 1500 && avg_roll <= 2500 {
            (*core::ptr::addr_of_mut!(ROLL_CALIB)).center = avg_roll;
            (*core::ptr::addr_of_mut!(ROLL_CALIB)).min = avg_roll.saturating_sub(850);
            (*core::ptr::addr_of_mut!(ROLL_CALIB)).max = avg_roll.saturating_add(850);
        }
        // Pitch: PA1 (RV)
        if avg_pitch >= 1500 && avg_pitch <= 2500 {
            (*core::ptr::addr_of_mut!(PITCH_CALIB)).center = avg_pitch;
            (*core::ptr::addr_of_mut!(PITCH_CALIB)).min = avg_pitch.saturating_sub(850);
            (*core::ptr::addr_of_mut!(PITCH_CALIB)).max = avg_pitch.saturating_add(850);
        }
        // Yaw: PA3 (LH)
        if avg_yaw >= 1500 && avg_yaw <= 2500 {
            (*core::ptr::addr_of_mut!(YAW_CALIB)).center = avg_yaw;
            (*core::ptr::addr_of_mut!(YAW_CALIB)).min = avg_yaw.saturating_sub(850);
            (*core::ptr::addr_of_mut!(YAW_CALIB)).max = avg_yaw.saturating_add(850);
        }
    }
}

/// Normalize rotary dials (0..4095) to -1000..+1000.
fn normalize_pot(raw: u16) -> i16 {
    let delta = raw as i32 - 2048;
    ((delta * 1000) / 1950).clamp(-1000, 1000) as i16
}

/// Decode resistor ladder analog switch voltage:
/// - UP:   0 .. 1365 (0 .. 1/3 Vcc)
/// - MID:  1366 .. 2730 (1/3 .. 2/3 Vcc)
/// - DOWN: 2731 .. 4095 (2/3 .. 1 Vcc)
fn decode_switch(raw: u16) -> SwitchPos {
    if raw < 1365 {
        SwitchPos::Up
    } else if raw < 2730 {
        SwitchPos::Mid
    } else {
        SwitchPos::Down
    }
}

/// Calculate battery voltage in millivolts using the OpenI6X calibrated formula.
/// Accounts for the 1/2 resistor divider and 0.20V series protection diode.
fn calculate_battery_mv(raw: u16) -> u16 {
    // OpenTX calibrated formula uses 11-bit ADC (raw / 2):
    // instant_vbat = ((raw/2) * 200) / 421 + 20 (in 10mV steps)
    // Directly from 12-bit raw: (raw * 100) / 421 + 20
    let vbat_10mv = ((raw as u32 * 100) / 421) + 20;
    (vbat_10mv * 10) as u16
}

/// Poll the ADC and return complete, processed flight controls.
pub fn poll() -> InputState {
    let raw = adc::read_raw();

    // Mode 2 Pinout:
    // raw[0] = PA0 (RH - Roll / Aileron)
    // raw[1] = PA1 (RV - Pitch / Elevator, spring return)
    // raw[2] = PA2 (LV - Throttle, friction ratchet / no spring return)
    // raw[3] = PA3 (LH - Yaw / Rudder, spring return)
    let sticks = unsafe {
        Sticks {
            roll: (*core::ptr::addr_of_mut!(ROLL_CALIB)).normalize(raw[0]),
            pitch: (*core::ptr::addr_of_mut!(PITCH_CALIB)).normalize(raw[1]),
            throttle: (*core::ptr::addr_of_mut!(THROTTLE_CALIB)).normalize(raw[2]),
            yaw: (*core::ptr::addr_of_mut!(YAW_CALIB)).normalize(raw[3]),
        }
    };

    // Pots: VRA on PA6, VRB on PA7
    let pots = Pots {
        vr1: normalize_pot(raw[6]), // PA6 (VRA)
        vr2: normalize_pot(raw[7]), // PA7 (VRB)
    };

    // Switches:
    // SA on PA4 (2-pos)
    // SB on PA5 (3-pos)
    // SC on PB0 (3-pos) - Swapped with VRB!
    // SD on PB1 (2-pos)
    let switches = Switches {
        sa: decode_switch(raw[4]), // PA4
        sb: decode_switch(raw[5]), // PA5
        sc: decode_switch(raw[8]), // PB0
        sd: decode_switch(raw[9]), // PB1
    };

    let battery_mv = calculate_battery_mv(raw[10]); // PC0

    InputState {
        sticks,
        pots,
        switches,
        battery_mv,
        raw,
    }
}
