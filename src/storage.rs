//! Flash storage for non-volatile configuration (RX ID, stick & pot calibrations).
//!
//! Stored in the last 2KB sector of Flash on STM32F072VB (0x0801_F800).

pub const FLASH_STORAGE_ADDR: usize = 0x0801_F800;
pub const FLASH_MAGIC: u32 = 0x4653_4B59; // "FSKY"
pub const CONFIG_VERSION: u32 = 2;

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

/// Persistent radio settings and calibration data (64 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct RadioConfig {
    pub magic: u32,
    pub version: u32,
    pub rx_id: u32,
    pub sticks: [ChannelCalib; 4], // 0: Roll, 1: Pitch, 2: Throttle, 3: Yaw (32 bytes)
    pub pots: [ChannelCalib; 2],   // 0: VRA, 1: VRB (16 bytes)
    pub throttle_trim: u8,         // 0: Disabled (safety lock), 1: Enabled
    pub audio_enabled: u8,         // 0: Muted, 1: Enabled
    pub backlight_timeout: u8,     // 0: Always On, 1: 15s, 2: 30s, 3: 60s
    pub backlight_brightness: u8,  // 1..10 (10%..100%, default 10)
}

impl RadioConfig {
    pub const fn default_factory() -> Self {
        Self {
            magic: FLASH_MAGIC,
            version: CONFIG_VERSION,
            rx_id: 0,
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
            throttle_trim: 0,        // Default disabled for FC arming safety
            audio_enabled: 1,        // Default audible buzzer enabled
            backlight_timeout: 0,    // Default Always On
            backlight_brightness: 10, // Default 100%
        }
    }
}

/// Load saved configuration from Flash, or return default factory settings if unprogrammed.
pub fn load_config() -> RadioConfig {
    unsafe {
        let magic = core::ptr::read_volatile(FLASH_STORAGE_ADDR as *const u32);
        if magic != FLASH_MAGIC {
            return RadioConfig::default_factory();
        }

        let word1 = core::ptr::read_volatile((FLASH_STORAGE_ADDR + 4) as *const u32);
        if word1 == CONFIG_VERSION {
            // Full RadioConfig v2 is saved
            let mut cfg = RadioConfig::default_factory();
            let src = FLASH_STORAGE_ADDR as *const u32;
            let dst = &mut cfg as *mut RadioConfig as *mut u32;
            let word_count = core::mem::size_of::<RadioConfig>() / 4;
            for i in 0..word_count {
                *dst.add(i) = core::ptr::read_volatile(src.add(i));
            }
            cfg
        } else if word1 == 1 {
            // Version 1 had sticks and pots but no settings tail (15 words = 60 bytes)
            let mut cfg = RadioConfig::default_factory();
            let src = FLASH_STORAGE_ADDR as *const u32;
            let dst = &mut cfg as *mut RadioConfig as *mut u32;
            for i in 0..15 {
                *dst.add(i) = core::ptr::read_volatile(src.add(i));
            }
            cfg.version = CONFIG_VERSION;
            cfg
        } else {
            // Legacy layout: [magic, rx_id]
            let mut cfg = RadioConfig::default_factory();
            if word1 != 0 && word1 != 0xFFFF_FFFF {
                cfg.rx_id = word1;
            }
            cfg
        }
    }
}

/// Save radio configuration to Flash.
pub fn save_config(config: &RadioConfig) {
    unsafe {
        // Unlock flash
        core::ptr::write_volatile(FLASH_KEYR, 0x4567_0123);
        core::ptr::write_volatile(FLASH_KEYR, 0xCDEF_89AB);

        while (core::ptr::read_volatile(FLASH_SR) & 1) != 0 {}

        // Erase page 0x0801_F800
        core::ptr::write_volatile(FLASH_CR, 1 << 1); // PER
        core::ptr::write_volatile(FLASH_AR, FLASH_STORAGE_ADDR as u32);
        core::ptr::write_volatile(FLASH_CR, (1 << 1) | (1 << 6)); // PER | STRT

        while (core::ptr::read_volatile(FLASH_SR) & 1) != 0 {}
        core::ptr::write_volatile(FLASH_CR, 0);

        // Program halfwords
        core::ptr::write_volatile(FLASH_CR, 1 << 0); // PG

        let halfword_count = core::mem::size_of::<RadioConfig>() / 2;
        let src = config as *const RadioConfig as *const u16;
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

/// Convenience helper to update just the RX ID while preserving current calibration.
#[allow(dead_code)]
pub fn save_rx_id(rx_id: u32) {
    let mut cfg = load_config();
    cfg.rx_id = rx_id;
    save_config(&cfg);
}
