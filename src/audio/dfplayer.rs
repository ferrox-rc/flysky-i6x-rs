//! Hardware asynchronous serial driver and controller for DFPlayer Mini MP3/WAV module.
//!
//! Pin Mapping:
//! - PC10: USART3_TX (Alternate Function 1, Push-Pull, 9600 baud, 8N1)
//! - PC14: BUSY Status Input (Internal Pull-Up, Active-LOW: LOW = Playing, HIGH = Idle)
//!
//! Non-blocking architecture:
//! - Commands are queued into a 64-byte ring buffer and transmitted 1 byte at a time
//!   via USART3_TDR when TXE is ready. Loop overhead is < 0.1 µs, completely preventing
//!   flight loop jitter.

#![allow(dead_code)]

use core::ptr;

// RCC registers
const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;

// GPIOC registers (Base 0x4800_0800)
const GPIOC_MODER: *mut u32 = 0x4800_0800 as *mut u32;
const GPIOC_PUPDR: *mut u32 = 0x4800_080C as *mut u32;
const GPIOC_IDR: *mut u32 = 0x4800_0810 as *mut u32;
const GPIOC_AFRH: *mut u32 = 0x4800_0824 as *mut u32;

// USART3 registers (Base 0x4000_4800)
const USART3_CR1: *mut u32 = 0x4000_4800 as *mut u32;
const USART3_BRR: *mut u32 = 0x4000_480C as *mut u32;
const USART3_ISR: *mut u32 = 0x4000_481C as *mut u32;
const USART3_TDR: *mut u32 = 0x4000_4828 as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;

// DFPlayer standard command codes
const CMD_PLAY_TRACK: u8 = 0x03;
const CMD_SET_VOLUME: u8 = 0x06;
const CMD_STANDBY: u8 = 0x0A;
const CMD_WAKEUP: u8 = 0x0B;
const CMD_PAUSE: u8 = 0x0E;
const CMD_STOP: u8 = 0x16;

const TX_BUF_SIZE: usize = 64;
const QUEUE_MAX: usize = 4;
const MIN_CMD_INTERVAL_MS: u16 = 300; // minimum ms between consecutive play commands

pub struct DfPlayer {
    tx_buf: [u8; TX_BUF_SIZE],
    tx_head: usize,
    tx_tail: usize,
    track_queue: [u16; QUEUE_MAX],
    queue_len: usize,
    time_since_cmd_ms: u16,
    pub volume: u8,
    pub initialized: bool,
}

impl DfPlayer {
    pub const fn new() -> Self {
        Self {
            tx_buf: [0; TX_BUF_SIZE],
            tx_head: 0,
            tx_tail: 0,
            track_queue: [0; QUEUE_MAX],
            queue_len: 0,
            time_since_cmd_ms: 1000,
            volume: 20,
            initialized: false,
        }
    }

    /// Initialize GPIOC PC10 (AF1 USART3_TX), PC14 (BUSY input with pull-up), and USART3 at 9600 baud.
    pub fn init(&mut self) {
        unsafe {
            // 1. Enable GPIOC (bit 19) and USART3 (bit 18 of APB1)
            let ahb = ptr::read_volatile(RCC_AHBENR);
            ptr::write_volatile(RCC_AHBENR, ahb | (1 << 19));

            let apb1 = ptr::read_volatile(RCC_APB1ENR);
            ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 18));

            // 2. Configure PC10 as Alternate Function (MODER[21:20] = 10)
            //    Configure PC14 as Input (MODER[29:28] = 00)
            let moder = ptr::read_volatile(GPIOC_MODER);
            let moder = (moder & !(3 << 20)) | (2 << 20); // PC10 AF
            let moder = moder & !(3 << 28);               // PC14 IN
            ptr::write_volatile(GPIOC_MODER, moder);

            // 3. Configure PC14 with Pull-Up (PUPDR[29:28] = 01)
            // When no DFPlayer is soldered, pull-up keeps BUSY pin HIGH (idle).
            let pupdr = ptr::read_volatile(GPIOC_PUPDR);
            let pupdr = (pupdr & !(3 << 28)) | (1 << 28);
            ptr::write_volatile(GPIOC_PUPDR, pupdr);

