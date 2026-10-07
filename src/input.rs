//! Input normalization, stick axis processing, switch decoding, and battery calculation.

use crate::adc;
use crate::storage::{AdcInputMode, RadioConfig};

/// Default ADC deadband window around mechanical center detent (±16 counts ≈ ±0.4% travel).
pub const DETENT_DEADBAND: u16 = 16;

/// 3-position switch states.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SwitchPos {
    #[default]
    Up,
    Mid,
    Down,
}

/// Normalized stick axes (-1000 .. +1000).
#[derive(Copy, Clone, Debug)]
pub struct Sticks {
    pub roll: i16,     // CH1 (AIL): -1000 (left) .. +1000 (right)
    pub pitch: i16,    // CH2 (ELE): -1000 (down) .. +1000 (up)
    pub throttle: i16, // CH3 (THR): -1000 (bottom/0%) .. +1000 (top/100%)
    pub yaw: i16,      // CH4 (RUD): -1000 (left) .. +1000 (right)
}

/// Normalized potentiometer rotary dials and auxiliary analog inputs (-1000 .. +1000).
#[derive(Copy, Clone, Debug)]
pub struct Pots {
    pub vr1: i16, // VRA (PA6)
    pub vr2: i16, // VRB (PA7)
    pub vr3: i16, // VRC (Header P7 AD12 / PC2)
    pub vr4: i16, // VRD (Header P7 AD13 / PC3)
    pub vr5: i16, // VRE (Header P7 AD14 / PC4)
    pub vr6: i16, // VRF (Header P7 AD15 / PC5)
}

/// Physical switch states on the radio.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Switches {
    pub sa: SwitchPos, // 2-pos
    pub sb: SwitchPos, // 3-pos
    pub sc: SwitchPos, // 3-pos
    pub sd: SwitchPos, // 2-pos
    pub se: SwitchPos, // 2-pos (Ext switch PC12)
    pub sf: SwitchPos, // 2-pos (Ext switch PC15)
}

impl Switches {
    pub const DEFAULT: Self = Self {
        sa: SwitchPos::Up,
        sb: SwitchPos::Up,
        sc: SwitchPos::Up,
        sd: SwitchPos::Up,
        se: SwitchPos::Up,
        sf: SwitchPos::Up,
    };

    /// Detect if a switch moved between prev and self, returning the matching condition index 1..14.
    pub fn detect_condition_change(&self, prev: &Switches) -> Option<u8> {
        if self.sa != prev.sa {
            return Some(if self.sa == SwitchPos::Up { 1 } else { 2 });
        }
        if self.sb != prev.sb {
            return Some(match self.sb {
                SwitchPos::Up => 3,
                SwitchPos::Mid => 4,
                SwitchPos::Down => 5,
            });
        }
        if self.sc != prev.sc {
            return Some(match self.sc {
                SwitchPos::Up => 6,
                SwitchPos::Mid => 7,
                SwitchPos::Down => 8,
            });
        }
        if self.sd != prev.sd {
            return Some(if self.sd == SwitchPos::Up { 9 } else { 10 });
        }
        if self.se != prev.se {
            return Some(if self.se == SwitchPos::Up { 11 } else { 12 });
        }
        if self.sf != prev.sf {
            return Some(if self.sf == SwitchPos::Up { 13 } else { 14 });
        }
        None
    }

    /// Detect if any switch toggled, returning the switch index 1..6.
    pub fn detect_dr_switch_change(&self, prev: &Switches) -> Option<u8> {
        if self.sa != prev.sa {
            Some(1)
        } else if self.sb != prev.sb {
            Some(2)
        } else if self.sc != prev.sc {
            Some(3)
        } else if self.sd != prev.sd {
            Some(4)
        } else if self.se != prev.se {
            Some(5)
        } else if self.sf != prev.sf {
            Some(6)
        } else {
            None
        }
    }
}

/// Full processed input snapshot.
pub struct InputState {
    pub sticks: Sticks,
    pub pots: Pots,
    pub switches: Switches,
    pub battery_mv: u16,
    pub aux_pots: [i16; 10],
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

