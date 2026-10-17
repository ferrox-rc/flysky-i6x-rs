//! Hardware USART2 driver and PC13 power switch control for CRSF / ExpressLRS.
//!
//! Pin mapping:
//! - PD5:  USART2_TX (AF0, Push-Pull, High Speed)
//! - PA15: USART2_RX (AF1, Pull-Up, High Speed)
//! - PC13: Module Power Switch (GPIO Output, Push-Pull, High = Power ON)

#![allow(dead_code)]

#[cfg(not(test))]
use stm32f0xx_hal::pac::{self, interrupt};

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(not(test))]
const RX_RING_SIZE: usize = 128;
#[cfg(not(test))]
static mut RX_RING: [u8; RX_RING_SIZE] = [0; RX_RING_SIZE];
#[cfg(not(test))]
static RX_HEAD: AtomicUsize = AtomicUsize::new(0);
#[cfg(not(test))]
static RX_TAIL: AtomicUsize = AtomicUsize::new(0);

static ACTIVE_HIGH: AtomicBool = AtomicBool::new(true);
static POWER_ON: AtomicBool = AtomicBool::new(false);
static HALF_DUPLEX_ACTIVE: AtomicBool = AtomicBool::new(false);
static ECHO_SKIP_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Apply current PC13 pin state based on power state and active polarity.
#[cfg(not(test))]
unsafe fn apply_power_pin() {
    let pin_high = if ACTIVE_HIGH.load(Ordering::Relaxed) {
        POWER_ON.load(Ordering::Relaxed)
    } else {
        !POWER_ON.load(Ordering::Relaxed)
    };
    let gpioc = &*pac::GPIOC::ptr();
    if pin_high {
        gpioc.bsrr.write(|w| w.bs13().set_bit()); // High
    } else {
        gpioc.bsrr.write(|w| w.br13().set_bit()); // Low
    }
}

#[cfg(test)]
unsafe fn apply_power_pin() {}

/// Initialize GPIO pins: PC13 as power switch (initially OFF), PD5 and PA15 peripheral clocks.
pub fn init(active_high: bool) {
    #[cfg(not(test))]
    unsafe {
        ACTIVE_HIGH.store(active_high, Ordering::Relaxed);
        POWER_ON.store(false, Ordering::Relaxed);

        // Enable GPIOA, GPIOC, GPIOD clocks in RCC_AHBENR
        let rcc = &*pac::RCC::ptr();
        rcc.ahbenr
            .modify(|_, w| w.iopaen().set_bit().iopcen().set_bit().iopden().set_bit());

        // Configure PC13 as general purpose output
        let gpioc = &*pac::GPIOC::ptr();
        gpioc.moder.modify(|_, w| w.moder13().output());

        // Apply initial OFF state according to polarity
        apply_power_pin();
    }
    #[cfg(test)]
    let _ = active_high;
}

/// Set active power polarity for PC13: true = Active HIGH (N-type), false = Active LOW (P-type).
pub fn set_power_polarity(active_high: bool) {
    #[cfg(not(test))]
    unsafe {
        ACTIVE_HIGH.store(active_high, Ordering::Relaxed);
        apply_power_pin();
    }
    #[cfg(test)]
    let _ = active_high;
}

/// Set external module power state via PC13
pub fn set_module_power(power_on: bool) {
    #[cfg(not(test))]
    unsafe {
        POWER_ON.store(power_on, Ordering::Relaxed);
        apply_power_pin();
    }
    #[cfg(test)]
    let _ = power_on;
}

/// Baud rate divisors for 48.000 MHz clock tree
pub fn get_brr_for_baud(baud_idx: u8) -> u32 {
    match baud_idx {
        0 => 417, // 115,200 bps (48,000,000 / 115,200 = 416.66)
        1 => 115, // 416,666 bps (48,000,000 / 416,666.67 = 115.20)
        2 => 114, // 420,000 bps (48,000,000 / 420,000 = 114.285)
        3 => 52,  // 921,600 bps (48,000,000 / 921,600 = 52.08)
        4 => 26,  // 1,875,000 bps (48,000,000 / 1,875,000 = 25.6 -> 26 = 1,846,154 bps, 1.5% error)
        _ => 114, // default 420k (ELRS)
    }
}

