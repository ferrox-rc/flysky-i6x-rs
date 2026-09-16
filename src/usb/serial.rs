//! USB CDC-ACM Serial communications and telemetry streaming driver.
//!
//! Provides virtual COM port communication with standard JSON-lines telemetry streaming,
//! model inspection, and interactive serial CLI.

use crate::rf::afhds2a::TelemetryData;
use usbd_serial::SerialPort;

pub struct SerialHandler {
    rx_queue: [u8; 128],
    rx_head: usize,
    rx_tail: usize,
    cmd_buf: [u8; 64],
    cmd_len: usize,
}

impl SerialHandler {
    pub const fn new() -> Self {
        Self {
            rx_queue: [0u8; 128],
            rx_head: 0,
            rx_tail: 0,
            cmd_buf: [0u8; 64],
            cmd_len: 0,
        }
    }

    /// Drain incoming data from the USB serial OUT endpoint into the internal FIFO queue.
    /// MUST be called from the USB interrupt handler (or when polling) so that
    /// the STM32 hardware `CTR_RX` flag on the CDC OUT endpoint is cleared.
    /// Leaving `CTR_RX` asserted triggers an infinite NVIC interrupt storm on IRQ 31,
    /// which starves the CPU and hangs the transmitter.
    pub fn drain_rx<B: usb_device::bus::UsbBus>(&mut self, serial: &mut SerialPort<B>) {
        let mut buf = [0u8; 64];
        while let Ok(count) = serial.read(&mut buf) {
            if count == 0 {
                break;
            }
            for &b in &buf[..count] {
                let next = (self.rx_head + 1) % self.rx_queue.len();
                if next != self.rx_tail {
                    self.rx_queue[self.rx_head] = b;
                    self.rx_head = next;
                }
            }
        }
    }

    /// Process queued incoming data from the main loop and write pending telemetry frames.
    pub fn update<B: usb_device::bus::UsbBus>(
        &mut self,
        serial: &mut SerialPort<B>,
        rf_chs: &[u16; 14],
        telem: &TelemetryData,
        battery_mv: u16,
    ) {
        // Drain any pending data from the USB hardware endpoint
        self.drain_rx(serial);

        while self.rx_tail != self.rx_head {
            let b = self.rx_queue[self.rx_tail];
            self.rx_tail = (self.rx_tail + 1) % self.rx_queue.len();

            if b == b'\r' || b == b'\n' {
                let _ = serial.write(b"\r\n");
                if self.cmd_len > 0 {
                    self.handle_command(serial, rf_chs, telem, battery_mv);
                    self.cmd_len = 0;
                }
            } else if b == 0x08 || b == 0x7F {
                // Backspace
                if self.cmd_len > 0 {
                    self.cmd_len -= 1;
                    let _ = serial.write(b"\x08 \x08");
                }
            } else if self.cmd_len < self.cmd_buf.len() {
                self.cmd_buf[self.cmd_len] = b;
                self.cmd_len += 1;
                let _ = serial.write(&[b]); // Local echo
            }
        }
    }

