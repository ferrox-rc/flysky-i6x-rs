//! I2C EEPROM storage (FT24C128A 16 KB) for non-volatile configuration and 20-model memory system.
//!
//! Pinout on FS-i6X mainboard:
//! - PB6: I2C1_SCL
//! - PB7: I2C1_SDA
//!
//! Physical capacity: 16,384 bytes (256 pages x 64 bytes).
//! Slot layout:
//! - 0x0000..0x0100 (256 B / 4 pages): Primary Radio Config (Slot A)
//! - 0x0100..0x0200 (256 B / 4 pages): Backup Radio Config (Slot B - A/B safe save)
//! - 0x0200..0x3E00 (15,360 B / 240 pages): 20 Models x 768 B (12 pages each)
//! - 0x3E00..0x4000 (512 B / 8 pages): Reserved / scratchpad

pub const FLASH_MAGIC: u32 = 0x4653_4B59; // "FSKY"
pub const CONFIG_VERSION: u32 = 5;
pub const NUM_MODELS: usize = 20;

/// 8-byte framing envelope magic ("FX" in little-endian = 0x58, 0x46 -> 0x4658)
pub const SLOT_MAGIC: u16 = 0x4658;
pub const SCHEMA_VERSION: u8 = 1;
pub const HEADER_SIZE: usize = 8;

pub const PAGE_SIZE: usize = 64;
pub const EEPROM_TOTAL_SIZE: usize = 16_384;

pub const RADIO_SLOT_A_ADDR: u16 = 0x0000;
pub const RADIO_SLOT_B_ADDR: u16 = 0x0100;
pub const RADIO_SLOT_SIZE: usize = 256;

pub const MODEL_BASE_ADDR: u16 = 0x0200;
pub const MODEL_SLOT_SIZE: usize = 768;

#[inline(always)]
pub const fn model_slot_addr(index: usize) -> Option<u16> {
    if index >= NUM_MODELS {
        None
    } else {
        Some(MODEL_BASE_ADDR + (index as u16) * (MODEL_SLOT_SIZE as u16))
    }
}

/// Calibration parameters for a single analog channel (8 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ChannelCalib {
    pub min: u16,
    pub center: u16,
    pub max: u16,
    pub _pad: u16,
}

impl ChannelCalib {
    pub const fn new(min: u16, center: u16, max: u16) -> Self {
        Self {
            min,
            center,
            max,
            _pad: 0,
        }
    }
}

/// Compact calibration parameters for an auxiliary potentiometer/slider (6 bytes).
/// Supports both standard pots (with calculated midpoint) and detented pots (with fixed detent center).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PotCalib {
    pub min: u16,
    pub center: u16,
    pub max: u16,
}

impl PotCalib {
    pub const fn new(min: u16, center: u16, max: u16) -> Self {
        Self { min, center, max }
    }
}

/// Universal input mode for auxiliary analog channels (SA..SD, VRA..VRB, VRC..VRF).
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AdcInputMode {
    #[default]
    Default = 0,
    TwoPos = 1,
    ThreePos = 2,
    SixPos = 3,
    Pot = 4,
    PotDetent = 5,
    InstantTrim = 6,
}

impl AdcInputMode {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::TwoPos,
            2 => Self::ThreePos,
            3 => Self::SixPos,
            4 => Self::Pot,
            5 => Self::PotDetent,
            6 => Self::InstantTrim,
            _ => Self::Default,
        }
    }

    /// Check if this mode represents any analog potentiometer (continuous or center-detent).
    pub fn is_pot(&self) -> bool {
        matches!(self, Self::Pot | Self::PotDetent)
    }

    /// Check if this mode utilizes a physical center detent.
    pub fn has_detent(&self) -> bool {
        matches!(self, Self::PotDetent)
    }

    /// Resolve effective mode for a given channel index (0..3: SA..SD, 4..5: VRA..VRB, 6..9: VRC..VRF)
    pub fn resolve(channel_idx: usize, mode_val: u8) -> Self {
        let mode = Self::from_u8(mode_val);
        if mode != Self::Default {
            return mode;
        }
        // Default hardware mapping:
        match channel_idx {
            0 => Self::TwoPos,               // SA (2-pos stock)
            1 => crate::hw::DEFAULT_SB_MODE, // SB (2-pos stock on i6X, 3-pos stock on i6S)
            2 => Self::ThreePos,             // SC (3-pos stock)
            3 => Self::TwoPos,               // SD (2-pos stock)
            _ => Self::Pot,                  // VRA, VRB, VRC, VRD, VRE, VRF (Pots stock)
        }
    }
}

/// Persistent system/radio-level configuration (140 bytes, 4-byte aligned for STM32F0 Flash programming).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RadioConfig {
    pub magic: u32,                // 0..4 ("FSKY")
    pub version: u32,              // 4..8 (5)
    pub active_model: u8,          // 8 (0..19, active model index)
    pub throttle_trim: u8,         // 9 (0: Off/Lock, 1: Idle T-Trim, 2: Linear)
    pub audio_enabled: u8,         // 10 (0: Muted, 1: Enabled)
    pub backlight_timeout: u8,     // 11 (0: Always On, 1: 15s, 2: 30s, 3: 60s)
    pub backlight_brightness: u8,  // 12 (1..10)
    pub vbat_warn_deci: u8,        // 13 (40..50 = 4.0V..5.0V, default 44 = 4.4V)
    pub lcd_contrast: u8,          // 14 (15..55, default 37 / 0x25)
    pub usb_mode: u8,              // 15 (0: Off, 1: Joystick, 2: Serial, 3: Composite)
    pub sticks: [ChannelCalib; 4], // 16..48 (32 bytes: Roll, Pitch, Throttle, Yaw)
    pub aux_pots: [PotCalib; 10],  // 48..108 (60 bytes: SA..SD, VRA..VRB, VRC..VRF)
    pub ext_module_pwr: u8,        // 108 (0: Active HIGH / N-type, 1: Active LOW / P-type)
    pub tone_style: u8,            // 109 (0: Simple / Standard, 1: Rich / Melodic)
    pub servo_rate_hz: u16,        // 110..112 (50..400 Hz, default 50 Hz for analog servo safety)
    pub rx_out_mode: u8,           // 112 (0: PWM, 1: PPM, default 0)
    pub rx_serial_proto: u8,       // 113 (0: i-BUS, 1: S.BUS, default 0)
    pub ext_switches: u8,          // 114 (0: Disabled, 1: Enabled / PC12+PC15)
    pub ext_adc: u8,               // 115 (0: Disabled, 1: Enabled / Header P7 AD12-AD15 or PC1-PC4)
    pub adc_modes: [u8; 10],       // 116..126: Input modes for the 10 auxiliary analog channels
    pub stick_mode: u8,            // 126 (0: Mode 1, 1: Mode 2, 2: Mode 3, 3: Mode 4)
    pub telem_slots: [u8; 4],      // 127..131: Configurable telemetry dashboard sensor slots (1..4)
    pub trainer_config: u8,        // 131: Bit 0: PPM Out on PF10, Bit 1: PPM In on PF9
    pub rear_left_func: u8,        // 132: 0: RC Switch (SWE), 1: Timer Reset, 2: Trims Modifier
    pub rear_right_func: u8,       // 133: 0: RC Switch (SWF), 1: Instant Trim, 2: Trims Modifier
    pub touch_disabled: u8,        // 134: 0: Touch Enabled, 1: Touch Disabled
    pub j4_sg_sh_en: u8,           // 135: 0: SWD Debug, 1: SG/SH Buttons on PA13/PA14
    pub switch_beep: u8,           // 136: 0: Muted, 1: Beep on switch toggle
    pub _pad_align: [u8; 3],       // 137..140: Align to 140 bytes (4-byte word boundary)
}

