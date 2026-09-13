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
    pub filtered_raw: u32,
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

    /// Normalize raw ADC count (0..4095) around center point to -1000..+1000.
    /// OpenTX/OpenI6X MMA filter: filters micro-jitter without adding latency or deadband.
    pub fn normalize(&mut self, raw: u16) -> i16 {
        if self.filtered_raw == 0 {
            self.filtered_raw = raw as u32 * 16;
        }

        let previous = (self.filtered_raw / 16) as u16;
        let diff = (raw as i32 - previous as i32).abs();

        // OpenTX jitter filter:
        // Pass through any change >= 20 counts directly (0 latency)
        // For small changes (< 20 counts), use MMA filter
        if diff < 20 {
            self.filtered_raw = (self.filtered_raw - previous as u32) + raw as u32;
        } else {
            self.filtered_raw = raw as u32 * 16;
        }
        let smoothed_raw = (self.filtered_raw / 16) as u16;

        let val = if smoothed_raw <= self.center {
            let span = (self.center - self.min).max(100) as i32;
            let delta = smoothed_raw as i32 - self.center as i32;
            ((delta * 1000) / span).clamp(-1000, 0)
        } else {
            let span = (self.max - self.center).max(100) as i32;
            let delta = smoothed_raw as i32 - self.center as i32;
            ((delta * 1000) / span).clamp(0, 1000)
        };

        if self.invert {
            -val as i16
        } else {
            val as i16
        }
    }
}

pub const GIMBAL_HALF_SPAN: u16 = 1670;
const DEFAULT_STICK_CENTER: u16 = 2048;
const DEFAULT_STICK_MIN: u16 = DEFAULT_STICK_CENTER - GIMBAL_HALF_SPAN; // 378
const DEFAULT_STICK_MAX: u16 = DEFAULT_STICK_CENTER + GIMBAL_HALF_SPAN; // 3718

// Initial default gimbal endpoints based on FlySky FS-i6X mechanical potentiometer throw
// Roll and Pitch pots are inverted on FlySky hardware (matching OpenI6X ana_direction = {1, -1, 1, -1})
static mut ROLL_CALIB: AxisCalib = AxisCalib::new(DEFAULT_STICK_MIN, DEFAULT_STICK_CENTER, DEFAULT_STICK_MAX, true);
static mut PITCH_CALIB: AxisCalib = AxisCalib::new(DEFAULT_STICK_MIN, DEFAULT_STICK_CENTER, DEFAULT_STICK_MAX, true);
static mut THROTTLE_CALIB: AxisCalib = AxisCalib::new(DEFAULT_STICK_MIN, DEFAULT_STICK_CENTER, DEFAULT_STICK_MAX, false);
static mut YAW_CALIB: AxisCalib = AxisCalib::new(DEFAULT_STICK_MIN, DEFAULT_STICK_CENTER, DEFAULT_STICK_MAX, false);

/// Initialize input subsystem and measure resting center for spring-loaded gimbals.
pub fn init() {
    // Wait for continuous DMA scanner to complete its initial cycle
    adc::wait_first_conversion();

    // Average 16 scans over ~4ms for rock-solid zero reference
    let mut sum_pitch = 0u32;
    let mut sum_roll = 0u32;
    let mut sum_yaw = 0u32;
    const SAMPLES: u32 = 16;

    for _ in 0..SAMPLES {
        let raw = adc::read_raw();
        sum_roll += raw[0] as u32;  // PA0 = Roll (Aileron)
        sum_pitch += raw[1] as u32; // PA1 = Pitch (Elevator)
        sum_yaw += raw[3] as u32;   // PA3 = LH (Yaw)
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
            (*core::ptr::addr_of_mut!(ROLL_CALIB)).min = avg_roll.saturating_sub(GIMBAL_HALF_SPAN);
            (*core::ptr::addr_of_mut!(ROLL_CALIB)).max = avg_roll.saturating_add(GIMBAL_HALF_SPAN);
        }
        // Pitch: PA1 (RV)
        if avg_pitch >= 1500 && avg_pitch <= 2500 {
            (*core::ptr::addr_of_mut!(PITCH_CALIB)).center = avg_pitch;
            (*core::ptr::addr_of_mut!(PITCH_CALIB)).min = avg_pitch.saturating_sub(GIMBAL_HALF_SPAN);
            (*core::ptr::addr_of_mut!(PITCH_CALIB)).max = avg_pitch.saturating_add(GIMBAL_HALF_SPAN);
        }
        // Yaw: PA3 (LH)
        if avg_yaw >= 1500 && avg_yaw <= 2500 {
            (*core::ptr::addr_of_mut!(YAW_CALIB)).center = avg_yaw;
            (*core::ptr::addr_of_mut!(YAW_CALIB)).min = avg_yaw.saturating_sub(GIMBAL_HALF_SPAN);
            (*core::ptr::addr_of_mut!(YAW_CALIB)).max = avg_yaw.saturating_add(GIMBAL_HALF_SPAN);
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

    // Mode 2 Pinout matching FlySky FS-i6X hardware:
    // raw[0] = PA0: RH (Right Horizontal - Roll / Aileron)
    // raw[1] = PA1: RV (Right Vertical - Pitch / Elevator)
    // raw[2] = PA2: LV (Left Vertical - Throttle, friction ratchet / no spring return)
    // raw[3] = PA3: LH (Left Horizontal - Yaw / Rudder)
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