    /// Responsive low-latency jitter filter:
    /// Passes changes >= 6 counts with 0 latency; applies 4-sample MMA filter for resting micro-noise.
    pub fn filter_raw(&mut self, raw: u16) -> u16 {
        if self.filtered_raw == 0 {
            self.filtered_raw = raw as u32 * 4;
        }

        let previous = (self.filtered_raw / 4) as u16;
        let diff = (raw as i32 - previous as i32).abs();

        if diff < 6 {
            self.filtered_raw = (self.filtered_raw - previous as u32) + raw as u32;
        } else {
            self.filtered_raw = raw as u32 * 4;
        }
        (self.filtered_raw / 4) as u16
    }

    /// Normalize raw ADC count (0..4095) around center point to -1000..+1000.
    pub fn normalize(&mut self, raw: u16) -> i16 {
        let smoothed_raw = self.filter_raw(raw);

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

    /// Normalize a center-detented potentiometer with a deadband around the mechanical detent.
    /// Guarantees a solid 0 output when resting in the physical notch, while maintaining full
    /// independent linear travel (-1000 to +1000) even when negative and positive spans are asymmetric.
    pub fn normalize_detent(&mut self, raw: u16, deadband: u16) -> i16 {
        let smoothed_raw = self.filter_raw(raw);

        let val = if smoothed_raw.abs_diff(self.center) <= deadband {
            0
        } else if smoothed_raw < self.center {
            let active_center = self.center.saturating_sub(deadband);
            let span = active_center.saturating_sub(self.min).max(100) as i32;
            let delta = smoothed_raw as i32 - active_center as i32;
            ((delta * 1000) / span).clamp(-1000, 0)
        } else {
            let active_center = self.center.saturating_add(deadband);
            let span = self.max.saturating_sub(active_center).max(100) as i32;
            let delta = smoothed_raw as i32 - active_center as i32;
            ((delta * 1000) / span).clamp(0, 1000)
        };

        if self.invert {
            -val as i16
        } else {
            val as i16
        }
    }
}

use crate::adc::{ADC_CENTER, ADC_MAX, ADC_MIN};

#[derive(Copy, Clone, Debug)]
pub struct InputCalibration {
    pub roll: AxisCalib,
    pub pitch: AxisCalib,
    pub throttle: AxisCalib,
    pub yaw: AxisCalib,
    pub aux: [AxisCalib; 10], // 0..3: SA..SD, 4..5: VRA..VRB, 6..9: VRC..VRF
    pub adc_modes: [u8; 10],
    pub filtered_battery_mv: u32,
    pub ext_switches: bool,
    pub ext_adc: bool,
}

impl InputCalibration {
    pub const fn default_factory() -> Self {
        Self {
            roll: AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, true),
            pitch: AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, true),
            throttle: AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false),
            yaw: AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false),
            aux: [
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // SA
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // SB
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // SC
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // SD
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // VRA
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // VRB
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // VRC (PC2)
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // VRD (PC3)
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // VRE (PC4)
                AxisCalib::new(ADC_MIN, ADC_CENTER, ADC_MAX, false), // VRF (PC5)
            ],
            adc_modes: [0; 10],
            filtered_battery_mv: 0,
            ext_switches: false,
            ext_adc: false,
        }
    }
}

use core::cell::UnsafeCell;

struct InputManagerCell(UnsafeCell<InputCalibration>);
unsafe impl Sync for InputManagerCell {}

static INPUT_MANAGER: InputManagerCell =
    InputManagerCell(UnsafeCell::new(InputCalibration::default_factory()));

