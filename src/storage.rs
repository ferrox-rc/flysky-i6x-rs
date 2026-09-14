//! Flash storage for non-volatile configuration and 20-model memory system.
//!
//! - Page 62: 0x0801_F000 (2048 bytes)
//! - Page 63: 0x0801_F800 (2048 bytes)
//!
//! Total available: 4096 bytes. Total used: 2688 bytes.

pub const FLASH_STORAGE_ADDR: usize = 0x0801_F000;
pub const FLASH_LEGACY_ADDR: usize = 0x0801_F800;
pub const FLASH_MAGIC: u32 = 0x4653_4B59; // "FSKY"
pub const CONFIG_VERSION: u32 = 4;
pub const NUM_MODELS: usize = 20;

const FLASH_KEYR: *mut u32 = 0x4002_2004 as *mut u32;
const FLASH_SR: *mut u32 = 0x4002_200C as *mut u32;
const FLASH_CR: *mut u32 = 0x4002_2010 as *mut u32;
const FLASH_AR: *mut u32 = 0x4002_2014 as *mut u32;

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

/// Persistent system/radio-level configuration (exactly 128 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RadioConfig {
    pub magic: u32,                // 0..4 ("FSKY")
    pub version: u32,              // 4..8 (3)
    pub active_model: u8,          // 8 (0..19, active model index)
    pub throttle_trim: u8,         // 9 (0: Off/Lock, 1: Idle T-Trim, 2: Linear)
    pub audio_enabled: u8,         // 10 (0: Muted, 1: Enabled)
    pub backlight_timeout: u8,     // 11 (0: Always On, 1: 15s, 2: 30s, 3: 60s)
    pub backlight_brightness: u8,  // 12 (1..10)
    pub vbat_warn_deci: u8,        // 13 (40..50 = 4.0V..5.0V, default 44 = 4.4V)
    pub _pad0: [u8; 2],            // 14..16 (align sticks to 16)
    pub sticks: [ChannelCalib; 4], // 16..48 (32 bytes: Roll, Pitch, Throttle, Yaw)
    pub pots: [ChannelCalib; 2],   // 48..64 (16 bytes: VRA, VRB)
    pub _reserved: [u8; 64],       // 64..128
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
            _pad0: [0; 2],
            sticks: [
                ChannelCalib::new(2048 - 1670, 2048, 2048 + 1670), // Roll (Horizontal)
                ChannelCalib::new(2048 - 1580, 2048, 2048 + 1580), // Pitch (Vertical)
                ChannelCalib::new(2048 - 1580, 2048, 2048 + 1580), // Throttle (Vertical)
                ChannelCalib::new(2048 - 1670, 2048, 2048 + 1670), // Yaw (Horizontal)
            ],
            pots: [
                ChannelCalib::new(2048 - 1950, 2048, 2048 + 1950), // VRA
                ChannelCalib::new(2048 - 1950, 2048, 2048 + 1950), // VRB
            ],
            _reserved: [0; 64],
        }
    }
}

/// A single freeform mix rule in the matrix mixer (6 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MixLine {
    pub target_ch: u8,   // 0: Disabled, 1..14: Target Channel CH1..CH14
    pub source: u8,      // 0: None, 1: Roll, 2: Pitch, 3: Thr, 4: Yaw, 5: VRA, 6: VRB, 7: SA, 8: SB, 9: SC, 10: SD, 11: MAX, 12..25: CH1..CH14
    pub weight: i8,      // -100% .. +100%
    pub offset: i8,      // -100% .. +100%
    pub switch: u8,      // 0: Always On, 1: SA_UP, 2: SA_DN, 3: SB_UP, 4: SB_MID, 5: SB_DN, 6: SC_UP, 7: SC_MID, 8: SC_DN, 9: SD_UP, 10: SD_DN
    pub mode: u8,        // 0: Add (+), 1: Multiply (*), 2: Replace (:=)
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
    pub name: [u8; 10],            // 0..10: 10-char ASCII name
    pub model_type: u8,            // 10: 0: Airplane, 1: Glider, 2: Heli, 3: Quad
    pub _pad0: u8,                 // 11: align rx_id to 4 bytes
    pub rx_id: u32,                // 12..16: Model Match bound receiver ID
    pub trims: [i8; 4],            // 16..20: Roll, Pitch, Throttle, Yaw (-25..+25)
    pub channel_reverse: u16,      // 20..22: Bitmask for CH1..CH14 inversion
    pub dr_switch: u8,             // 22: 0: None, 1: SA, 2: SB, 3: SC, 4: SD
    pub thr_curve_pts: u8,         // 23: 5 or 9 points mode
    pub thr_curve_smooth: u8,      // 24: 0: Linear, 1: Spline / Smooth
    pub thr_curve: [u8; 9],        // 25..34: Points 1..9 (0..100%)
    pub dr_high: [u8; 3],          // 34..37: High rate (AIL, ELE, RUD: 50..100%)
    pub dr_low: [u8; 3],           // 37..40: Low rate (AIL, ELE, RUD: 30..100%)
    pub expo_high: [i8; 3],        // 40..43: High expo (-100..+100%)
    pub expo_low: [i8; 3],         // 43..46: Low expo (-100..+100%)
    pub timer_secs: u16,           // 46..48: Countdown timer in seconds (e.g. 300 = 5 min)
    pub timer_source: u8,          // 48: 0: Off, 1: Thr > 5%, 2: SA, 3: SB, 4: SC, 5: SD
    pub protocol_subtype: u8,      // 49: 0: PWM, 1: PPM, 2: i-BUS, 3: S.BUS
    pub failsafe_thr: u16,         // 50..52: Failsafe throttle pulse in µs (e.g. 1000)
    pub aux_channels: [u8; 10],    // 52..62: Source for CH5..CH14 (0: None, 1..4: AETR, 5..6: VRA/VRB, 7..10: SA..SD)
    pub wing_tail_mix: u8,         // 62: 0: Normal, 1: Elevon/Delta, 2: V-Tail, 3: Flaperon
    pub template_diff: i8,         // 63: Differential / mix ratio (-100..+100)
    pub mixes: [MixLine; 8],       // 64..112: 8 freeform mix rules (8 * 6 = 48 bytes)
    pub failsafe_mode: u8,         // 112: 0: Hold last, 1: Custom pulses
    pub failsafe_timeout: u8,      // 113: 10..50 (1.0s..5.0s)
    pub _reserved: [u8; 14],       // 114..128: 14 reserved bytes
}