/// Returns true if half-duplex (1-wire) mode is currently active.
pub fn is_half_duplex() -> bool {
    HALF_DUPLEX_ACTIVE.load(Ordering::Relaxed)
}

/// Configure and enable/disable USART2
pub fn set_uart_enabled(enabled: bool, baud_idx: u8, half_duplex: bool) {
    HALF_DUPLEX_ACTIVE.store(half_duplex, Ordering::Relaxed);
    ECHO_SKIP_COUNT.store(0, Ordering::Relaxed);

    #[cfg(not(test))]
    unsafe {
        let rcc = &*pac::RCC::ptr();
        let gpiod = &*pac::GPIOD::ptr();
        let gpioa = &*pac::GPIOA::ptr();
        let usart2 = &*pac::USART2::ptr();

        if enabled {
            // 1. Enable USART2 clock in RCC_APB1ENR (bit 17)
            rcc.apb1enr.modify(|_, w| w.usart2en().set_bit());

            // 2. Configure PD5 as AF0 (USART2_TX): MODER=10, AFRL bit 20..23 = 0000
            gpiod.moder.modify(|_, w| w.moder5().alternate());
            gpiod.ospeedr.modify(|_, w| w.ospeedr5().high_speed());
            gpiod.afrl.modify(|_, w| w.afrl5().af0());

            if half_duplex {
                // Half-duplex single-wire: PD5 Open-Drain with internal Pull-Up
                gpiod.otyper.modify(|_, w| w.ot5().open_drain());
                gpiod.pupdr.modify(|_, w| w.pupdr5().pull_up());

                // PA15 is not used in half-duplex; float as input
                gpioa.moder.modify(|_, w| w.moder15().input());
            } else {
                // Full-duplex: PD5 Push-Pull, no pull resistor
                gpiod.otyper.modify(|_, w| w.ot5().push_pull());
                gpiod.pupdr.modify(|_, w| w.pupdr5().floating());

                // Configure PA15 as AF1 (USART2_RX): MODER=10, AFRH bit 28..31 = 0001, Pull-Up
                gpioa.moder.modify(|_, w| w.moder15().alternate());
                gpioa.pupdr.modify(|_, w| w.pupdr15().pull_up());
                gpioa.afrh.modify(|_, w| w.afrh15().af1());
            }

            // 4. Reset USART2 registers
            usart2.cr1.reset();
            usart2.cr2.reset();

            // In half-duplex mode, enable HDSEL (bit 3 in CR3)
            if half_duplex {
                usart2.cr3.write(|w| w.hdsel().set_bit());
            } else {
                usart2.cr3.reset();
            }

            // 5. Set Baud rate divisor
            let brr = get_brr_for_baud(baud_idx);
            usart2.brr.write(|w| w.brr().bits(brr as u16));

            // Clear any pending error flags
            usart2.icr.write(|w| w.bits(0xFFFF_FFFF));

            // Reset RX ring buffer indices
            RX_HEAD.store(0, Ordering::Relaxed);
            RX_TAIL.store(0, Ordering::Relaxed);

            // Configure NVIC for USART2 (IRQ 28)
            cortex_m::peripheral::NVIC::unpend(pac::Interrupt::USART2);
            let mut cp = cortex_m::peripheral::Peripherals::steal();
            cp.NVIC.set_priority(pac::Interrupt::USART2, 0x40);
            cortex_m::peripheral::NVIC::unmask(pac::Interrupt::USART2);

            // 6. Enable UE, TE, RE, and RXNEIE
            usart2.cr1.write(|w| {
                w.ue()
                    .enabled()
                    .te()
                    .enabled()
                    .re()
                    .enabled()
                    .rxneie()
                    .enabled()
            });
        } else {
            // Mask USART2 in NVIC
            cortex_m::peripheral::NVIC::mask(pac::Interrupt::USART2);

            // Disable UE & RXNEIE
            usart2.cr1.reset();

            // Reset RX ring buffer indices
            RX_HEAD.store(0, Ordering::Relaxed);
            RX_TAIL.store(0, Ordering::Relaxed);

            // Disable USART2 clock in RCC_APB1ENR (bit 17)
            rcc.apb1enr.modify(|_, w| w.usart2en().clear_bit());

            // Set PD5 and PA15 back to inputs to float pins
            gpiod.moder.modify(|_, w| w.moder5().input());
            gpiod.otyper.modify(|_, w| w.ot5().push_pull());
            gpioa.moder.modify(|_, w| w.moder15().input());
        }
    }
    #[cfg(test)]
    let _ = (enabled, baud_idx, half_duplex);
}