    /// Dispatch interactive CLI command responses.
    fn handle_command<B: usb_device::bus::UsbBus>(
        &self,
        serial: &mut SerialPort<B>,
        rf_chs: &[u16; 14],
        telem: &TelemetryData,
        battery_mv: u16,
    ) {
        let cmd = core::str::from_utf8(&self.cmd_buf[..self.cmd_len]).unwrap_or("").trim();

        if cmd.eq_ignore_ascii_case("help") {
            let _ = serial.write(b"Commands: help, status, channels, telem, reboot\r\n");
        } else if cmd.eq_ignore_ascii_case("status") {
            let _ = serial.write(concat!("FlySky FS-i6X Rust Firmware v", env!("CARGO_PKG_VERSION"), "\r\n").as_bytes());
            self.send_telemetry_line(serial, rf_chs, telem, battery_mv);
        } else if cmd.eq_ignore_ascii_case("channels") {
            let mut cbuf = [0u8; 128];
            let mut pos = 0;
            macro_rules! append_ch {
                ($bytes:expr) => {
                    let s = $bytes;
                    if pos + s.len() <= cbuf.len() {
                        cbuf[pos..pos + s.len()].copy_from_slice(s);
                        pos += s.len();
                    }
                };
            }
            append_ch!(b"{\"ch\":[");
            for (i, &ch) in rf_chs.iter().enumerate() {
                if i > 0 {
                    append_ch!(b",");
                }
                let mut num = [b'0'; 4];
                num[0] = b'0' + ((ch / 1000) % 10) as u8;
                num[1] = b'0' + ((ch / 100) % 10) as u8;
                num[2] = b'0' + ((ch / 10) % 10) as u8;
                num[3] = b'0' + (ch % 10) as u8;
                append_ch!(&num);
            }
            append_ch!(b"]}\r\n");
            let _ = serial.write(&cbuf[..pos]);
        } else if cmd.eq_ignore_ascii_case("telem") {
            self.send_telemetry_line(serial, rf_chs, telem, battery_mv);
        } else if cmd.eq_ignore_ascii_case("reboot") {
            let _ = serial.write(b"Rebooting...\r\n");
            cortex_m::peripheral::SCB::sys_reset();
        } else if !cmd.is_empty() {
            let _ = serial.write(b"Unknown command. Type 'help'\r\n");
        }
    }

    /// Stream a single formatted JSON telemetry line over USB CDC.
    /// Format: {"vbat":5.18,"rssi":98,"rx_v":5.02,"tx":15820,"rx":15798,"err":22,"ch":[1500,...]}\r\n
    pub fn send_telemetry_line<B: usb_device::bus::UsbBus>(
        &self,
        serial: &mut SerialPort<B>,
        rf_chs: &[u16; 14],
        telem: &TelemetryData,
        battery_mv: u16,
    ) {
        // Only stream when host terminal is actively connected (DTR asserted)
        if !serial.dtr() {
            return;
        }

        let mut buf = [0u8; 192];
        let mut pos = 0;

        macro_rules! append {
            ($bytes:expr) => {
                let slice = $bytes;
                if pos + slice.len() <= buf.len() {
                    buf[pos..pos + slice.len()].copy_from_slice(slice);
                    pos += slice.len();
                }
            };
        }

        macro_rules! append_u32 {
            ($val:expr) => {
                let mut num_buf = [0u8; 10];
                let mut n: u32 = $val;
                let mut idx = 10;
                if n == 0 {
                    append!(b"0");
                } else {
                    while n > 0 {
                        idx -= 1;
                        num_buf[idx] = b'0' + (n % 10) as u8;
                        n /= 10;
                    }
                    append!(&num_buf[idx..]);
                }
            };
        }

        macro_rules! append_volt {
            ($mv:expr) => {
                let v = ($mv / 1000) as u8;
                let d1 = (($mv % 1000) / 100) as u8;
                let d2 = (($mv % 100) / 10) as u8;
                let mut v_buf = [b'0'; 4];
                v_buf[0] = b'0' + v;
                v_buf[1] = b'.';
                v_buf[2] = b'0' + d1;
                v_buf[3] = b'0' + d2;
                append!(&v_buf);
            };
        }

        append!(b"{\"vbat\":");
        append_volt!(battery_mv);
        append!(b",\"rssi\":");
        append_u32!(telem.rssi as u32);
        append!(b",\"rx_v\":");
        append_volt!(telem.rx_voltage_mv);
        append!(b",\"tx\":");
        append_u32!(telem.packets_sent);
        append!(b",\"rx\":");
        append_u32!(telem.packets_received);
        append!(b",\"err\":");
        append_u32!(telem.packets_sent.saturating_sub(telem.packets_received));
        append!(b",\"ch\":[");
        for (i, &ch) in rf_chs.iter().enumerate() {
            if i > 0 {
                append!(b",");
            }
            append_u32!(ch as u32);
        }
        append!(b"]}\r\n");

        let _ = serial.write(&buf[..pos]);
    }
}
