//! Key matrix scanning and DFU bootloader detection.

use core::ptr;
use crate::chip::{self, McuProfile};

const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;

const GPIOC_MODER: *mut u32 = 0x4800_0800 as *mut u32;
const GPIOC_BSRR: *mut u32 = 0x4800_0818 as *mut u32;

const GPIOD_MODER: *mut u32 = 0x4800_0C00 as *mut u32;
const GPIOD_PUPDR: *mut u32 = 0x4800_0C0C as *mut u32;
const GPIOD_IDR: *const u32 = 0x4800_0C10 as *const u32;

const GPIOF_MODER: *mut u32 = 0x4800_1400 as *mut u32;
const GPIOF_PUPDR: *mut u32 = 0x4800_140C as *mut u32;
const GPIOF_IDR: *const u32 = 0x4800_1410 as *const u32;

#[inline(always)]
fn delay_cycles(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}

/// Initialize GPIO clocks and pins for keys and trims.
pub fn init_keys() {
    unsafe {
        // Enable GPIOC, GPIOD, GPIOF clocks (bits 19, 20, 22)
        let ahbenr = ptr::read_volatile(RCC_AHBENR);
        ptr::write_volatile(RCC_AHBENR, ahbenr | (1 << 19) | (1 << 20) | (1 << 22));

        // Configure PC6, PC7, PC8 as outputs (MODER = 01)
        let c_moder = ptr::read_volatile(GPIOC_MODER);
        ptr::write_volatile(GPIOC_MODER, (c_moder & !(0x3F << 12)) | (0x15 << 12));

        // Set PC6, PC7, PC8 initially HIGH
        ptr::write_volatile(GPIOC_BSRR, (1 << 6) | (1 << 7) | (1 << 8));

        // Configure PD12..PD15 as inputs (MODER = 00) with pull-ups (PUPDR = 01)
        let d_moder = ptr::read_volatile(GPIOD_MODER);
        ptr::write_volatile(GPIOD_MODER, d_moder & !(0xFF << 24));
        let d_pupdr = ptr::read_volatile(GPIOD_PUPDR);
        ptr::write_volatile(GPIOD_PUPDR, (d_pupdr & !(0xFF << 24)) | (0x55 << 24));

        // Configure PF2 (Bind button) as input with pull-up (MODER = 00, PUPDR = 01)
        let f_moder = ptr::read_volatile(GPIOF_MODER);
        ptr::write_volatile(GPIOF_MODER, f_moder & !(3 << 4));
        let f_pupdr = ptr::read_volatile(GPIOF_PUPDR);
        ptr::write_volatile(GPIOF_PUPDR, (f_pupdr & !(3 << 4)) | (1 << 4));
    }
}

/// Scan key matrix and return a 16-bit bitfield of pressed buttons:
/// - bit 0:  Col 0 Line 0 (PC6, PD12) -> Roll R
/// - bit 1:  Col 0 Line 1 (PC6, PD13) -> Roll L (Inward trim for Roll)
/// - bit 2:  Col 0 Line 2 (PC6, PD14) -> Pitch U
/// - bit 3:  Col 0 Line 3 (PC6, PD15) -> Pitch D
/// - bit 4:  Col 1 Line 0 (PC7, PD12) -> Throttle U
/// - bit 5:  Col 1 Line 1 (PC7, PD13) -> Throttle D
/// - bit 6:  Col 1 Line 2 (PC7, PD14) -> Yaw R (Inward trim for Yaw)
/// - bit 7:  Col 1 Line 3 (PC7, PD15) -> Yaw L
/// - bit 8:  Col 2 Line 0 (PC8, PD12) -> Down
/// - bit 9:  Col 2 Line 1 (PC8, PD13) -> Up
/// - bit 10: Col 2 Line 2 (PC8, PD14) -> OK / Enter
/// - bit 11: Col 2 Line 3 (PC8, PD15) -> Cancel / Exit
/// - bit 12: Dedicated Bind key (PF2)
pub fn scan_keys() -> u16 {
    let mut result = 0u16;

    unsafe {
        let cols = [6, 7, 8];
        for (col_idx, &col) in cols.iter().enumerate() {
            // Drive active column LOW
            ptr::write_volatile(GPIOC_BSRR, 1 << (col + 16));
            delay_cycles(150); // allow line capacitance to discharge

            // Read lines PD12..PD15 (active LOW)
            let idr = ptr::read_volatile(GPIOD_IDR);
            let lines = ((idr >> 12) & 0x0F) as u8;
            let pressed = (!lines) & 0x0F;

            result |= (pressed as u16) << (col_idx * 4);

            // Restore column to HIGH
            ptr::write_volatile(GPIOC_BSRR, 1 << col);
            delay_cycles(100);
        }

        // Check Bind button (PF2, active LOW)
        let fidr = ptr::read_volatile(GPIOF_IDR);
        if (fidr & (1 << 2)) == 0 {
            result |= 1 << 12; // Bind pressed
        }
    }

    result
}

/// Check if the DFU bootloader key combination is held:
/// Both horizontal trims pushed inward towards power switch:
/// Roll Left (bit 1) + Yaw Right (bit 6).
pub fn is_dfu_requested(keys: u16) -> bool {
    let rh_inward = (keys & (1 << 1)) != 0; // Roll Left
    let lh_inward = (keys & (1 << 6)) != 0; // Yaw Right

    rh_inward && lh_inward
}

/// Power-on boot check for DFU entry.
pub fn check_dfu_entry(profile: &McuProfile) {
    init_keys();

    // Wait ~20ms for power rails and switch contacts to settle at power-on
    delay_cycles(40_000);

    let mut match_count = 0;
    for _ in 0..5 {
        let keys = scan_keys();
        if is_dfu_requested(keys) {
            match_count += 1;
        }
        delay_cycles(2_000);
    }

    if match_count >= 3 {
        chip::enter_dfu_bootloader(profile);
    }
}
