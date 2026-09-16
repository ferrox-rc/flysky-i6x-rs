//! Hardware USART2 driver and PC13 power switch control for CRSF / ExpressLRS.
//!
//! Pin mapping:
//! - PD5:  USART2_TX (AF0, Push-Pull, High Speed)
//! - PA15: USART2_RX (AF1, Pull-Up, High Speed)
//! - PC13: Module Power Switch (GPIO Output, Push-Pull, High = Power ON)

#![allow(dead_code)]

use core::ptr;

// RCC registers
const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;

// GPIOA registers (Base: 0x4800_0000)
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_OSPEEDR: *mut u32 = 0x4800_0008 as *mut u32;
const GPIOA_PUPDR: *mut u32 = 0x4800_000C as *mut u32;
const GPIOA_AFRH: *mut u32 = 0x4800_0024 as *mut u32;

// GPIOC registers (Base: 0x4800_0800)
const GPIOC_MODER: *mut u32 = 0x4800_0800 as *mut u32;
const GPIOC_BSRR: *mut u32 = 0x4800_0818 as *mut u32;

// GPIOD registers (Base: 0x4800_0C00)
const GPIOD_MODER: *mut u32 = 0x4800_0C00 as *mut u32;
const GPIOD_OSPEEDR: *mut u32 = 0x4800_0C08 as *mut u32;
const GPIOD_PUPDR: *mut u32 = 0x4800_0C0C as *mut u32;
const GPIOD_AFRL: *mut u32 = 0x4800_0C20 as *mut u32;

// USART2 registers (Base: 0x4000_4400)
const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_CR2: *mut u32 = 0x4000_4404 as *mut u32;
const USART2_CR3: *mut u32 = 0x4000_4408 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *mut u32 = 0x4000_441C as *mut u32;
const USART2_ICR: *mut u32 = 0x4000_4420 as *mut u32;
const USART2_RDR: *mut u32 = 0x4000_4424 as *mut u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;

// Status register flags
const USART_ISR_TXE: u32 = 1 << 7;
const USART_ISR_TC: u32 = 1 << 6;
const USART_ISR_RXNE: u32 = 1 << 5;
const USART_ISR_ORE: u32 = 1 << 3;

/// Initialize GPIO pins: PC13 as power switch (initially OFF), PD5 and PA15 peripheral clocks.
pub fn init() {
    unsafe {
        // Enable GPIOA (bit 17), GPIOC (bit 19), GPIOD (bit 20) clocks
        let ahb = ptr::read_volatile(RCC_AHBENR);
        ptr::write_volatile(RCC_AHBENR, ahb | (1 << 17) | (1 << 19) | (1 << 20));

        // Configure PC13 as output (MODER 01)
        let c_moder = ptr::read_volatile(GPIOC_MODER);
        ptr::write_volatile(GPIOC_MODER, (c_moder & !(3 << 26)) | (1 << 26));

        // Ensure PC13 is LOW (Module power OFF by default)
        ptr::write_volatile(GPIOC_BSRR, 1 << (13 + 16));
    }
}

/// Set external module power state via PC13
pub fn set_module_power(power_on: bool) {
    unsafe {
        if power_on {
            ptr::write_volatile(GPIOC_BSRR, 1 << 13); // High
        } else {
            ptr::write_volatile(GPIOC_BSRR, 1 << (13 + 16)); // Low
        }
    }
}

/// Baud rate divisors for 48.000 MHz clock tree
pub fn get_brr_for_baud(baud_idx: u8) -> u32 {
    match baud_idx {
        0 => 114, // 420,000 bps (48,000,000 / 420,000 = 114.285)
        1 => 115, // 416,666 bps (48,000,000 / 416,666.67 = 115.20)
        2 => 417, // 115,200 bps (48,000,000 / 115,200 = 416.66)
        3 => 52,  // 921,600 bps (48,000,000 / 921,600 = 52.08)
        _ => 114,
    }
}