/// Apply a full set of stick and pot calibration endpoints.
pub fn apply_calibration(config: &RadioConfig) {
    let calib = unsafe { &mut *INPUT_MANAGER.0.get() };

    calib.ext_switches = config.ext_switches != 0;
    calib.ext_adc = config.ext_adc != 0;
    calib.adc_modes = config.adc_modes;

    // Roll: PA0 (RH)
    calib.roll.invert = true;
    calib.roll.min = config.sticks[0].min;
    calib.roll.center = config.sticks[0].center;
    calib.roll.max = config.sticks[0].max;

    // Pitch: PA1 (RV)
    calib.pitch.invert = true;
    calib.pitch.min = config.sticks[1].min;
    calib.pitch.center = config.sticks[1].center;
    calib.pitch.max = config.sticks[1].max;

    // Throttle: PA2 (LV)
    calib.throttle.invert = false;
    calib.throttle.min = config.sticks[2].min;
    calib.throttle.center = config.sticks[2].center;
    calib.throttle.max = config.sticks[2].max;

    // Yaw: PA3 (LH)
    calib.yaw.invert = false;
    calib.yaw.min = config.sticks[3].min;
    calib.yaw.center = config.sticks[3].center;
    calib.yaw.max = config.sticks[3].max;

    // 10 Auxiliary Analog Channels
    for i in 0..10 {
        let mode = AdcInputMode::resolve(i, config.adc_modes[i]);
        calib.aux[i].invert = false;
        calib.aux[i].min = config.aux_pots[i].min;
        calib.aux[i].max = config.aux_pots[i].max;

        if mode == AdcInputMode::PotDetent {
            // Fixed at the captured physical detent center
            calib.aux[i].center = config.aux_pots[i].center;
        } else {
            // Standard continuous pot (or switch): clean calculated arithmetic midpoint
            calib.aux[i].center =
                ((config.aux_pots[i].min as u32 + config.aux_pots[i].max as u32) / 2) as u16;
        }
    }
}

pub fn set_ext_switches_enabled(enabled: bool) {
    let calib = unsafe { &mut *INPUT_MANAGER.0.get() };
    calib.ext_switches = enabled;
}

pub fn is_ext_switches_enabled() -> bool {
    let calib = unsafe { &*INPUT_MANAGER.0.get() };
    calib.ext_switches
}

pub fn set_ext_adc_enabled(enabled: bool) {
    let calib = unsafe { &mut *INPUT_MANAGER.0.get() };
    calib.ext_adc = enabled;
}

pub fn is_ext_adc_enabled() -> bool {
    let calib = unsafe { &*INPUT_MANAGER.0.get() };
    calib.ext_adc
}

/// Map raw analog reading to a 6-position flight mode index (1..6).
pub fn decode_switch_6pos_num(raw: u16) -> u8 {
    if raw < 682 {
        1
    } else if raw < 1365 {
        2
    } else if raw < 2048 {
        3
    } else if raw < 2730 {
        4
    } else if raw < 3413 {
        5
    } else {
        6
    }
}

/// Decode a 3-position switch from raw ADC counts.
pub fn decode_switch_3pos(raw: u16) -> SwitchPos {
    if raw < 1365 {
        SwitchPos::Up
    } else if raw < 2730 {
        SwitchPos::Mid
    } else {
        SwitchPos::Down
    }
}

/// Decode a 2-position switch from raw ADC counts.
pub fn decode_switch_2pos(raw: u16) -> SwitchPos {
    if raw < 2048 {
        SwitchPos::Up
    } else {
        SwitchPos::Down
    }
}

/// Map an aux channel index (0..9) to raw ADC channel index.
#[inline(always)]
fn aux_index_to_raw_adc(ch: usize) -> usize {
    match ch {
        0 => 4,  // SA: PA4
        1 => 5,  // SB: PA5
        2 => 8,  // SC: PB0
        3 => 9,  // SD: PB1
        4 => 6,  // VRA: PA6
        5 => 7,  // VRB: PA7
        6 => 11, // VRC: PC2 (P7 Header)
        7 => 12, // VRD: PC3 (P7 Header)
        8 => 13, // VRE: PC4 (P7 Header)
        _ => 14, // VRF: PC5 (P7 Header)
    }
}