impl ModelConfig {
    pub const fn default_for_index(idx: usize) -> Self {
        let num = (idx + 1) as u8;
        let digit1 = b'0' + (num / 10);
        let digit2 = b'0' + (num % 10);
        Self {
            name: [b'M', b'O', b'D', b'E', b'L', b' ', digit1, digit2, b' ', b' '],
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
            protocol_subtype: 0,
            failsafe_thr: 1000,
            // Defaults for CH5..CH14: CH5=SA(7), CH6=SB(8), CH7=VRA(5), CH8=VRB(6), CH9=SC(9), CH10=SD(10), CH11..14=None(0)
            aux_channels: [7, 8, 5, 6, 9, 10, 0, 0, 0, 0],
            wing_tail_mix: 0,
            template_diff: 0,
            mixes: [MixLine::disabled(); 8],
            failsafe_mode: 0,
            failsafe_timeout: 20,
            _reserved: [0; 14],
        }
    }
}

/// Complete Flash storage layout containing radio settings and 20 models (2,688 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct RadioStorage {
    pub radio: RadioConfig,
    pub models: [ModelConfig; NUM_MODELS],
}

impl RadioStorage {
    pub const fn default_factory() -> Self {
        let radio = RadioConfig::default_factory();
        let mut models = [ModelConfig::default_for_index(0); NUM_MODELS];
        let mut i = 1;
        while i < NUM_MODELS {
            models[i] = ModelConfig::default_for_index(i);
            i += 1;
        }
        Self { radio, models }
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
    /// Prevents uninitialized Flash bytes or corrupted rates/mixes from impairing flight controls.
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

        for (idx, m) in self.models.iter_mut().enumerate() {
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
            if m.dr_switch > 4 {
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
            // If aux_channels are all 0 (uninitialized Flash from v0.9.x and earlier), restore defaults
            if m.aux_channels[0..6] == [0; 6] {
                m.aux_channels = [7, 8, 5, 6, 9, 10, 0, 0, 0, 0];
            }
            for src in m.aux_channels.iter_mut() {
                if *src > 25 {
                    *src = 0;
                }
            }
            for mix in m.mixes.iter_mut() {
                if mix.target_ch > 14 || mix.source > 25 || mix.mode > 2 || mix.switch > 10 {
                    *mix = MixLine::disabled();
                }
            }
            if m.thr_curve_pts != 5 && m.thr_curve_pts != 9 {
                m.thr_curve_pts = 5;
                m.thr_curve = [0, 25, 50, 75, 100, 0, 0, 0, 0];
            }
            if m.name[0] == 0 || m.name[0] == 0xFF {
                let num = (idx + 1) as u8;
                let digit1 = b'0' + (num / 10);
                let digit2 = b'0' + (num % 10);
                m.name = [b'M', b'O', b'D', b'E', b'L', b' ', digit1, digit2, b' ', b' '];
            }
        }
    }
}

// Compile-time size guarantees
const _: () = assert!(core::mem::size_of::<RadioConfig>() == 128);
const _: () = assert!(core::mem::size_of::<ModelConfig>() == 128);
const _: () = assert!(core::mem::size_of::<RadioStorage>() == 2688);

/// Load complete storage from Flash (with automatic migration from legacy v1/v2/v3).
pub fn load_storage() -> RadioStorage {
    unsafe {
        let magic = core::ptr::read_volatile(FLASH_STORAGE_ADDR as *const u32);
        let version = core::ptr::read_volatile((FLASH_STORAGE_ADDR + 4) as *const u32);

        if magic == FLASH_MAGIC && (version == CONFIG_VERSION || version == 3) {
            let mut storage = RadioStorage::default_factory();
            let src = FLASH_STORAGE_ADDR as *const u32;
            let dst = &mut storage as *mut RadioStorage as *mut u32;
            let word_count = core::mem::size_of::<RadioStorage>() / 4;
            for i in 0..word_count {
                *dst.add(i) = core::ptr::read_volatile(src.add(i));
            }
            storage.sanitize();
            if version == 3 {
                save_storage(&storage);
            }
            return storage;
        }

        // Check for legacy v1/v2 at FLASH_LEGACY_ADDR (0x0801_F800)
        let legacy_magic = core::ptr::read_volatile(FLASH_LEGACY_ADDR as *const u32);
        let legacy_ver = core::ptr::read_volatile((FLASH_LEGACY_ADDR + 4) as *const u32);
        if legacy_magic == FLASH_MAGIC && (legacy_ver == 1 || legacy_ver == 2) {
            let mut storage = RadioStorage::default_factory();
            let rx_id = core::ptr::read_volatile((FLASH_LEGACY_ADDR + 8) as *const u32);
            if rx_id != 0 && rx_id != 0xFFFF_FFFF {
                storage.models[0].rx_id = rx_id;
            }

            // Copy sticks (32 bytes)
            let src_sticks = (FLASH_LEGACY_ADDR + 12) as *const ChannelCalib;
            for i in 0..4 {
                storage.radio.sticks[i] = core::ptr::read_volatile(src_sticks.add(i));
            }
            // Copy pots (16 bytes)
            let src_pots = (FLASH_LEGACY_ADDR + 44) as *const ChannelCalib;
            for i in 0..2 {
                storage.radio.pots[i] = core::ptr::read_volatile(src_pots.add(i));
            }
            if legacy_ver == 2 {
                storage.radio.throttle_trim = core::ptr::read_volatile((FLASH_LEGACY_ADDR + 60) as *const u8);
                storage.radio.audio_enabled = core::ptr::read_volatile((FLASH_LEGACY_ADDR + 61) as *const u8);
                storage.radio.backlight_timeout = core::ptr::read_volatile((FLASH_LEGACY_ADDR + 62) as *const u8);
                storage.radio.backlight_brightness = core::ptr::read_volatile((FLASH_LEGACY_ADDR + 63) as *const u8);
            }

            // Immediately persist upgraded v3 storage to 0x0801_F000
            save_storage(&storage);
            return storage;
        }

        RadioStorage::default_factory()
    }
}

/// Save complete storage to Flash (Pages 62 and 63).
pub fn save_storage(storage: &RadioStorage) {
    unsafe {
        // Unlock flash
        core::ptr::write_volatile(FLASH_KEYR, 0x4567_0123);
        core::ptr::write_volatile(FLASH_KEYR, 0xCDEF_89AB);

        while (core::ptr::read_volatile(FLASH_SR) & 1) != 0 {}

        // Erase Page 62 (0x0801_F000)
        core::ptr::write_volatile(FLASH_CR, 1 << 1); // PER
        core::ptr::write_volatile(FLASH_AR, FLASH_STORAGE_ADDR as u32);
        core::ptr::write_volatile(FLASH_CR, (1 << 1) | (1 << 6)); // PER | STRT
        while (core::ptr::read_volatile(FLASH_SR) & 1) != 0 {}

        // Erase Page 63 (0x0801_F800)
        core::ptr::write_volatile(FLASH_AR, (FLASH_STORAGE_ADDR + 2048) as u32);
        core::ptr::write_volatile(FLASH_CR, (1 << 1) | (1 << 6)); // PER | STRT
        while (core::ptr::read_volatile(FLASH_SR) & 1) != 0 {}
        core::ptr::write_volatile(FLASH_CR, 0);

        // Program halfwords
        core::ptr::write_volatile(FLASH_CR, 1 << 0); // PG

        let halfword_count = core::mem::size_of::<RadioStorage>() / 2;
        let src = storage as *const RadioStorage as *const u16;
        let dst = FLASH_STORAGE_ADDR as *mut u16;

        for i in 0..halfword_count {
            let hw = *src.add(i);
            core::ptr::write_volatile(dst.add(i), hw);
            while (core::ptr::read_volatile(FLASH_SR) & 1) != 0 {}
        }

        // Lock flash
        core::ptr::write_volatile(FLASH_CR, 1 << 7);
    }
}

/// Convenience helper to load current RadioConfig.
pub fn load_config() -> RadioConfig {
    load_storage().radio
}

/// Convenience helper to save current RadioConfig while preserving models.
pub fn save_config(config: &RadioConfig) {
    let mut storage = load_storage();
    storage.radio = *config;
    save_storage(&storage);
}