impl RadioConfig {
    pub const fn default_factory() -> Self {
        Self {
            magic: FLASH_MAGIC,
            version: CONFIG_VERSION,
            active_model: 0,
            throttle_trim: 0,
            audio_enabled: 1,
            backlight_timeout: 0,
            backlight_brightness: 10,
            vbat_warn_deci: 44,
            lcd_contrast: 37,
            usb_mode: 0,
            sticks: [
                ChannelCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // Roll (Horizontal)
                ChannelCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // Pitch (Vertical)
                ChannelCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // Throttle (Vertical)
                ChannelCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // Yaw (Horizontal)
            ],
            aux_pots: [
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // SA
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // SB
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // SC
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // SD
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // VRA
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // VRB
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // VRC
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // VRD
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // VRE
                PotCalib::new(
                    crate::adc::ADC_MIN,
                    crate::adc::ADC_CENTER,
                    crate::adc::ADC_MAX,
                ), // VRF
            ],
            ext_module_pwr: 0,
            tone_style: 1,
            servo_rate_hz: 50,
            rx_out_mode: 0,
            rx_serial_proto: 0,
            ext_switches: 0,
            ext_adc: 0,
            adc_modes: [0; 10], // Default hardware modes
            stick_mode: 1,      // Mode 2 default
            telem_slots: [0, 1, 2, 3], // Default: RxBat, RSSI/LQ, TxBat, Current/Link
            trainer_config: 0,
            rear_left_func: 0,
            rear_right_func: 0,
            touch_disabled: 0,
            j4_sg_sh_en: 0,
            switch_beep: 1,
            _pad_align: [0; 3],
        }
    }

    /// Check if PPM Output on PF10 is enabled.
    #[inline(always)]
    pub fn is_ppm_out_enabled(&self) -> bool {
        (self.trainer_config & 0x01) != 0
    }

    /// Check if PPM Input on PF9 is enabled.
    #[inline(always)]
    pub fn is_ppm_in_enabled(&self) -> bool {
        (self.trainer_config & 0x02) != 0
    }

    /// Set PPM Output on PF10 enabled state.
    #[inline(always)]
    pub fn set_ppm_out_enabled(&mut self, enabled: bool) {
        if enabled {
            self.trainer_config |= 0x01;
        } else {
            self.trainer_config &= !0x01;
        }
    }

    /// Set PPM Input on PF9 enabled state.
    #[inline(always)]
    pub fn set_ppm_in_enabled(&mut self, enabled: bool) {
        if enabled {
            self.trainer_config |= 0x02;
        } else {
            self.trainer_config &= !0x02;
        }
    }

    /// Get raw configured ADC mode index (0..=6) for auxiliary input `ch` (`0..10`).
    #[inline(always)]
    pub const fn get_adc_mode_val(&self, ch: usize) -> u8 {
        if ch < 10 {
            self.adc_modes[ch]
        } else {
            0
        }
    }

    /// Set raw configured ADC mode index (0..=6) for auxiliary input `ch` (`0..10`).
    #[inline(always)]
    pub fn set_adc_mode_val(&mut self, ch: usize, val: u8) {
        if ch < 10 {
            self.adc_modes[ch] = val.min(6);
        }
    }

    /// Get the effective AdcInputMode for an auxiliary channel (0..9: SA..SD, VRA..VRB, VRC..VRF).
    pub fn get_adc_mode(&self, ch: usize) -> AdcInputMode {
        if ch < 10 {
            AdcInputMode::resolve(ch, self.adc_modes[ch])
        } else {
            AdcInputMode::Default
        }
    }

    /// Get the configured raw AdcInputMode (including Default).
    pub fn get_raw_adc_mode(&self, ch: usize) -> AdcInputMode {
        if ch < 10 {
            AdcInputMode::from_u8(self.adc_modes[ch])
        } else {
            AdcInputMode::Default
        }
    }
}

/// A single freeform mix rule in the matrix mixer (6 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MixLine {
    pub target_ch: u8, // 0: Disabled, 1..18: Target Channel CH1..CH18
    pub source: u8, // 0: None, 1: Roll, 2: Pitch, 3: Thr, 4: Yaw, 5: VRA, 6: VRB, 7: SA, 8: SB, 9: SC, 10: SD, 11: MAX, 12..29: CH1..CH18, 30: ThrUnipolar, 31..42: SE..VRF
    pub weight: i8, // -100% .. +100%
    pub offset: i8, // -100% .. +100%
    pub switch: u8, // 0: Always On, 1..26: SA_UP..SWL_DN
    pub mode: u8,   // 0: Add (+), 1: Multiply (*), 2: Replace (:=)
}

impl MixLine {
    pub const fn disabled() -> Self {
        Self {
            target_ch: 0,
            source: 0,
            weight: 100,
            offset: 0,
            switch: 0,
            mode: 0,
        }
    }
}

