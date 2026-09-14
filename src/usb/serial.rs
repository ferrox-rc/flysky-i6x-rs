//! USB CDC-ACM Serial communications and telemetry streaming driver.
//!
//! Provides virtual COM port communication for live telemetry streaming,
//! model parameters inspection, and interactive serial CLI.

use crate::rf::afhds2a::TelemetryData;
use usbd_serial::SerialPort;

pub struct SerialHandler {
    rx_buf: [u8; 64],
    rx_len: usize,
}

impl SerialHandler {
    pub const fn new() -> Self {
        Self {
            rx_buf: [0u8; 64],
            rx_len: 0,
        }
    }

    /// Process incoming data and write pending telemetry frames.
    pub fn update<B: usb_device::bus::UsbBus>(
        &mut self,
        serial: &mut SerialPort<B>,
        rf_chs: &[u16; 14],
        telem: &TelemetryData,
        battery_mv: u16,
    ) {
        let mut buf = [0u8; 32];
        if let Ok(count) = serial.read(&mut buf) {
            for &b in &buf[..count] {
                if b == b'\r' || b == b'\n' {
                    if self.rx_len > 0 {
                        self.handle_command(serial, rf_chs, telem, battery_mv);
                        self.rx_len = 0;
                    }
                } else if self.rx_len < self.rx_buf.len() {
                    self.rx_buf[self.rx_len] = b;
                    self.rx_len += 1;
                }
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
        let cmd = core::str::from_utf8(&self.rx_buf[..self.rx_len]).unwrap_or("").trim();

        if cmd.eq_ignore_ascii_case("help") {
            let _ = serial.write(b"Commands: help, status, channels, telem\r\n");
        } else if cmd.eq_ignore_ascii_case("status") {
            let _ = serial.write(b"FS-i6X Rust Firmware v0.12.0\r\n");
            self.send_telemetry_line(serial, telem, battery_mv);
        } else if cmd.eq_ignore_ascii_case("channels") {
            let _ = serial.write(b"CH:");
            for &ch in rf_chs.iter() {
                let mut cbuf = [0u8; 5];
                cbuf[0] = b' ';
                cbuf[1] = b'0' + ((ch / 1000) % 10) as u8;
                cbuf[2] = b'0' + ((ch / 100) % 10) as u8;
                cbuf[3] = b'0' + ((ch / 10) % 10) as u8;
                cbuf[4] = b'0' + (ch % 10) as u8;
                let _ = serial.write(&cbuf);
            }
            let _ = serial.write(b"\r\n");
        } else if cmd.eq_ignore_ascii_case("telem") {
            self.send_telemetry_line(serial, telem, battery_mv);
        } else {
            let _ = serial.write(b"Unknown command. Type 'help'\r\n");
        }
    }

    /// Stream a single formatted telemetry line over USB CDC.
    pub fn send_telemetry_line<B: usb_device::bus::UsbBus>(
        &self,
        serial: &mut SerialPort<B>,
        telem: &TelemetryData,
        battery_mv: u16,
    ) {
        let mut line = [b' '; 64];
        // Format: "TLM: VBAT=X.YYV RSSI=XX% RXV=X.YYV TX=N RX=N\r\n"
        let mut i = 0;
        let prefix = b"TLM: VBAT=";
        line[..prefix.len()].copy_from_slice(prefix);
        i += prefix.len();

        let v = (battery_mv / 1000) as u8;
        let d1 = ((battery_mv % 1000) / 100) as u8;
        let d2 = ((battery_mv % 100) / 10) as u8;
        line[i] = b'0' + v; i += 1;
        line[i] = b'.'; i += 1;
        line[i] = b'0' + d1; i += 1;
        line[i] = b'0' + d2; i += 1;
        line[i] = b'V'; i += 1;

        let rssi_lbl = b" RSSI=";
        line[i..i + rssi_lbl.len()].copy_from_slice(rssi_lbl);
        i += rssi_lbl.len();
        let r = telem.rssi.min(100);
        line[i] = b'0' + (r / 10); i += 1;
        line[i] = b'0' + (r % 10); i += 1;
        line[i] = b'%'; i += 1;

        let rxv_lbl = b" RXV=";
        line[i..i + rxv_lbl.len()].copy_from_slice(rxv_lbl);
        i += rxv_lbl.len();
        let rx_v = (telem.rx_voltage_mv / 1000) as u8;
        let rx_d1 = ((telem.rx_voltage_mv % 1000) / 100) as u8;
        let rx_d2 = ((telem.rx_voltage_mv % 100) / 10) as u8;
        line[i] = b'0' + rx_v; i += 1;
        line[i] = b'.'; i += 1;
        line[i] = b'0' + rx_d1; i += 1;
        line[i] = b'0' + rx_d2; i += 1;
        line[i] = b'V'; i += 1;

        line[i] = b'\r'; i += 1;
        line[i] = b'\n'; i += 1;

        let _ = serial.write(&line[..i]);
    }
}