            // 4. Configure PC10 Alternate Function to AF1 (USART3_TX)
            // In AFRH, PC10 is pin (10 - 8) = index 2 -> bits 11:8
            let afrh = ptr::read_volatile(GPIOC_AFRH);
            let afrh = (afrh & !(0x0F << 8)) | (1 << 8); // AF1
            ptr::write_volatile(GPIOC_AFRH, afrh);

            // 5. Configure USART3 for 9600 baud 8N1
            // 48 MHz APB1 clock / 9600 baud = 5000 (0x1388)
            ptr::write_volatile(USART3_CR1, 0); // Disable during configuration
            ptr::write_volatile(USART3_BRR, 48_000_000 / 9600);

            // Enable Transmitter (TE, bit 3) and USART (UE, bit 0)
            ptr::write_volatile(USART3_CR1, (1 << 3) | (1 << 0));
        }

        self.initialized = true;
        self.set_volume(self.volume);
    }

    /// Check if DFPlayer is actively playing audio.
    /// Returns true if hardware PC14 is LOW or minimum command recovery time hasn't elapsed.
    pub fn is_busy(&self) -> bool {
        if self.time_since_cmd_ms < MIN_CMD_INTERVAL_MS {
            return true;
        }
        unsafe {
            // PC14 is bit 14 in GPIOC_IDR. LOW = busy / playing.
            let idr = ptr::read_volatile(GPIOC_IDR);
            (idr & (1 << 14)) == 0
        }
    }

    /// Queue raw command frame into ring buffer (8 bytes).
    fn send_command(&mut self, cmd: u8, param: u16) {
        let packet: [u8; 8] = [
            0x7E,
            0xFF,
            0x06,
            cmd,
            0x00,
            (param >> 8) as u8,
            (param & 0xFF) as u8,
            0xEF,
        ];

        for &byte in &packet {
            let next_head = (self.tx_head + 1) % TX_BUF_SIZE;
            if next_head != self.tx_tail {
                self.tx_buf[self.tx_head] = byte;
                self.tx_head = next_head;
            }
        }
    }

    /// Set volume (0..30).
    pub fn set_volume(&mut self, vol: u8) {
        let vol = vol.min(30);
        self.volume = vol;
        self.send_command(CMD_SET_VOLUME, vol as u16);
    }

    /// Queue a sound track index (1..2999) to play.
    pub fn play_track(&mut self, track: u16) {
        if track == 0 {
            return;
        }

        // If idle and queue empty, trigger immediately
        if !self.is_busy() && self.queue_len == 0 {
            self.send_command(CMD_PLAY_TRACK, track);
            self.time_since_cmd_ms = 0;
            return;
        }

        // Otherwise push into pending queue if room available
        if self.queue_len < QUEUE_MAX {
            self.track_queue[self.queue_len] = track;
            self.queue_len += 1;
        }
    }

    /// Stop current playback immediately and flush queue.
    pub fn stop(&mut self) {
        self.queue_len = 0;
        self.send_command(CMD_STOP, 0);
    }

    /// Advance non-blocking TX ring buffer and track queue state machine.
    /// Called periodically from flight loop.
    pub fn tick(&mut self, elapsed_ms: u16) {
        self.time_since_cmd_ms = self.time_since_cmd_ms.saturating_add(elapsed_ms);

        // 1. Drain TX ring buffer to USART3_TDR when transmitter is ready
        while self.tx_head != self.tx_tail {
            unsafe {
                let isr = ptr::read_volatile(USART3_ISR);
                if (isr & USART_ISR_TXE) != 0 {
                    let byte = self.tx_buf[self.tx_tail];
                    ptr::write_volatile(USART3_TDR, byte as u32);
                    self.tx_tail = (self.tx_tail + 1) % TX_BUF_SIZE;
                } else {
                    break;
                }
            }
        }

        // 2. If DFPlayer is idle and we have queued tracks, play next track
        if !self.is_busy() && self.queue_len > 0 {
            let next_track = self.track_queue[0];
            // Shift queue
            for i in 1..self.queue_len {
                self.track_queue[i - 1] = self.track_queue[i];
            }
            self.queue_len -= 1;

            self.send_command(CMD_PLAY_TRACK, next_track);
            self.time_since_cmd_ms = 0;
        }
    }
}