/// Complete profile for an individual aircraft/model (exactly 128 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ModelConfig {
    pub name: [u8; 10],         // 0..10: 10-char ASCII name
    pub model_type: u8,         // 10: 0: Airplane, 1: Glider, 2: Heli, 3: Quad, 4: General
    pub _pad0: u8,              // 11: align rx_id to 4 bytes
    pub rx_id: u32,             // 12..16: Model Match bound receiver ID
    pub trims: [i8; 4],         // 16..20: Roll, Pitch, Throttle, Yaw (-25..+25)
    pub channel_reverse: u32,   // 20..24: Bitmask for CH1..CH18 inversion
    pub dr_switch: u8,          // 24: 0: None, 1: SA, 2: SB, 3: SC, 4: SD, 5: SE, 6: SF
    pub thr_curve_pts: u8,      // 25: 5 or 9 points mode
    pub thr_curve_smooth: u8,   // 26: 0: Linear, 1: Spline / Smooth
    pub thr_curve: [u8; 9],     // 27..36: Points 1..9 (0..100%)
    pub dr_high: [u8; 3],       // 36..39: High rate (AIL, ELE, RUD: 50..100%)
    pub dr_low: [u8; 3],        // 39..42: Low rate (AIL, ELE, RUD: 30..100%)
    pub expo_high: [i8; 3],     // 42..45: High expo (-100..+100%)
    pub expo_low: [i8; 3],      // 45..48: Low expo (-100..+100%)
    pub timer_secs: u16,        // 48..50: Countdown timer in seconds (e.g. 300 = 5 min)
    pub timer_source: u8, // 50: 0: Off, 1: THs (Thr>5%), 2: THt (Thr Latched), 3: Always On, 4..17: SA^..SFv
    pub rf_power: u8, // 51: AFHDS 2A RF power (0: 100mW, 1: 10mW, 2: 1mW, 3: 100uW)
    pub failsafe_thr: u16, // 52..54: Failsafe throttle pulse in µs (e.g. 1000)
    pub aux_channels: [u8; 14], // 54..68: Source for CH5..CH18
    pub wing_tail_mix: u8, // 68: 0: Normal, 1: Elevon/Delta, 2: V-Tail, 3: Flaperon
    pub template_diff: i8, // 69: Differential / mix ratio (-100..+100)
    pub mixes: [MixLine; 8], // 70..118: 8 freeform mix rules (8 * 6 = 48 bytes)
    pub failsafe_mode: u8, // 118: 0: Hold last, 1: Custom pulses
    pub failsafe_timeout: u8, // 119: 10..50 (1.0s..5.0s)
    pub rf_protocol: u8,  // 120: 0: AFHDS 2A, 1: CRSF / ELRS
    pub crsf_baud: u8,    // 121: 0: 115.2k, 1: 416.6k, 2: 420k, 3: 921.6k, 4: 1.875M
    pub arm_switch: u8, // 122: 0: None, 1..14: SA^..SFv
    pub rx_out_mode: u8, // 123: 0: PWM, 1: PPM (default 0)
    pub servo_rate_hz: u16, // 124..126: 50..400 Hz (default 50 Hz)
    pub rx_serial_proto: u8, // 126: 0: i-BUS, 1: S.BUS (default 0)
    pub crsf_half_duplex: u8, // 127: 0: Full-Duplex (2-Wire), 1: Half-Duplex (1-Wire)
}

impl ModelConfig {
    pub const fn default_for_index(idx: usize) -> Self {
        let num = (idx + 1) as u8;
        let digit1 = b'0' + (num / 10);
        let digit2 = b'0' + (num % 10);
        Self {
            name: [
                b'M', b'O', b'D', b'E', b'L', b' ', digit1, digit2, b' ', b' ',
            ],
            model_type: 0,
            _pad0: 0,
            rx_id: 0,
            trims: [0, 0, 0, 0],
            channel_reverse: 0,
            dr_switch: 0,
            thr_curve_pts: 5,
            thr_curve_smooth: 0,
            thr_curve: [0, 25, 50, 75, 100, 0, 0, 0, 0],
            dr_high: [100, 100, 100],
            dr_low: [70, 70, 70],
            expo_high: [0, 0, 0],
            expo_low: [0, 0, 0],
            timer_secs: 300,
            timer_source: 1,
            rf_power: 0,
            failsafe_thr: 1000,
            aux_channels: [7, 8, 5, 6, 9, 10, 0, 0, 0, 0, 0, 0, 0, 0],
            wing_tail_mix: 0,
            template_diff: 0,
            mixes: [MixLine::disabled(); 8],
            failsafe_mode: 0,
            failsafe_timeout: 20,
            rf_protocol: 0,
            crsf_baud: 2,
            arm_switch: 0,
            servo_rate_hz: 50,
            rx_out_mode: 0,
            rx_serial_proto: 0,
            crsf_half_duplex: 0,
        }
    }

    #[inline(always)]
    pub fn model_type(&self) -> crate::safety::ModelType {
        crate::safety::ModelType::from_u8(self.model_type)
    }

    #[inline(always)]
    pub fn set_model_type(&mut self, mtype: crate::safety::ModelType) {
        self.model_type = mtype.to_u8();
    }
}

/// Complete Flash storage layout containing radio settings and 20 models (2,698 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct RadioStorage {
    pub radio: RadioConfig,
    pub models: [ModelConfig; NUM_MODELS],
}

impl RadioStorage {
    pub const fn empty() -> Self {
        Self {
            radio: RadioConfig::default_factory(),
            models: [ModelConfig::default_for_index(0); NUM_MODELS],
        }
    }

    #[allow(dead_code)]
    pub const fn default_factory() -> Self {
        let mut models = [ModelConfig::default_for_index(0); NUM_MODELS];
        let mut i = 0;
        while i < NUM_MODELS {
            models[i] = ModelConfig::default_for_index(i);
            i += 1;
        }
        Self {
            radio: RadioConfig::default_factory(),
            models,
        }
    }

    pub fn init_default(&mut self) {
        self.radio = RadioConfig::default_factory();
        let mut i = 0;
        while i < NUM_MODELS {
            self.models[i] = ModelConfig::default_for_index(i);
            i += 1;
        }
    }

    pub fn active_model(&self) -> &ModelConfig {
        let idx = (self.radio.active_model as usize).min(NUM_MODELS - 1);
        &self.models[idx]
    }

    pub fn active_model_mut(&mut self) -> &mut ModelConfig {
        let idx = (self.radio.active_model as usize).min(NUM_MODELS - 1);
        &mut self.models[idx]
    }

