//! AFHDS 2A (Automatic Frequency Hopping Digital System 2nd Gen) Protocol.
//!
//! Implements:
//! - Deterministic 16-channel spread-spectrum frequency hopping from TX ID
//! - 14-channel 38-byte control frame generation (1000..2000 µs pulse widths)
//! - 4-phase bidirectional bind sequence
//! - 37-byte telemetry frame parsing (RSSI, RX battery voltage)

#![allow(dead_code)]

use super::{a7105, spi};

pub const NUM_CHANNELS: usize = 14;
pub const NUM_FREQ: usize = 16;
pub const TX_PACKET_SIZE: usize = 38;
pub const RX_PACKET_SIZE: usize = 37;

// Packet command bytes
pub const PACKET_STICKS: u8 = 0x58;
pub const PACKET_FAILSAFE: u8 = 0x56;
pub const PACKET_SETTINGS: u8 = 0xAA;
pub const PACKET_BIND1: u8 = 0xBB;
pub const PACKET_BIND2: u8 = 0xBC;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RadioMode {
    Normal,
    Binding,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PacketType {
    Sticks,
    Settings,
    Failsafe,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum SubState {
    WaitingTxDone,
    ListeningRx,
    Idle,
}

#[derive(Copy, Clone, Debug)]
pub struct TelemetryData {
    pub connected: bool,
    pub rssi: u8,            // 0..100%
    pub rx_voltage_mv: u16,  // in millivolts (e.g. 5000 for 5.00V)
    pub packets_sent: u32,
    pub packets_received: u32,
}

impl TelemetryData {
    pub const fn new() -> Self {
        Self {
            connected: false,
            rssi: 0,
            rx_voltage_mv: 0,
            packets_sent: 0,
            packets_received: 0,
        }
    }
}

/// Read persisted receiver ID from Flash if previously bound.
pub fn load_saved_rx_id() -> Option<u32> {
    let storage = crate::storage::load_storage();
    let rx_id = storage.active_model().rx_id;
    if rx_id != 0 && rx_id != 0xFFFF_FFFF {
        Some(rx_id)
    } else {
        None
    }
}

pub struct Afhds2a {
    pub tx_id: u32,
    pub rx_id: u32,
    pub hopping_table: [u8; NUM_FREQ],
    pub hopping_idx: u8,
    pub mode: RadioMode,
    sub_state: SubState,
    bind_phase: u8,
    pub bind_confirm_count: u8,
    pub bind_done: bool,
    pub rx_id_needs_save: bool,
    pub next_packet_type: PacketType,
    pub channels: [u16; NUM_CHANNELS],
    pub telemetry: TelemetryData,
    loss_counter: u16,
}

impl Afhds2a {
    pub fn new(tx_id: u32) -> Self {
        let hopping_table = calculate_hopping_table(tx_id);
        let rx_id = load_saved_rx_id().unwrap_or(0xFFFF_FFFF);
        let is_initially_bound = rx_id != 0xFFFF_FFFF && rx_id != 0;
        Self {
            tx_id,
            rx_id,
            hopping_table,
            hopping_idx: 0,
            mode: RadioMode::Normal,
            sub_state: SubState::Idle,
            bind_phase: 0,
            bind_confirm_count: 0,
            bind_done: is_initially_bound,
            rx_id_needs_save: false,
            next_packet_type: PacketType::Sticks,
            channels: [1500; NUM_CHANNELS], // All centered 1500 µs
            telemetry: TelemetryData::new(),
            loss_counter: 0,
        }
    }

    /// Update 14 RC channel pulse widths (1000..2000 µs).
    pub fn set_channels(&mut self, chs: &[u16; NUM_CHANNELS]) {
        self.channels.copy_from_slice(chs);
    }

    /// Enter or exit binding mode.
    pub fn set_bind_mode(&mut self, enable: bool) {
        if enable {
            self.mode = RadioMode::Binding;
            self.bind_phase = 1;
            self.bind_confirm_count = 0;
            self.bind_done = false;
            a7105::set_power(a7105::BIND_POWER);
        } else {
            self.mode = RadioMode::Normal;
            self.bind_phase = 0;
            self.bind_confirm_count = 0;
            self.bind_done = true;
            self.next_packet_type = PacketType::Sticks;
            a7105::set_power(a7105::HIGH_POWER);

            // If an RX ID was not captured (one-way receiver like FS-A8S, Fli14, etc.),
            // assign a deterministic non-zero ID derived from TX ID and mark for save!
            if self.rx_id == 0 || self.rx_id == 0xFFFF_FFFF {
                self.rx_id = 0x0001_0000 | (self.tx_id & 0xFFFF);
            }
            self.rx_id_needs_save = true;
        }
    }

    /// Dynamically update receiver ID (called when switching active model profile).
    pub fn set_rx_id(&mut self, rx_id: u32) {
        self.rx_id = if rx_id != 0 { rx_id } else { 0xFFFF_FFFF };
        self.bind_done = rx_id != 0 && rx_id != 0xFFFF_FFFF;
        self.loss_counter = 0;
        self.telemetry.connected = false;
    }

    /// Periodic timer callback (called every 3.85 ms from TIM16 ISR).
    pub fn on_timer_tick(&mut self) {
        // Prevent race conditions with GIO2 during packet generation and SPI setup
        spi::disable_gio2_irq();

        self.telemetry.packets_sent = self.telemetry.packets_sent.wrapping_add(1);

        // Connection loss detection (no RX packets received for > 200 ms ~ 52 packets)
        self.loss_counter = self.loss_counter.saturating_add(1);
        if self.loss_counter > 52 {
            self.telemetry.connected = false;
            self.telemetry.rssi = 0;
        }

        let mut tx_buf = [0u8; TX_PACKET_SIZE];
        let channel: u8;

        match self.mode {
            RadioMode::Binding => {
                // Alternate bind channels between 0x0D (13) and 0x8C (140)
                channel = if (self.telemetry.packets_sent & 1) != 0 {
                    0x0D
                } else {
                    0x8C
                };
                self.build_bind_packet(&mut tx_buf);

                // In BIND4: send 4 confirmation packets before hopping
                if self.bind_phase == 4 {
                    self.bind_confirm_count = self.bind_confirm_count.saturating_add(1);
                    if self.bind_confirm_count >= 4 {
                        self.mode = RadioMode::Normal;
                        self.bind_phase = 0;
                        self.bind_confirm_count = 0;
                        self.bind_done = true;
                        self.rx_id_needs_save = true;
                        self.hopping_idx = 1;
                        self.next_packet_type = PacketType::Sticks;
                        a7105::set_power(a7105::HIGH_POWER);
                    }
                }
            }
            RadioMode::Normal => {
                channel = self.hopping_table[(self.hopping_idx as usize) & 0x0F];
                self.hopping_idx = (self.hopping_idx + 1) & 0x0F;

                // Alternate antenna on each frequency hop for spatial diversity
                if self.hopping_idx != 0 {
                    spi::switch_antenna();
                }

                // Autonomous periodic failsafe broadcast: every 1,569 packets (~6.0s at 260 Hz),
                // refresh receiver failsafe register memory even if downlink frames were dropped.
                if self.next_packet_type == PacketType::Sticks && self.telemetry.packets_sent.is_multiple_of(1569) {
                    self.next_packet_type = PacketType::Failsafe;
                }

                match self.next_packet_type {
                    PacketType::Settings => {
                        self.build_settings_packet(&mut tx_buf);
                        self.next_packet_type = PacketType::Sticks;
                    }
                    PacketType::Failsafe => {
                        self.build_failsafe_packet(&mut tx_buf);
                        self.next_packet_type = PacketType::Sticks;
                    }
                    PacketType::Sticks => {
                        self.build_stick_packet(&mut tx_buf);
                    }
                }
            }
        }

        // Transmit packet on selected RF channel
        a7105::write_fifo(&tx_buf, channel);

        // Wait for GIO2 to signal TX completion
        self.sub_state = SubState::WaitingTxDone;
        spi::enable_gio2_irq();
    }

    /// Event handler for A7105 GIO2 pin falling edge (from EXTI2_3 ISR).
    pub fn on_gio2_event(&mut self) {
        spi::disable_gio2_irq();

        match self.sub_state {
            SubState::WaitingTxDone => {
                // TX completed! Switch to RX mode to listen for downlink telemetry
                self.sub_state = SubState::ListeningRx;

                if self.mode == RadioMode::Binding {
                    // Turn LNA off during bind RX to prevent front-end swamping at near range (< 20 cm)
                    spi::set_tx_rx_mode(spi::RF_MODE_OFF);
                    // Cycle bind searching phases 1 -> 2 -> 3 -> 1
                    if self.bind_phase < 4 {
                        self.bind_phase = (self.bind_phase % 3) + 1;
                    }
                } else {
                    // Normal mode: enable RX LNA for maximum downlink sensitivity
                    spi::set_tx_rx_mode(spi::RF_MODE_RX_EN);
                }

                a7105::strobe(a7105::STROBE_RX);
                spi::enable_gio2_irq();
            }
            SubState::ListeningRx => {
                // Packet received in RX FIFO!
                self.sub_state = SubState::Idle;

                if a7105::is_crc_ok() {
                    let mut rx_buf = [0u8; RX_PACKET_SIZE];
                    a7105::read_fifo(&mut rx_buf);
                    self.process_rx_packet(&rx_buf);
                }

                // Cleanly return to standby; GIO2 IRQ remains disabled until next on_timer_tick
                a7105::strobe(a7105::STROBE_STANDBY);
            }
            SubState::Idle => {}
        }
    }

    fn build_stick_packet(&self, out: &mut [u8; TX_PACKET_SIZE]) {
        out[0] = PACKET_STICKS;
        out[1..5].copy_from_slice(&self.tx_id.to_le_bytes());
        out[5..9].copy_from_slice(&self.rx_id.to_le_bytes());

        // Pack 14 channels (each 12 bits, little endian)
        for ch in 0..NUM_CHANNELS {
            let val = self.channels[ch].clamp(1000, 2000);
            out[9 + ch * 2] = (val & 0xFF) as u8;
            out[10 + ch * 2] = ((val >> 8) & 0x0F) as u8;
        }

        out[37] = 0x00;
    }

    fn build_settings_packet(&self, out: &mut [u8; TX_PACKET_SIZE]) {
        out[0] = PACKET_SETTINGS; // 0xAA
        out[1..5].copy_from_slice(&self.tx_id.to_le_bytes());
        out[5..9].copy_from_slice(&self.rx_id.to_le_bytes());
        out[9] = 0xFD;
        out[10] = 0xFF;
        out[11] = 0x90; // 400 Hz servo refresh rate (low byte: 400 = 0x0190)
        out[12] = 0x01; // high byte
        out[13] = 0x00; // PWM output enabled (0x00 = PWM, 0x01 = PPM)
        out[14] = 0x00;
        out[15..37].fill(0xFF);
        out[18] = 0x05;
        out[19] = 0xDC; // 1500 µs center pulse
        out[20] = 0x05;
        out[21] = 0xDE; // i-BUS serial output enabled (0xDE = i-BUS, 0xDD = SBUS)
        out[37] = 0x00;
    }

    fn build_failsafe_packet(&self, out: &mut [u8; TX_PACKET_SIZE]) {
        out[0] = PACKET_FAILSAFE; // 0x56
        out[1..5].copy_from_slice(&self.tx_id.to_le_bytes());
        out[5..9].copy_from_slice(&self.rx_id.to_le_bytes());

        for ch in 0..NUM_CHANNELS {
            if ch == 2 {
                // CH3 (Throttle): failsafe cutoff to 1000 µs (motor stop)
                out[9 + ch * 2] = 0xE8;
                out[10 + ch * 2] = 0x03;
            } else {
                // All other channels: Hold last position (0xFFFF)
                out[9 + ch * 2] = 0xFF;
                out[10 + ch * 2] = 0xFF;
            }
        }

        out[37] = 0x00;
    }

    fn build_bind_packet(&self, out: &mut [u8; TX_PACKET_SIZE]) {
        out[1..5].copy_from_slice(&self.tx_id.to_le_bytes());
        out[10] = 0x00;

        match self.bind_phase {
            1 => {
                out[0] = PACKET_BIND1; // 0xBB
                out[5..9].fill(0xFF);
                out[9] = 0x01;
                out[11..27].copy_from_slice(&self.hopping_table);
                out[27..37].fill(0xFF);
            }
            2 => {
                out[0] = PACKET_BIND2; // 0xBC
                out[5..9].fill(0xFF);
                out[9] = 0x00;
                out[11..27].copy_from_slice(&self.hopping_table);
                out[27] = 0x01;
                out[28] = 0x80;
                out[29..37].fill(0xFF);
            }
            3 => {
                out[0] = PACKET_BIND2; // 0xBC
                out[5..9].fill(0xFF);
                out[9] = 0x01;
                out[11..27].copy_from_slice(&self.hopping_table);
                out[27] = 0x01;
                out[28] = 0x80;
                out[29..37].fill(0xFF);
            }
            _ => {
                // BIND4 confirmation packet
                out[0] = PACKET_BIND2; // 0xBC
                out[5..9].copy_from_slice(&self.rx_id.to_le_bytes());
                out[9] = 0x02;
                out[11..27].fill(0xFF);
                out[27] = 0x01;
                out[28] = 0x80;
                out[29..37].fill(0xFF);
            }
        }

        out[37] = 0x00;
    }

    fn process_rx_packet(&mut self, rx: &[u8; RX_PACKET_SIZE]) {
        if self.mode == RadioMode::Binding {
            // Receiver responding to bind request
            if rx[0] == PACKET_BIND2 && rx[9] == 0x01 {
                let mut rid_bytes = [0u8; 4];
                rid_bytes.copy_from_slice(&rx[5..9]);
                self.rx_id = u32::from_le_bytes(rid_bytes);
                self.bind_phase = 4;
                self.bind_confirm_count = 0;
            }
            return;
        }

        // Standard downlink frames (0xAA or 0xAC)
        if rx[0] == 0xAA || rx[0] == 0xAC {
            // Verify TX ID
            let mut tx_bytes = [0u8; 4];
            tx_bytes.copy_from_slice(&rx[1..5]);
            if u32::from_le_bytes(tx_bytes) != self.tx_id {
                return;
            }

            // Verify or capture RX ID
            let mut rx_bytes = [0u8; 4];
            rx_bytes.copy_from_slice(&rx[5..9]);
            let rid = u32::from_le_bytes(rx_bytes);
            let is_placeholder = self.rx_id == 0xFFFF_FFFF
                || self.rx_id == 0
                || (self.rx_id & 0xFFFF_0000) == 0x0001_0000;
            if is_placeholder && rid != 0xFFFF_FFFF && rid != 0 {
                self.rx_id = rid;
                self.rx_id_needs_save = true;
                self.bind_done = true;
            } else if rid != self.rx_id && !is_placeholder {
                return;
            } else {
                self.bind_done = true;
            }

            // Downlink requests from receiver
            if rx[0] == 0xAA && rx[9] == 0xFC {
                // Receiver is actively requesting configuration settings!
                self.next_packet_type = PacketType::Settings;
                return;
            } else if rx[0] == 0xAA && rx[9] == 0xFD {
                // Receiver is requesting failsafe channel configuration!
                self.next_packet_type = PacketType::Failsafe;
                return;
            }

            self.telemetry.connected = true;
            self.telemetry.packets_received = self.telemetry.packets_received.wrapping_add(1);
            self.loss_counter = 0;

            // Compute signal strength (RSSI) from A7105 RSSI threshold register
            let raw_rssi = a7105::read_raw_rssi();
            let rssi_val = 256i32 - ((raw_rssi as i32 * 8) / 5);
            self.telemetry.rssi = (rssi_val.clamp(0, 100)) as u8;

            // Parse IBUS sensors (starting at byte 10, 4 bytes per sensor: [id, inst, val_lo, val_hi])
            let mut offset = 10;
            while offset + 4 <= RX_PACKET_SIZE {
                let sensor_id = rx[offset];
                let val = (rx[offset + 2] as u16) | ((rx[offset + 3] as u16) << 8);

                match sensor_id {
                    0x00 => {
                        // Internal receiver voltage in 0.01V units -> convert to mV
                        self.telemetry.rx_voltage_mv = val * 10;
                    }
                    0x03 => {
                        // External battery voltage
                        self.telemetry.rx_voltage_mv = val * 10;
                    }
                    0xFE => {
                        // Link Quality indicator (100 - error_rate)
                        let lqi = (100u16).saturating_sub(val);
                        if lqi <= 100 {
                            self.telemetry.rssi = lqi as u8;
                        }
                    }
                    _ => {}
                }

                offset += 4;
            }
        }
    }
}

/// Generate 16 spread-spectrum hopping channels from unique 32-bit TX ID.
pub fn calculate_hopping_table(tx_id: u32) -> [u8; NUM_FREQ] {
    let mut hopping = [0u8; NUM_FREQ];
    let mut idx = 0;
    let mut rnd = tx_id;
    let tx_byte3 = ((tx_id >> 24) & 0xFF) as u8;
    let mut attempts = 0u32;

    while idx < NUM_FREQ {
        attempts += 1;
        let band_no = (((idx << 1) | ((idx >> 1) & 0x01)) as u8 + tx_byte3) & 0x03;
        rnd = rnd.wrapping_mul(0x0019_660D).wrapping_add(0x3C6E_F35F);

        let next_ch = band_no * 41 + 1 + (((rnd >> idx) % 41) as u8);

        // Relax channel separation if excessive collisions occur on pathological UIDs
        let min_sep = if attempts > 500 { 1 } else if attempts > 200 { 3 } else { 5 };

        let mut valid = true;
        for &h in hopping.iter().take(idx) {
            if next_ch.abs_diff(h) < min_sep {
                valid = false;
                break;
            }
        }

        if valid {
            hopping[idx] = next_ch;
            idx += 1;
        }
    }

    hopping
}