/// Transmit a buffer over USART2 (non-blocking if space available, bounded timeout)
pub fn write_bytes(bytes: &[u8]) -> usize {
    #[cfg(test)]
    {
        mock::record_tx(bytes);
        bytes.len()
    }
    #[cfg(not(test))]
    unsafe {
        let is_hd = HALF_DUPLEX_ACTIVE.load(Ordering::Relaxed);
        let usart2 = &*pac::USART2::ptr();
        let mut sent = 0;
        for &b in bytes {
            let mut timeout = 2500u32;
            while usart2.isr.read().txe().bit_is_clear() {
                timeout -= 1;
                if timeout == 0 {
                    return sent;
                }
            }
            if is_hd {
                cortex_m::interrupt::free(|_| {
                    let cur = ECHO_SKIP_COUNT.load(Ordering::Relaxed);
                    ECHO_SKIP_COUNT.store(cur + 1, Ordering::Relaxed);
                });
            }
            usart2.tdr.write(|w| w.tdr().bits(b as u16));
            sent += 1;
        }
        sent
    }
}

/// Read a byte from USART2 RX if available
pub fn read_byte() -> Option<u8> {
    #[cfg(test)]
    {
        mock::pop_rx_byte()
    }
    #[cfg(not(test))]
    {
        let head = RX_HEAD.load(Ordering::Acquire);
        let tail = RX_TAIL.load(Ordering::Relaxed);
        if head != tail {
            let b = unsafe { RX_RING[tail] };
            RX_TAIL.store((tail + 1) & (RX_RING_SIZE - 1), Ordering::Release);
            Some(b)
        } else {
            None
        }
    }
}

#[cfg(not(test))]
#[interrupt]
fn USART2() {
    unsafe {
        let usart2 = &*pac::USART2::ptr();
        let isr = usart2.isr.read();
        // Clear overrun error if present
        if isr.ore().bit_is_set() {
            usart2.icr.write(|w| w.orecf().clear());
        }

        if isr.rxne().bit_is_set() {
            let b = (usart2.rdr.read().rdr().bits() & 0xFF) as u8;
            if HALF_DUPLEX_ACTIVE.load(Ordering::Relaxed) {
                let skip = ECHO_SKIP_COUNT.load(Ordering::SeqCst);
                if skip > 0 {
                    ECHO_SKIP_COUNT.store(skip - 1, Ordering::SeqCst);
                    return;
                }
            }

            let head = RX_HEAD.load(Ordering::Relaxed);
            let tail = RX_TAIL.load(Ordering::Acquire);
            let next_head = (head + 1) & (RX_RING_SIZE - 1);
            if next_head != tail {
                RX_RING[head] = b;
                RX_HEAD.store(next_head, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
pub mod mock {
    extern crate std;
    use std::sync::Mutex;
    use std::vec::Vec;

    static RX_QUEUE: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    static TX_LOG: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

    pub fn push_rx_bytes(bytes: &[u8]) {
        let mut q = RX_QUEUE.lock().unwrap();
        q.extend_from_slice(bytes);
    }

    pub fn pop_rx_byte() -> Option<u8> {
        let mut q = RX_QUEUE.lock().unwrap();
        if q.is_empty() {
            None
        } else {
            Some(q.remove(0))
        }
    }

    pub fn record_tx(data: &[u8]) {
        let mut log = TX_LOG.lock().unwrap();
        log.push(data.to_vec());
    }

    pub fn take_tx() -> Vec<Vec<u8>> {
        let mut log = TX_LOG.lock().unwrap();
        std::mem::take(&mut *log)
    }

    pub fn tx_count() -> usize {
        TX_LOG.lock().unwrap().len()
    }

    pub fn clear() {
        RX_QUEUE.lock().unwrap().clear();
        TX_LOG.lock().unwrap().clear();
    }
}