    /// Sanitize all radio and model parameters to valid operating bounds.
    pub fn sanitize(&mut self) {
        self.radio.magic = FLASH_MAGIC;
        self.radio.version = CONFIG_VERSION;
        if self.radio.vbat_warn_deci < 35 || self.radio.vbat_warn_deci > 60 {
            self.radio.vbat_warn_deci = 44;
        }
        if self.radio.active_model >= NUM_MODELS as u8 {
            self.radio.active_model = 0;
        }
        if self.radio.backlight_brightness == 0 || self.radio.backlight_brightness > 10 {
            self.radio.backlight_brightness = 10;
        }
        if self.radio.lcd_contrast < 15 || self.radio.lcd_contrast > 55 {
            self.radio.lcd_contrast = 37;
        }
        if self.radio.usb_mode > 3 {
            self.radio.usb_mode = 0;
        }
        if self.radio.ext_module_pwr > 1 {
            self.radio.ext_module_pwr = 0;
        }
        if self.radio.throttle_trim > 2 {
            self.radio.throttle_trim = 0;
        }
        if self.radio.tone_style > 1 {
            self.radio.tone_style = 1;
        }
        if self.radio.servo_rate_hz < 50 || self.radio.servo_rate_hz > 400 {
            self.radio.servo_rate_hz = 50;
        }
        if self.radio.rx_out_mode > 1 {
            self.radio.rx_out_mode = 0;
        }
        if self.radio.rx_serial_proto > 1 {
            self.radio.rx_serial_proto = 0;
        }
        if self.radio.ext_switches > 1 {
            self.radio.ext_switches = 0;
        }
        if self.radio.rear_left_func > 2 {
            self.radio.rear_left_func = 0;
        }
        if self.radio.rear_right_func > 2 {
            self.radio.rear_right_func = 0;
        }
        if self.radio.touch_disabled > 1 {
            self.radio.touch_disabled = 0;
        }
        if self.radio.j4_sg_sh_en > 1 {
            self.radio.j4_sg_sh_en = 0;
        }
        if self.radio.switch_beep > 1 {
            self.radio.switch_beep = 1;
        }

        for stick in self.radio.sticks.iter_mut() {
            if stick.min >= stick.center
                || stick.center >= stick.max
                || stick.max > crate::adc::ADC_MAX
            {
                stick.min = crate::adc::ADC_MIN;
                stick.center = crate::adc::ADC_CENTER;
                stick.max = crate::adc::ADC_MAX;
            }
        }
        if self.radio.ext_adc > 1 {
            self.radio.ext_adc = 0;
        }
        for pot in self.radio.aux_pots.iter_mut() {
            if pot.min >= pot.center || pot.center >= pot.max || pot.max > crate::adc::ADC_MAX {
                pot.min = crate::adc::ADC_MIN;
                pot.center = crate::adc::ADC_CENTER;
                pot.max = crate::adc::ADC_MAX;
            }
        }
        for mode in self.radio.adc_modes.iter_mut() {
            if *mode > 6 {
                *mode = 0; // Default
            }
        }
        for slot in self.radio.telem_slots.iter_mut() {
            if *slot > 12 {
                *slot = 12; // OFF
            }
        }
        if self.radio.trainer_config > 3 {
            self.radio.trainer_config = 0;
        }

        for (idx, m) in self.models.iter_mut().enumerate() {
            if m.model_type > 4 {
                m.model_type = 0;
            }
            if m.rf_protocol > 1 {
                m.rf_protocol = 0;
            }
            if m.crsf_baud > 4 {
                m.crsf_baud = 2;
            }
            if m.crsf_half_duplex > 1 {
                m.crsf_half_duplex = 0;
            }
            for axis in 0..3 {
                if m.dr_high[axis] < 30 || m.dr_high[axis] > 100 {
                    m.dr_high[axis] = 100;
                }
                if m.dr_low[axis] < 30 || m.dr_low[axis] > 100 {
                    m.dr_low[axis] = 70;
                }
                if m.expo_high[axis] < -100 || m.expo_high[axis] > 100 {
                    m.expo_high[axis] = 0;
                }
                if m.expo_low[axis] < -100 || m.expo_low[axis] > 100 {
                    m.expo_low[axis] = 0;
                }
            }
            if m.dr_switch > 6 {
                m.dr_switch = 0;
            }
            if m.wing_tail_mix > 3 {
                m.wing_tail_mix = 0;
            }
            if m.template_diff < -100 || m.template_diff > 100 {
                m.template_diff = 0;
            }
            if m.failsafe_thr < 900 || m.failsafe_thr > 2100 {
                m.failsafe_thr = 1000;
            }
            if m.aux_channels[0..6] == [0; 6] {
                m.aux_channels = [7, 8, 5, 6, 9, 10, 0, 0, 0, 0, 0, 0, 0, 0];
            }
            for src in m.aux_channels.iter_mut() {
                if *src > 42 {
                    *src = 0;
                }
            }
            for mix in m.mixes.iter_mut() {
                if mix.target_ch > 18 || mix.source > 42 || mix.mode > 2 || mix.switch > 26 {
                    *mix = MixLine::disabled();
                }
            }
            if m.thr_curve_pts != 5 && m.thr_curve_pts != 9 {
                m.thr_curve_pts = 5;
                m.thr_curve = [0, 25, 50, 75, 100, 0, 0, 0, 0];
            }
            if m.arm_switch > 14 {
                m.arm_switch = 0;
            }
            if m.timer_source > 17 {
                m.timer_source = 0;
            }
            if m.timer_secs > 3600 {
                m.timer_secs = 0;
            }
            if m.rf_power > 3 {
                m.rf_power = 0;
            }
            if m.servo_rate_hz < 50 || m.servo_rate_hz > 400 {
                m.servo_rate_hz = 50;
            }
            if m.rx_out_mode > 1 {
                m.rx_out_mode = 0;
            }
            if m.rx_serial_proto > 1 {
                m.rx_serial_proto = 0;
            }
            if m.name[0] == 0 || m.name[0] == 0xFF {
                let num = (idx + 1) as u8;
                let digit1 = b'0' + (num / 10);
                let digit2 = b'0' + (num % 10);
                m.name = [
                    b'M', b'O', b'D', b'E', b'L', b' ', digit1, digit2, b' ', b' ',
                ];
            }
        }
    }
}