/// Configure and enable/disable USART2
pub fn set_uart_enabled(enabled: bool, baud_idx: u8) {
    unsafe {
        if enabled {
            // 1. Enable USART2 clock in RCC_APB1ENR (bit 17)
            let apb1 = ptr::read_volatile(RCC_APB1ENR);
            ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 17));

            // 2. Configure PD5 as AF0 (USART2_TX): MODER=10, AFRL bit 20..23 = 0000
            let d_moder = ptr::read_volatile(GPIOD_MODER);
            ptr::write_volatile(GPIOD_MODER, (d_moder & !(3 << 10)) | (2 << 10));
            let d_ospeedr = ptr::read_volatile(GPIOD_OSPEEDR);
            ptr::write_volatile(GPIOD_OSPEEDR, d_ospeedr | (3 << 10)); // High speed
            let d_afrl = ptr::read_volatile(GPIOD_AFRL);
            ptr::write_volatile(GPIOD_AFRL, d_afrl & !(0xF << 20)); // AF0

            // 3. Configure PA15 as AF1 (USART2_RX): MODER=10, AFRH bit 28..31 = 0001
            let a_moder = ptr::read_volatile(GPIOA_MODER);
            ptr::write_volatile(GPIOA_MODER, (a_moder & !(3 << 30)) | (2 << 30));
            let a_pupdr = ptr::read_volatile(GPIOA_PUPDR);
            ptr::write_volatile(GPIOA_PUPDR, (a_pupdr & !(3 << 30)) | (1 << 30)); // Pull-up
            let a_afrh = ptr::read_volatile(GPIOA_AFRH);
            ptr::write_volatile(GPIOA_AFRH, (a_afrh & !(0xF << 28)) | (1 << 28)); // AF1

            // 4. Reset USART2 registers
            ptr::write_volatile(USART2_CR1, 0);
            ptr::write_volatile(USART2_CR2, 0);
            ptr::write_volatile(USART2_CR3, 0);

            // 5. Set Baud rate divisor
            let brr = get_brr_for_baud(baud_idx);
            ptr::write_volatile(USART2_BRR, brr);

            // Clear any pending error flags
            ptr::write_volatile(USART2_ICR, 0xFFFF_FFFF);

            // 6. Enable UE (bit 0), TE (bit 3), RE (bit 2)
            ptr::write_volatile(USART2_CR1, (1 << 0) | (1 << 3) | (1 << 2));
        } else {
            // Disable UE
            ptr::write_volatile(USART2_CR1, 0);

            // Disable USART2 clock in RCC_APB1ENR (bit 17)
            let apb1 = ptr::read_volatile(RCC_APB1ENR);
            ptr::write_volatile(RCC_APB1ENR, apb1 & !(1 << 17));

            // Set PD5 and PA15 back to inputs to float pins
            let d_moder = ptr::read_volatile(GPIOD_MODER);
            ptr::write_volatile(GPIOD_MODER, d_moder & !(3 << 10));
            let a_moder = ptr::read_volatile(GPIOA_MODER);
            ptr::write_volatile(GPIOA_MODER, a_moder & !(3 << 30));
        }
    }
}

/// Transmit a buffer over USART2 (non-blocking if space available, bounded timeout)
pub fn write_bytes(bytes: &[u8]) -> usize {
    let mut sent = 0;
    unsafe {
        for &b in bytes {
            let mut timeout = 2500u32;
            while (ptr::read_volatile(USART2_ISR) & USART_ISR_TXE) == 0 {
                timeout -= 1;
                if timeout == 0 {
                    return sent;
                }
            }
            ptr::write_volatile(USART2_TDR, b as u32);
            sent += 1;
        }
    }
    sent
}

/// Read a byte from USART2 RX if available
pub fn read_byte() -> Option<u8> {
    unsafe {
        let isr = ptr::read_volatile(USART2_ISR);
        // Clear overrun error if present
        if (isr & USART_ISR_ORE) != 0 {
            ptr::write_volatile(USART2_ICR, USART_ISR_ORE);
        }

        if (isr & USART_ISR_RXNE) != 0 {
            let data = (ptr::read_volatile(USART2_RDR) & 0xFF) as u8;
            Some(data)
        } else {
            None
        }
    }
}
