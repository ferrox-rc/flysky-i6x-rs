//! CRSF / ExpressLRS protocol subsystem for FlySky FS-i6X.
//!
//! Handles USART2 hardware setup on PD5/PA15, PC13 module power management,
//! periodic channel packet framing, and incoming telemetry processing.

#![allow(dead_code)]
#![allow(static_mut_refs)]

pub mod protocol;
pub mod uart;

use protocol::{build_channels_frame, parse_telemetry_frame, CrsfTelemetry, CRSF_RC_FRAME_SIZE};

static mut TELEMETRY: CrsfTelemetry = CrsfTelemetry::new();
static mut CRSF_ENABLED: bool = false;
static mut CURRENT_BAUD: u8 = 0xFF;
static mut LAST_TX_MS: u32 = 0;
static mut RX_BUF: [u8; 64] = [0; 64];
static mut RX_LEN: usize = 0;

/// Initialize CRSF subsystem hardware pins.
pub fn init() {
    uart::init();
}

/// Enable or disable CRSF protocol, power external module, and configure baud rate.
pub fn set_enabled(enabled: bool, baud_idx: u8) {
    unsafe {
        if enabled != CRSF_ENABLED || (enabled && baud_idx != CURRENT_BAUD) {
            CRSF_ENABLED = enabled;
            CURRENT_BAUD = baud_idx;

            uart::set_uart_enabled(enabled, baud_idx);
            uart::set_module_power(enabled);

            if !enabled {
                TELEMETRY = CrsfTelemetry::new();
                RX_LEN = 0;
            }
        }
    }
}

/// Returns true if CRSF mode is currently active.
pub fn is_enabled() -> bool {
    unsafe { CRSF_ENABLED }
}

/// Transmit RC channels packet to external module if interval elapsed (~100 Hz / 10 ms).
pub fn update_channels(now_ms: u32, channels: &[u16; 14]) {
    unsafe {
        if !CRSF_ENABLED {
            return;
        }

        // Pacing: 10 ms (100 Hz update rate)
        if now_ms.wrapping_sub(LAST_TX_MS) >= 10 {
            LAST_TX_MS = now_ms;
            let mut frame = [0u8; CRSF_RC_FRAME_SIZE];
            build_channels_frame(channels, &mut frame);
            uart::write_bytes(&frame);
        }
    }
}

/// Poll USART2 for incoming telemetry bytes from external module.
pub fn poll_telemetry(now_ms: u32) {
    unsafe {
        if !CRSF_ENABLED {
            return;
        }

        // Drain available bytes from USART2 RX
        let mut count = 0;
        while let Some(b) = uart::read_byte() {
            if RX_LEN == 0 {
                // Look for frame start: valid destination addresses (0xEE, 0xEA, 0xC8, etc.)
                if b == protocol::CRSF_ADDRESS_RADIO_TRANSMITTER
                    || b == protocol::CRSF_ADDRESS_CRSF_TRANSMITTER
                    || b == protocol::CRSF_ADDRESS_FLIGHT_CONTROLLER
                {
                    RX_BUF[0] = b;
                    RX_LEN = 1;
                }
            } else if RX_LEN == 1 {
                // Length byte: frame size is length + 2 (addr + len)
                let frame_len = b as usize;
                if frame_len >= 2 && (frame_len + 2) <= RX_BUF.len() {
                    RX_BUF[1] = b;
                    RX_LEN = 2;
                } else {
                    RX_LEN = 0; // Invalid length, reset parser
                }
            } else {
                let total_expected = (RX_BUF[1] as usize) + 2;
                if RX_LEN < total_expected {
                    RX_BUF[RX_LEN] = b;
                    RX_LEN += 1;

                    if RX_LEN == total_expected {
                        // Full frame received! Parse telemetry
                        let _ = parse_telemetry_frame(&RX_BUF[..RX_LEN], &mut TELEMETRY, now_ms);
                        RX_LEN = 0;
                    }
                } else {
                    RX_LEN = 0;
                }
            }

            count += 1;
            if count > 64 {
                break;
            }
        }

        // Connection timeout: if no telemetry received for 1000ms, mark disconnected
        if TELEMETRY.connected && now_ms.wrapping_sub(TELEMETRY.last_telemetry_ms) > 1000 {
            TELEMETRY.connected = false;
        }
    }
}

/// Get latest decoded CRSF telemetry data.
pub fn get_telemetry() -> CrsfTelemetry {
    unsafe { TELEMETRY }
}