const _: () = assert!(core::mem::size_of::<RadioConfig>() == 140);
const _: () = assert!(core::mem::size_of::<ModelConfig>() == 128);
const _: () = assert!(core::mem::size_of::<RadioStorage>() == 2700);

static mut GLOBAL_STORAGE: RadioStorage = RadioStorage::empty();

/// Get global mutable reference to persistent RadioStorage.
#[allow(static_mut_refs)]
pub fn get_storage() -> &'static mut RadioStorage {
    unsafe { &mut GLOBAL_STORAGE }
}

pub const EEPROM_I2C_ADDR: u8 = 0x50;

/// Compute standard CRC-16 (IBM / Bisync / SDLC, poly 0x8005, init 0x0000, unreversed).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x8005;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum StorageError {
    I2cError,
    EmptySlot,
    VersionMismatch(u8),
    LengthMismatch(u16),
    CrcMismatch,
    BufferTooSmall,
}

/// 8-byte Slot Header:
/// [0..2] magic: u16 (SLOT_MAGIC = 0x4658, "FX")
/// [2]    version: u8 (SCHEMA_VERSION = 1)
/// [3]    flags: u8 (0 = Valid active slot, 0xFF = Deleted)
/// [4..6] len: u16 LE (exact payload byte length)
/// [6..8] crc16: u16 LE (CRC-16 over payload)
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SlotHeader {
    pub magic: u16,
    pub version: u8,
    pub flags: u8,
    pub len: u16,
    pub crc16: u16,
}

impl SlotHeader {
    pub fn parse(buf: &[u8; HEADER_SIZE]) -> Self {
        Self {
            magic: u16::from_le_bytes([buf[0], buf[1]]),
            version: buf[2],
            flags: buf[3],
            len: u16::from_le_bytes([buf[4], buf[5]]),
            crc16: u16::from_le_bytes([buf[6], buf[7]]),
        }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut out = [0u8; HEADER_SIZE];
        out[0..2].copy_from_slice(&self.magic.to_le_bytes());
        out[2] = self.version;
        out[3] = self.flags;
        out[4..6].copy_from_slice(&self.len.to_le_bytes());
        out[6..8].copy_from_slice(&self.crc16.to_le_bytes());
        out
    }

    pub fn is_valid(&self, expected_len: usize) -> Result<(), StorageError> {
        if self.magic != SLOT_MAGIC {
            return Err(StorageError::EmptySlot);
        }
        if self.version != SCHEMA_VERSION {
            return Err(StorageError::VersionMismatch(self.version));
        }
        if self.flags != 0 {
            return Err(StorageError::EmptySlot);
        }
        if self.len as usize != expected_len {
            return Err(StorageError::LengthMismatch(self.len));
        }
        Ok(())
    }
}

/// Direct hardware I2C1 EEPROM driver for FT24C128A (PB6 = SCL, PB7 = SDA).
#[cfg(not(test))]
pub struct EepromI2c;

#[cfg(not(test))]
impl EepromI2c {
    /// Initialize I2C1 peripheral on PB6 (SCL) and PB7 (SDA) at ~400 kHz Fast Mode.
    pub fn init() {
        let rcc = unsafe { &*stm32f0xx_hal::pac::RCC::ptr() };
        let gpiob = unsafe { &*stm32f0xx_hal::pac::GPIOB::ptr() };
        let i2c1 = unsafe { &*stm32f0xx_hal::pac::I2C1::ptr() };
        let syscfg = unsafe { &*stm32f0xx_hal::pac::SYSCFG::ptr() };

        // 1. Enable GPIOB, SYSCFG, and I2C1 clocks
        rcc.ahbenr.modify(|_, w| w.iopben().enabled());
        rcc.apb2enr.modify(|_, w| w.syscfgen().enabled());
        rcc.apb1enr.modify(|_, w| w.i2c1en().enabled());

        // Configure I2C1 clock source as SYSCLK (48 MHz)
        rcc.cfgr3.modify(|_, w| w.i2c1sw().sysclk());

        // 2. Enable Fast Mode Plus (Fm+ 20 mA high-drive sink) on PB6 and PB7 in SYSCFG
        syscfg.cfgr1.modify(|_, w| w.i2c_pb6_fmp().fmp().i2c_pb7_fmp().fmp());

        // 3. Configure PB6 and PB7 as Alternate Function 1 (I2C1), Open-Drain, High-Speed, Pull-up
        gpiob.afrl.modify(|_, w| w.afrl6().af1().afrl7().af1());
        gpiob.moder.modify(|_, w| w.moder6().alternate().moder7().alternate());
        gpiob.otyper.modify(|_, w| w.ot6().open_drain().ot7().open_drain());
        gpiob.ospeedr.modify(|_, w| w.ospeedr6().high_speed().ospeedr7().high_speed());
        gpiob.pupdr.modify(|_, w| w.pupdr6().pull_up().pupdr7().pull_up());

        // 4. Disable I2C1 while configuring timing
        i2c1.cr1.modify(|_, w| w.pe().disabled());

        // Timing for 48 MHz SYSCLK -> 1 MHz (1000 kHz) Fast Mode Plus (Fm+)
        // PRESC=0, SCLDEL=3 (~80ns), SDADEL=0, SCLH=9 (~250ns), SCLL=25 (~650ns)
        // Total I2C cycle = (1 + 9 + 1 + 25) * 20.83ns = 750ns nominal bus + rise/fall -> 1.0 MHz
        i2c1.timingr.write(|w| {
            w.presc().bits(0)
                .scldel().bits(3)
                .sdadel().bits(0)
                .sclh().bits(9)
                .scll().bits(25)
        });

        // 5. Enable I2C1 peripheral
        i2c1.cr1.modify(|_, w| w.pe().enabled());
    }