/// Process a single complete sample frame of raw ADC channels into an `InputState`.
pub fn process(raw: [u16; adc::NUM_CHANNELS]) -> InputState {
    let calib = unsafe { &mut *INPUT_MANAGER.0.get() };

    // Process primary stick axes
    let sticks = Sticks {
        roll: calib.roll.normalize(raw[0]),
        pitch: calib.pitch.normalize(raw[1]),
        throttle: calib.throttle.normalize(raw[2]),
        yaw: calib.yaw.normalize(raw[3]),
    };

    // Process auxiliary analog channels (0..9)
    let mut aux_pots = [0i16; 10];
    let max_channels = if calib.ext_adc { 10 } else { 6 };

    for i in 0..max_channels {
        let raw_val = raw[aux_index_to_raw_adc(i)];
        let mode = AdcInputMode::resolve(i, calib.adc_modes[i]);

        aux_pots[i] = match mode {
            AdcInputMode::PotDetent => calib.aux[i].normalize_detent(raw_val, DETENT_DEADBAND),
            AdcInputMode::Pot => calib.aux[i].normalize(raw_val),
            AdcInputMode::TwoPos | AdcInputMode::InstantTrim => {
                if raw_val < 2048 {
                    -1000
                } else {
                    1000
                }
            }
            AdcInputMode::ThreePos => {
                if raw_val < 1365 {
                    -1000
                } else if raw_val < 2730 {
                    0
                } else {
                    1000
                }
            }
            AdcInputMode::SixPos => {
                let step = decode_switch_6pos_num(raw_val);
                -1000 + ((step as i16 - 1) * 400)
            }
            AdcInputMode::Default => calib.aux[i].normalize(raw_val),
        };
    }

    let pots = Pots {
        vr1: aux_pots[4],
        vr2: aux_pots[5],
        vr3: aux_pots[6],
        vr4: aux_pots[7],
        vr5: aux_pots[8],
        vr6: aux_pots[9],
    };

    // Switches decoding
    let sa = match AdcInputMode::resolve(0, calib.adc_modes[0]) {
        AdcInputMode::ThreePos => decode_switch_3pos(raw[4]),
        _ => decode_switch_2pos(raw[4]),
    };
    let sb = match AdcInputMode::resolve(1, calib.adc_modes[1]) {
        AdcInputMode::TwoPos => decode_switch_2pos(raw[5]),
        _ => decode_switch_3pos(raw[5]),
    };
    let sc = match AdcInputMode::resolve(2, calib.adc_modes[2]) {
        AdcInputMode::TwoPos => decode_switch_2pos(raw[8]),
        _ => decode_switch_3pos(raw[8]),
    };
    let sd = match AdcInputMode::resolve(3, calib.adc_modes[3]) {
        AdcInputMode::ThreePos => decode_switch_3pos(raw[9]),
        _ => decode_switch_2pos(raw[9]),
    };

    // External switches PC12 (SE) and PC15 (SF)
    let (se, sf) = if calib.ext_switches {
        let sw_e = if raw[11] < 2048 {
            SwitchPos::Up
        } else {
            SwitchPos::Down
        };
        let sw_f = if raw[12] < 2048 {
            SwitchPos::Up
        } else {
            SwitchPos::Down
        };
        (sw_e, sw_f)
    } else {
        (SwitchPos::Up, SwitchPos::Up)
    };

    let switches = Switches {
        sa,
        sb,
        sc,
        sd,
        se,
        sf,
    };

    let instant_mv = calculate_battery_mv(raw[10]); // PC0
    let battery_mv = if calib.filtered_battery_mv == 0 {
        calib.filtered_battery_mv = (instant_mv as u32) << 8;
        instant_mv
    } else {
        // Exponential moving average filter (alpha = 1/32) to stabilize hundredths digit
        calib.filtered_battery_mv = calib.filtered_battery_mv - (calib.filtered_battery_mv >> 5)
            + ((instant_mv as u32) << 3);
        (calib.filtered_battery_mv >> 8) as u16
    };

    InputState {
        sticks,
        pots,
        switches,
        battery_mv,
        aux_pots,
        raw,
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

/// Read the latest frame from the ADC and return the processed input state.
pub fn read() -> InputState {
    let raw = adc::read_raw();
    process(raw)
}

/// Initialize input subsystem, load saved calibration from flash, and prime filters.
pub fn init() {
    adc::wait_first_conversion();
    let cfg = crate::storage::load_config();
    apply_calibration(&cfg);

    // Prime the jitter and noise filters across 16 sample cycles
    const SAMPLES: u32 = 16;
    for _ in 0..SAMPLES {
        let raw = adc::read_raw();
        let _ = process(raw);
    }
}
