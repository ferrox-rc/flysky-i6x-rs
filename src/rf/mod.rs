//! RF subsystem module for the FlySky FS-i6X.
//!
//! Orchestrates the A7105 2.4 GHz transceiver, SPI1 driver, and AFHDS 2A stack
//! via TIM16 (3.85 ms framing) and EXTI2 (A7105 GIO2 WTR signal).

#![allow(static_mut_refs)]

pub mod a7105;
pub mod afhds2a;
pub mod spi;

use afhds2a::{Afhds2a, TelemetryData, NUM_CHANNELS};
use stm32f0xx_hal::pac::interrupt;

static mut RF_DRIVER: Option<Afhds2a> = None;
static mut PENDING_CHANNELS: [u16; NUM_CHANNELS] = [1500; NUM_CHANNELS];

/// Initialize RF subsystem: SPI1, A7105 transceiver, and AFHDS 2A stack.
/// Returns true if A7105 responded and passed silicon verification (0x9E).
pub fn init(tx_id: u32) -> bool {
    spi::init();

    let mut reset_ok = a7105::reset();

    a7105::init();

    let post_sig = a7105::read_reg(a7105::REG_PLL_II);
    if post_sig == 0x9E {
        reset_ok = true;
    }
    unsafe {
        a7105::LAST_CHIP_ID = post_sig;
    }

    unsafe {
        RF_DRIVER = Some(Afhds2a::new(tx_id));
    }

    // Start 3.85 ms (260 Hz) periodic packet framing timer
    spi::init_tim16(3850);

    reset_ok
}

/// Get the last read byte from A7105 register 0x10 during reset.
pub fn get_last_chip_id() -> u8 {
    unsafe { a7105::LAST_CHIP_ID }
}

static mut RF_SILENCED: bool = false;

/// Put RF frontend and A7105 into standby (silent running) during USB Joystick simulator mode.
pub fn set_silenced(silenced: bool) {
    cortex_m::interrupt::free(|_| unsafe {
        if RF_SILENCED != silenced {
            RF_SILENCED = silenced;
            if silenced {
                a7105::strobe(a7105::STROBE_STANDBY);
                spi::set_tx_rx_mode(spi::RF_MODE_OFF);
            }
        }
    });
}

/// Check if RF transmission is silenced.
#[allow(dead_code)]
pub fn is_silenced() -> bool {
    unsafe { RF_SILENCED }
}

/// Update channel outputs (CH1..CH14) in microseconds (1000..2000 µs).
pub fn set_channels(channels: &[u16; NUM_CHANNELS]) {
    cortex_m::interrupt::free(|_| {
        unsafe {
            PENDING_CHANNELS.copy_from_slice(channels);
        }
    });
}

/// Enter or exit binding mode.
pub fn set_bind_mode(enable: bool) {
    cortex_m::interrupt::free(|_| {
        if let Some(ref mut driver) = unsafe { RF_DRIVER.as_mut() } {
            driver.set_bind_mode(enable);
        }
    });
}

/// Dynamically update receiver ID in the RF driver (e.g. on model switch).
pub fn set_rx_id(rx_id: u32) {
    cortex_m::interrupt::free(|_| {
        if let Some(ref mut driver) = unsafe { RF_DRIVER.as_mut() } {
            driver.set_rx_id(rx_id);
        }
    });
}

/// Query whether binding has completed.
pub fn is_bound() -> bool {
    cortex_m::interrupt::free(|_| {
        unsafe {
            RF_DRIVER
                .as_ref()
                .map(|d| d.bind_done || (d.rx_id != 0 && d.rx_id != 0xFFFF_FFFF))
                .unwrap_or(false)
        }
    })
}

/// Query whether the radio is currently in binding mode.
pub fn is_binding() -> bool {
    cortex_m::interrupt::free(|_| {
        unsafe {
            RF_DRIVER.as_ref().map(|d| d.mode == afhds2a::RadioMode::Binding).unwrap_or(false)
        }
    })
}

/// Check if a newly captured/bound RX ID needs to be persisted to Flash, and return it.
pub fn take_pending_rx_save() -> Option<u32> {
    cortex_m::interrupt::free(|_| {
        if let Some(ref mut driver) = unsafe { RF_DRIVER.as_mut() } {
            if driver.rx_id_needs_save {
                driver.rx_id_needs_save = false;
                return Some(driver.rx_id);
            }
        }
        None
    })
}

/// Get currently active receiver ID.
#[allow(dead_code)]
pub fn get_rx_id() -> u32 {
    cortex_m::interrupt::free(|_| {
        unsafe {
            RF_DRIVER.as_ref().map(|d| d.rx_id).unwrap_or(0xFFFF_FFFF)
        }
    })
}

/// Get latest downlink telemetry from the receiver.
pub fn get_telemetry() -> TelemetryData {
    cortex_m::interrupt::free(|_| {
        unsafe {
            RF_DRIVER.as_ref().map(|d| d.telemetry).unwrap_or(TelemetryData::new())
        }
    })
}

/// TIM16 Interrupt Handler: Triggers transmission of next AFHDS 2A frame.
#[interrupt]
fn TIM16() {
    spi::clear_tim16_flag();

    if unsafe { RF_SILENCED } {
        return;
    }

    if let Some(ref mut driver) = unsafe { RF_DRIVER.as_mut() } {
        let chs = unsafe { &PENDING_CHANNELS };
        driver.set_channels(chs);
        driver.on_timer_tick();
    }
}

/// EXTI2_3 Interrupt Handler: Triggers when A7105 GIO2 transitions LOW
/// (indicating TX packet finished or RX packet received).
#[interrupt]
fn EXTI2_3() {
    unsafe {
        let exti_pr = core::ptr::read_volatile(0x4001_0414 as *const u32);
        if (exti_pr & (1 << 2)) != 0 {
            // Clear pending flag for EXTI2
            core::ptr::write_volatile(0x4001_0414 as *mut u32, 1 << 2);

            if let Some(ref mut driver) = RF_DRIVER.as_mut() {
                driver.on_gio2_event();
            }
        }
    }
}