    /// Read sequential bytes starting at 16-bit EEPROM `addr`.
    pub fn read(addr: u16, data: &mut [u8]) -> Result<(), StorageError> {
        if data.is_empty() {
            return Ok(());
        }

        let i2c1 = unsafe { &*stm32f0xx_hal::pac::I2C1::ptr() };

        // 1. Send 2-byte memory address write header without STOP (autoend=false)
        let addr_bytes = addr.to_be_bytes();
        i2c1.cr2.write(|w| {
            w.sadd().bits((EEPROM_I2C_ADDR << 1) as u16)
                .nbytes().bits(2)
                .rd_wrn().write()
                .autoend().software()
                .start().start()
        });

        for &byte in &addr_bytes {
            let mut timeout = 100_000u32;
            while i2c1.isr.read().txis().bit_is_clear() {
                if i2c1.isr.read().nackf().bit_is_set() {
                    i2c1.icr.write(|w| w.nackcf().clear());
                    return Err(StorageError::I2cError);
                }
                timeout -= 1;
                if timeout == 0 {
                    return Err(StorageError::I2cError);
                }
            }
            i2c1.txdr.write(|w| w.txdata().bits(byte));
        }

        // Wait for TC (Transfer Complete)
        let mut timeout = 100_000u32;
        while i2c1.isr.read().tc().bit_is_clear() {
            timeout -= 1;
            if timeout == 0 {
                return Err(StorageError::I2cError);
            }
        }

        // 2. Read sequential data bytes in chunks of up to 255 bytes (NBYTES limitation)
        let mut read_offset = 0;
        while read_offset < data.len() {
            let chunk_len = core::cmp::min(255, data.len() - read_offset);
            let is_last = (read_offset + chunk_len) == data.len();

            i2c1.cr2.write(|w| {
                w.sadd().bits((EEPROM_I2C_ADDR << 1) as u16)
                    .nbytes().bits(chunk_len as u8)
                    .rd_wrn().read()
                    .autoend().bit(is_last)
                    .start().start()
            });

            for i in 0..chunk_len {
                let mut byte_timeout = 100_000u32;
                while i2c1.isr.read().rxne().bit_is_clear() {
                    byte_timeout -= 1;
                    if byte_timeout == 0 {
                        return Err(StorageError::I2cError);
                    }
                }
                data[read_offset + i] = i2c1.rxdr.read().rxdata().bits();
            }

            if !is_last {
                let mut tc_timeout = 100_000u32;
                while i2c1.isr.read().tc().bit_is_clear() {
                    tc_timeout -= 1;
                    if tc_timeout == 0 {
                        return Err(StorageError::I2cError);
                    }
                }
            } else {
                let mut stop_timeout = 100_000u32;
                while i2c1.isr.read().stopf().bit_is_clear() {
                    stop_timeout -= 1;
                    if stop_timeout == 0 {
                        break;
                    }
                }
                i2c1.icr.write(|w| w.stopcf().clear());
            }

            read_offset += chunk_len;
        }

        Ok(())
    }

    /// Write a single page chunk (up to 64 bytes) that does NOT cross a 64-byte boundary.
    fn write_page_aligned(addr: u16, chunk: &[u8]) -> Result<(), StorageError> {
        if chunk.is_empty() {
            return Ok(());
        }

        let i2c1 = unsafe { &*stm32f0xx_hal::pac::I2C1::ptr() };
        let total_bytes = (2 + chunk.len()) as u8;
        let addr_bytes = addr.to_be_bytes();

        i2c1.cr2.write(|w| {
            w.sadd().bits((EEPROM_I2C_ADDR << 1) as u16)
                .nbytes().bits(total_bytes)
                .rd_wrn().write()
                .autoend().automatic()
                .start().start()
        });

        // 1. Send 2 address bytes
        for &byte in &addr_bytes {
            let mut timeout = 100_000u32;
            while i2c1.isr.read().txis().bit_is_clear() {
                if i2c1.isr.read().nackf().bit_is_set() {
                    i2c1.icr.write(|w| w.nackcf().clear());
                    return Err(StorageError::I2cError);
                }
                timeout -= 1;
                if timeout == 0 {
                    return Err(StorageError::I2cError);
                }
            }
            i2c1.txdr.write(|w| w.txdata().bits(byte));
        }

        // 2. Send payload bytes
        for &byte in chunk {
            let mut timeout = 100_000u32;
            while i2c1.isr.read().txis().bit_is_clear() {
                if i2c1.isr.read().nackf().bit_is_set() {
                    i2c1.icr.write(|w| w.nackcf().clear());
                    return Err(StorageError::I2cError);
                }
                timeout -= 1;
                if timeout == 0 {
                    return Err(StorageError::I2cError);
                }
            }
            i2c1.txdr.write(|w| w.txdata().bits(byte));
        }

        // Wait for STOPF
        let mut stop_timeout = 100_000u32;
        while i2c1.isr.read().stopf().bit_is_clear() {
            stop_timeout -= 1;
            if stop_timeout == 0 {
                break;
            }
        }
        i2c1.icr.write(|w| w.stopcf().clear());

        // 3. ACK Polling for EEPROM internal write cycle (max 5ms)
        Self::poll_ack();

        Ok(())
    }

    /// Poll I2C address until EEPROM ACKs that its internal self-timed write cycle has completed.
    fn poll_ack() {
        let i2c1 = unsafe { &*stm32f0xx_hal::pac::I2C1::ptr() };
        let mut retries = 50u32; // up to ~10ms total polling

        while retries > 0 {
            crate::watchdog::feed();
            retries -= 1;

            // Send 0-byte write to test device readiness
            i2c1.cr2.write(|w| {
                w.sadd().bits((EEPROM_I2C_ADDR << 1) as u16)
                    .nbytes().bits(0)
                    .rd_wrn().write()
                    .autoend().automatic()
                    .start().start()
            });

            let mut wait = 20_000u32;
            let mut got_nack = false;
            while i2c1.isr.read().stopf().bit_is_clear() && wait > 0 {
                if i2c1.isr.read().nackf().bit_is_set() {
                    got_nack = true;
                    i2c1.icr.write(|w| w.nackcf().clear());
                }
                wait -= 1;
            }
            i2c1.icr.write(|w| w.stopcf().clear());

            if !got_nack && wait > 0 {
                return; // Device ACKed, write complete!
            }

            {
                let start = crate::time::millis();
                while crate::time::millis().wrapping_sub(start) < 1 {
                    crate::watchdog::feed();
                }
            }
        }
    }

    /// Write arbitrary slice with automatic 64-byte page boundary splitting to prevent wrap-around.
    pub fn write(mut addr: u16, data: &[u8]) -> Result<(), StorageError> {
        let mut written = 0;
        while written < data.len() {
            let page_offset = (addr % (PAGE_SIZE as u16)) as usize;
            let chunk_len = core::cmp::min(PAGE_SIZE - page_offset, data.len() - written);

            let chunk = &data[written..written + chunk_len];
            Self::write_page_aligned(addr, chunk)?;

            addr = addr.saturating_add(chunk_len as u16);
            written += chunk_len;
        }
        Ok(())
    }
}

/// Helper to read a raw C struct from an EEPROM slot with 8-byte framing and CRC16 verification.
pub fn load_slot<T: Copy>(addr: u16, dst: &mut T) -> Result<(), StorageError> {
    #[cfg(test)]
    {
        let _ = addr;
        let _ = dst;
        return Err(StorageError::EmptySlot);
    }

    #[cfg(not(test))]
    {
        // 1. Read 8-byte slot header
        let mut hdr_bytes = [0u8; HEADER_SIZE];
        EepromI2c::read(addr, &mut hdr_bytes)?;
        let hdr = SlotHeader::parse(&hdr_bytes);

        let expected_size = core::mem::size_of::<T>();
        hdr.is_valid(expected_size)?;

        // 2. Read payload directly into destination struct
        let dst_bytes = unsafe {
            core::slice::from_raw_parts_mut(dst as *mut T as *mut u8, expected_size)
        };
        EepromI2c::read(addr + HEADER_SIZE as u16, dst_bytes)?;

        // 3. Verify CRC-16
        let computed_crc = crc16(dst_bytes);
        if computed_crc != hdr.crc16 {
            return Err(StorageError::CrcMismatch);
        }

        Ok(())
    }
}

/// Helper to write a raw C struct into an EEPROM slot with 8-byte framing and CRC16.
pub fn save_slot<T: Copy>(addr: u16, src: &T) -> Result<(), StorageError> {
    #[cfg(test)]
    {
        let _ = addr;
        let _ = src;
        return Ok(());
    }

    #[cfg(not(test))]
    {
        let size = core::mem::size_of::<T>();
        let src_bytes = unsafe {
            core::slice::from_raw_parts(src as *const T as *const u8, size)
        };

        let crc = crc16(src_bytes);
        let hdr = SlotHeader {
            magic: SLOT_MAGIC,
            version: SCHEMA_VERSION,
            flags: 0,
            len: size as u16,
            crc16: crc,
        };

        // Write header first
        let hdr_bytes = hdr.to_bytes();
        EepromI2c::write(addr, &hdr_bytes)?;

        // Write struct payload
        EepromI2c::write(addr + HEADER_SIZE as u16, src_bytes)?;

        Ok(())
    }
}

/// Read only model name and magic without transferring the entire 128-byte ModelConfig.
pub fn peek_model_name(slot_idx: usize) -> Option<[u8; 10]> {
    #[cfg(test)]
    {
        let _ = slot_idx;
        None
    }
    #[cfg(not(test))]
    {
        let addr = model_slot_addr(slot_idx)?;
        // Read 8-byte header + first 10 bytes (ModelConfig::name) = 18 bytes
        let mut buf = [0u8; HEADER_SIZE + 10];
        if EepromI2c::read(addr, &mut buf).is_err() {
            return None;
        }

        let hdr = SlotHeader::parse(unsafe { &*(buf[0..HEADER_SIZE].as_ptr() as *const [u8; HEADER_SIZE]) });
        if hdr.is_valid(core::mem::size_of::<ModelConfig>()).is_err() {
            return None;
        }

        let mut name = [0u8; 10];
        name.copy_from_slice(&buf[HEADER_SIZE..HEADER_SIZE + 10]);
        Some(name)
    }
}

/// Delete a model slot in O(1) by invalidating the magic bytes in the 8-byte header.
pub fn delete_model_slot(slot_idx: usize) -> bool {
    #[cfg(test)]
    {
        let _ = slot_idx;
        true
    }
    #[cfg(not(test))]
    {
        let Some(addr) = model_slot_addr(slot_idx) else {
            return false;
        };
        // Overwrite magic with 0xFFFF to invalidate slot
        let invalid_magic = [0xFFu8, 0xFFu8];
        EepromI2c::write(addr, &invalid_magic).is_ok()
    }
}

/// Initialize EEPROM peripheral and load radio storage into memory.
#[inline(never)]
pub fn load_storage_into(storage: &mut RadioStorage) {
    #[cfg(test)]
    {
        storage.init_default();
        return;
    }

    #[cfg(not(test))]
    {
        EepromI2c::init();

        // 1. Try loading RadioConfig from Primary Slot A
        let mut radio_loaded = false;
        if load_slot(RADIO_SLOT_A_ADDR, &mut storage.radio).is_ok() {
            radio_loaded = true;
        } else if load_slot(RADIO_SLOT_B_ADDR, &mut storage.radio).is_ok() {
            // Primary Slot A failed, but Backup Slot B succeeded!
            radio_loaded = true;
            // Restore Slot A from Slot B
            let _ = save_slot(RADIO_SLOT_A_ADDR, &storage.radio);
        }

        if !radio_loaded {
            // Fresh / uninitialized EEPROM -> load factory defaults and save to Slot A and B
            storage.init_default();
            let _ = save_slot(RADIO_SLOT_A_ADDR, &storage.radio);
            let _ = save_slot(RADIO_SLOT_B_ADDR, &storage.radio);
            for idx in 0..NUM_MODELS {
                if let Some(addr) = model_slot_addr(idx) {
                    let _ = save_slot(addr, &storage.models[idx]);
                }
            }
            storage.sanitize();
            return;
        }

        // 2. Load all 20 models from EEPROM
        for idx in 0..NUM_MODELS {
            if let Some(addr) = model_slot_addr(idx) {
                if load_slot(addr, &mut storage.models[idx]).is_err() {
                    storage.models[idx] = ModelConfig::default_for_index(idx);
                }
            }
        }

        storage.sanitize();
    }
}

#[allow(dead_code)]
pub fn load_storage() -> RadioStorage {
    let mut storage = RadioStorage::empty();
    load_storage_into(&mut storage);
    storage
}

/// Save the currently active model to its dedicated EEPROM slot.
pub fn save_active_model(storage: &RadioStorage) -> bool {
    #[cfg(test)]
    {
        let _ = storage;
        return true;
    }
    #[cfg(not(test))]
    {
        let idx = (storage.radio.active_model as usize).min(NUM_MODELS - 1);
        if let Some(addr) = model_slot_addr(idx) {
            save_slot(addr, &storage.models[idx]).is_ok()
        } else {
            false
        }
    }
}

/// Save RadioConfig using A/B ping-pong save to ensure power-loss safety.
pub fn save_radio_config(storage: &RadioStorage) -> bool {
    #[cfg(test)]
    {
        let _ = storage;
        return true;
    }
    #[cfg(not(test))]
    {
        // Save to Backup Slot B first, then to Primary Slot A
        let ok_b = save_slot(RADIO_SLOT_B_ADDR, &storage.radio).is_ok();
        let ok_a = save_slot(RADIO_SLOT_A_ADDR, &storage.radio).is_ok();
        ok_a || ok_b
    }
}

pub fn save_storage(storage: &RadioStorage) -> bool {
    let mut ok = save_radio_config(storage);
    ok = ok && save_active_model(storage);
    ok
}

pub fn load_config() -> RadioConfig {
    let storage = get_storage();
    load_storage_into(storage);
    storage.radio
}

/// Read persisted receiver ID from active model if previously bound.
pub fn load_saved_rx_id() -> Option<u32> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        let storage = get_storage();
        let active = storage.active_model();
        if active.rx_id != 0 && active.rx_id != 0xFFFF_FFFF {
            Some(active.rx_id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adc_input_mode_detent() {
        assert_eq!(AdcInputMode::from_u8(4), AdcInputMode::Pot);
        assert_eq!(AdcInputMode::from_u8(5), AdcInputMode::PotDetent);
        assert_eq!(AdcInputMode::from_u8(6), AdcInputMode::InstantTrim);
        assert!(AdcInputMode::Pot.is_pot());
        assert!(AdcInputMode::PotDetent.is_pot());
        assert!(!AdcInputMode::InstantTrim.is_pot());
        assert!(!AdcInputMode::Pot.has_detent());
        assert!(AdcInputMode::PotDetent.has_detent());
    }

    #[test]
    fn test_pot_calib_and_radio_config_defaults() {
        let radio = RadioConfig::default_factory();
        assert_eq!(radio.aux_pots.len(), 10);
        assert_eq!(radio.adc_modes.len(), 10);
        for i in 0..10 {
            assert_eq!(radio.aux_pots[i].min, crate::adc::ADC_MIN);
            assert_eq!(radio.aux_pots[i].center, crate::adc::ADC_CENTER);
            assert_eq!(radio.aux_pots[i].max, crate::adc::ADC_MAX);
            assert_eq!(radio.adc_modes[i], 0);
        }

        let pot = PotCalib::new(100, 2000, 3900);
        assert_eq!(pot.min, 100);
        assert_eq!(pot.center, 2000);
        assert_eq!(pot.max, 3900);
    }

    #[test]
    fn test_model_config_size_and_rf_power() {
        assert_eq!(core::mem::size_of::<ModelConfig>(), 128);
        assert_eq!(core::mem::size_of::<RadioConfig>(), 140);

        let default_m = ModelConfig::default_for_index(0);
        assert_eq!(default_m.rf_power, 0);

        let mut storage = RadioStorage::default_factory();
        storage.models[0].rf_power = 99;
        storage.sanitize();
        assert_eq!(storage.models[0].rf_power, 0);
    }

    #[test]
    fn test_telem_slots_sanitize() {
        let mut storage = RadioStorage::default_factory();
        storage.radio.telem_slots = [0, 11, 12, 13];
        storage.sanitize();
        assert_eq!(storage.radio.telem_slots, [0, 11, 12, 12]);
    }

    #[test]
    fn test_trainer_config_helpers_and_sanitize() {
        let mut radio = RadioConfig::default_factory();
        assert_eq!(radio.trainer_config, 0);
        assert!(!radio.is_ppm_out_enabled());
        assert!(!radio.is_ppm_in_enabled());

        radio.set_ppm_out_enabled(true);
        assert!(radio.is_ppm_out_enabled());
        assert!(!radio.is_ppm_in_enabled());
        assert_eq!(radio.trainer_config, 1);

        radio.set_ppm_in_enabled(true);
        assert!(radio.is_ppm_out_enabled());
        assert!(radio.is_ppm_in_enabled());
        assert_eq!(radio.trainer_config, 3);

        radio.set_ppm_out_enabled(false);
        assert!(!radio.is_ppm_out_enabled());
        assert!(radio.is_ppm_in_enabled());
        assert_eq!(radio.trainer_config, 2);

        let mut storage = RadioStorage::default_factory();
        storage.radio.trainer_config = 99;
        storage.sanitize();
        assert_eq!(storage.radio.trainer_config, 0);
    }

    #[test]
    fn test_crc16_and_slot_header() {
        let test_payload = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let crc = crc16(&test_payload);
        assert_ne!(crc, 0);

        let hdr = SlotHeader {
            magic: SLOT_MAGIC,
            version: SCHEMA_VERSION,
            flags: 0,
            len: test_payload.len() as u16,
            crc16: crc,
        };
        let bytes = hdr.to_bytes();
        let parsed = SlotHeader::parse(&bytes);
        assert_eq!(parsed, hdr);
        assert!(parsed.is_valid(test_payload.len()).is_ok());

        // Corrupted length check
        assert_eq!(
            parsed.is_valid(test_payload.len() + 1),
            Err(StorageError::LengthMismatch(test_payload.len() as u16))
        );

        // Invalid magic check
        let mut corrupted_bytes = bytes;
        corrupted_bytes[0] = 0xFF;
        let corrupted_hdr = SlotHeader::parse(&corrupted_bytes);
        assert_eq!(corrupted_hdr.is_valid(test_payload.len()), Err(StorageError::EmptySlot));
    }

    #[test]
    fn test_eeprom_slot_addressing_and_bounds() {
        assert_eq!(RADIO_SLOT_A_ADDR, 0x0000);
        assert_eq!(RADIO_SLOT_B_ADDR, 0x0100);
        assert_eq!(MODEL_BASE_ADDR, 0x0200);

        // Verify model slot addresses are strictly 64-byte page-aligned
        for i in 0..NUM_MODELS {
            let addr = model_slot_addr(i).expect("Valid model slot");
            assert_eq!(addr % (PAGE_SIZE as u16), 0);
            assert!(addr >= MODEL_BASE_ADDR);
            assert!((addr as usize + MODEL_SLOT_SIZE) <= EEPROM_TOTAL_SIZE);
        }
        assert_eq!(model_slot_addr(NUM_MODELS), None);
    }
}
