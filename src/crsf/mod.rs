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

/// Initialize CRSF subsystem hardware pins with configured PC13 power switch polarity.
pub fn init(active_high: bool) {
    uart::init(active_high);
}

/// Set external module power switch polarity (PC13).
pub fn set_power_polarity(active_high: bool) {
    uart::set_power_polarity(active_high);
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

pub const MAX_PARAMS: usize = 16;

static mut CHUNK_BUF: [u8; 96] = [0; 96];
static mut CHUNK_LEN: usize = 0;
static mut CHUNK_PARAM_ID: u8 = 0;

#[inline]
unsafe fn send_ping() {
    let mut buf = [0u8; 8];
    let len = protocol::build_ping_frame(&mut buf);
    uart::write_bytes(&buf[..len]);
}

#[inline]
unsafe fn send_param_read(target: u8, param_id: u8, chunk: u8) {
    let mut buf = [0u8; 8];
    let len = protocol::build_param_read_frame(target, param_id, chunk, &mut buf);
    uart::write_bytes(&buf[..len]);
}

#[inline]
unsafe fn send_param_write(target: u8, param_id: u8, val: u8) {
    let mut buf = [0u8; 8];
    let len = protocol::build_param_write_frame(target, param_id, val, &mut buf);
    uart::write_bytes(&buf[..len]);
}

/// Extract null-terminated string from `data` starting at `offset` into `dest`,
/// returning (offset_after_null, length_copied).
fn extract_null_string(data: &[u8], offset: usize, dest: &mut [u8]) -> (usize, u8) {
    let mut end = offset;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    let full_len = end.saturating_sub(offset);
    let copy_len = full_len.min(dest.len());
    dest[..copy_len].copy_from_slice(&data[offset..offset + copy_len]);
    let next_offset = if end < data.len() { end + 1 } else { end };
    (next_offset, copy_len as u8)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ActiveCommandState {
    Idle,
    Starting {
        param_id: u8,
        timeout_ms: u32,
    },
    WaitingConfirm {
        param_id: u8,
    },
    Running {
        param_id: u8,
        poll_timer_ms: u32,
        timeout_ms: u32,
    },
}

#[derive(Copy, Clone, Debug)]
pub struct Parameter {
    pub id: u8,
    pub parent: u8,
    pub param_type: u8,
    pub name: [u8; 16],
    pub name_len: u8,
    pub value: u8,
    pub max_value: u8,
    pub options: [u8; 48],
    pub options_len: u8,
    pub status: u8,
}

impl Parameter {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            parent: 0,
            param_type: 0,
            name: [0; 16],
            name_len: 0,
            value: 0,
            max_value: 0,
            options: [0; 48],
            options_len: 0,
            status: 0,
        }
    }

    /// Extract option string for current `value` index into buffer.
    pub fn current_option_str<'a>(&'a self, buf: &'a mut [u8; 16]) -> &'a str {
        if self.options_len == 0 {
            return "";
        }
        let opts = &self.options[..self.options_len as usize];
        let mut idx = 0u8;
        let mut start = 0usize;

        for (i, &b) in opts.iter().enumerate() {
            if b == b';' || b == 0 {
                if idx == self.value {
                    let chunk = &opts[start..i];
                    let copy_len = chunk.len().min(15);
                    buf[..copy_len].copy_from_slice(&chunk[..copy_len]);
                    return core::str::from_utf8(&buf[..copy_len]).unwrap_or("");
                }
                idx += 1;
                start = i + 1;
            }
        }
        if idx == self.value && start < opts.len() {
            let chunk = &opts[start..];
            let copy_len = chunk.len().min(15);
            buf[..copy_len].copy_from_slice(&chunk[..copy_len]);
            return core::str::from_utf8(&buf[..copy_len]).unwrap_or("");
        }
        ""
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ElrsConfigState {
    Idle,
    Discovering,      // Sending ping 0x28
    Connected,        // Received device info 0x29
    LoadingParam(u8), // Requesting param 1..param_count
    Ready,            // All parameters cached and interactive
}

pub struct ElrsConfigEngine {
    pub state: ElrsConfigState,
    pub active_cmd: ActiveCommandState,
    pub device_id: u8,
    pub device_name: [u8; 20],
    pub device_name_len: u8,
    pub param_count: u8,
    pub params: [Parameter; MAX_PARAMS],
    pub params_len: usize,
    pub last_req_ms: u32,
    pub current_chunk: u8,
}

impl ElrsConfigEngine {
    pub const fn new() -> Self {
        Self {
            state: ElrsConfigState::Idle,
            active_cmd: ActiveCommandState::Idle,
            device_id: 0, // Zeroed by default to sit in .bss
            device_name: [0; 20],
            device_name_len: 0,
            param_count: 0,
            params: [Parameter::empty(); MAX_PARAMS],
            params_len: 0,
            last_req_ms: 0,
            current_chunk: 0,
        }
    }
}

static mut CONFIG_ENGINE: ElrsConfigEngine = ElrsConfigEngine::new();

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
                        // Full frame received! Verify CRC8
                        let frame = &RX_BUF[..RX_LEN];
                        let expected_crc = frame[RX_LEN - 1];
                        let calc = protocol::crc8(&frame[2..RX_LEN - 1]);
                        if expected_crc == calc {
                            let frame_type = frame[2];
                            match frame_type {
                                protocol::CRSF_FRAMETYPE_LINK_STATISTICS
                                | protocol::CRSF_FRAMETYPE_BATTERY_SENSOR => {
                                    let _ = parse_telemetry_frame(frame, &mut TELEMETRY, now_ms);
                                }
                                protocol::CRSF_FRAMETYPE_ELRS_STATUS => {
                                    TELEMETRY.connected = true;
                                    TELEMETRY.last_telemetry_ms = now_ms;
                                }
                                protocol::CRSF_FRAMETYPE_DEVICE_INFO => {
                                    handle_device_info_frame(&frame[3..RX_LEN - 1], now_ms);
                                }
                                protocol::CRSF_FRAMETYPE_PARAMETER_SETTINGS_ENTRY => {
                                    handle_param_entry_frame(&frame[3..RX_LEN - 1], now_ms);
                                }
                                _ => {}
                            }
                        }
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

        // Service parameter configurator background retries
        elrs_tick(now_ms);
    }
}

unsafe fn handle_device_info_frame(payload: &[u8], now_ms: u32) {
    if payload.len() < 3 {
        return;
    }
    let orig = payload[1];
    CONFIG_ENGINE.device_id = orig;

    let (next_offset, name_len) = extract_null_string(payload, 2, &mut CONFIG_ENGINE.device_name);
    CONFIG_ENGINE.device_name_len = name_len;

    // Skip past serial (4B), hw (4B), fw (4B) to read param_count
    let param_count_offset = next_offset + 4 + 4 + 4;
    if param_count_offset < payload.len() {
        CONFIG_ENGINE.param_count = payload[param_count_offset];
    } else {
        CONFIG_ENGINE.param_count = 10;
    }

    if CONFIG_ENGINE.param_count > 0 {
        CONFIG_ENGINE.state = ElrsConfigState::LoadingParam(1);
        CONFIG_ENGINE.params_len = 0;
        CONFIG_ENGINE.current_chunk = 0;
        CHUNK_LEN = 0;
        CHUNK_PARAM_ID = 0;
        CONFIG_ENGINE.last_req_ms = now_ms;
        send_param_read(CONFIG_ENGINE.device_id, 1, 0);
    } else {
        CONFIG_ENGINE.state = ElrsConfigState::Ready;
        CONFIG_ENGINE.params_len = 0;
    }
}

unsafe fn handle_param_entry_frame(payload: &[u8], now_ms: u32) {
    if payload.len() < 5 {
        return;
    }
    let param_id = payload[2];
    let chunks_remain = payload[3];
    let chunk_slice = &payload[4..];

    // If starting a new parameter or chunk index 0, reset accumulator
    if CONFIG_ENGINE.current_chunk == 0 || CHUNK_PARAM_ID != param_id {
        CHUNK_LEN = 0;
        CHUNK_PARAM_ID = param_id;
    }

    // Append chunk payload to accumulator
    let avail = CHUNK_BUF.len().saturating_sub(CHUNK_LEN);
    let to_copy = chunk_slice.len().min(avail);
    CHUNK_BUF[CHUNK_LEN..CHUNK_LEN + to_copy].copy_from_slice(&chunk_slice[..to_copy]);
    CHUNK_LEN += to_copy;

    // Multi-frame parameter: request next chunk until complete
    if chunks_remain > 0 {
        CONFIG_ENGINE.current_chunk += 1;
        send_param_read(
            CONFIG_ENGINE.device_id,
            param_id,
            CONFIG_ENGINE.current_chunk,
        );
        CONFIG_ENGINE.last_req_ms = now_ms;
        return;
    }

    // Parameter stream complete! Parse reassembled payload
    let chunk = &CHUNK_BUF[..CHUNK_LEN];
    CONFIG_ENGINE.current_chunk = 0;

    if chunk.len() >= 3 {
        let parent = chunk[0];
        let p_type = chunk[1] & 0x7F;

        let mut name_buf = [0u8; 16];
        let (rest_start, name_len) = extract_null_string(chunk, 2, &mut name_buf);

        let mut opt_buf = [0u8; 48];
        let mut opt_len = 0u8;
        let mut val = 0u8;
        let mut max_val = 0u8;
        let mut status = 0u8;

        if rest_start < chunk.len() {
            if p_type == protocol::CRSF_TYPE_SELECT {
                let mut opt_end = rest_start;
                let mut opt_count = 1u8;
                while opt_end < chunk.len() && chunk[opt_end] != 0 {
                    if chunk[opt_end] == b';' {
                        opt_count += 1;
                    }
                    opt_end += 1;
                }
                let copy_len = (opt_end - rest_start).min(48);
                opt_buf[..copy_len].copy_from_slice(&chunk[rest_start..rest_start + copy_len]);
                opt_len = copy_len as u8;
                max_val = opt_count.saturating_sub(1);

                let val_pos = opt_end + 1;
                if val_pos < chunk.len() {
                    val = chunk[val_pos];
                }
            } else if p_type == protocol::CRSF_TYPE_COMMAND {
                status = chunk[rest_start];
                val = status;

                // Drive active command state transitions based on module response
                match CONFIG_ENGINE.active_cmd {
                    ActiveCommandState::Starting {
                        param_id: cmd_id, ..
                    }
                    | ActiveCommandState::Running {
                        param_id: cmd_id, ..
                    } if cmd_id == param_id => match status {
                        protocol::STATUS_CONFIRMATION_NEEDED => {
                            CONFIG_ENGINE.active_cmd =
                                ActiveCommandState::WaitingConfirm { param_id };
                        }
                        protocol::STATUS_PROGRESS => {
                            CONFIG_ENGINE.active_cmd = ActiveCommandState::Running {
                                param_id,
                                poll_timer_ms: now_ms.wrapping_add(250),
                                timeout_ms: now_ms.wrapping_add(8000),
                            };
                        }
                        protocol::STATUS_READY => {
                            CONFIG_ENGINE.active_cmd = ActiveCommandState::Idle;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        let mut found = false;
        for p in &mut CONFIG_ENGINE.params[..CONFIG_ENGINE.params_len] {
            if p.id == param_id {
                p.value = val;
                p.status = status;
                p.options = opt_buf;
                p.options_len = opt_len;
                p.max_value = max_val;
                found = true;
                break;
            }
        }
        if !found && CONFIG_ENGINE.params_len < MAX_PARAMS {
            CONFIG_ENGINE.params[CONFIG_ENGINE.params_len] = Parameter {
                id: param_id,
                parent,
                param_type: p_type,
                name: name_buf,
                name_len,
                value: val,
                max_value: max_val,
                options: opt_buf,
                options_len: opt_len,
                status,
            };
            CONFIG_ENGINE.params_len += 1;
        }
    }

    // Only advance loading sequence if engine was actively in initial loading phase
    if let ElrsConfigState::LoadingParam(loading_id) = CONFIG_ENGINE.state {
        if param_id == loading_id {
            if param_id < CONFIG_ENGINE.param_count && CONFIG_ENGINE.params_len < MAX_PARAMS {
                let next_id = param_id + 1;
                CONFIG_ENGINE.state = ElrsConfigState::LoadingParam(next_id);
                CONFIG_ENGINE.current_chunk = 0;
                CHUNK_LEN = 0;
                send_param_read(CONFIG_ENGINE.device_id, next_id, 0);
                CONFIG_ENGINE.last_req_ms = now_ms;
            } else {
                CONFIG_ENGINE.state = ElrsConfigState::Ready;
            }
        }
    }
}

unsafe fn elrs_tick(now_ms: u32) {
    match CONFIG_ENGINE.state {
        ElrsConfigState::Discovering => {
            if now_ms.wrapping_sub(CONFIG_ENGINE.last_req_ms) >= 300 {
                CONFIG_ENGINE.last_req_ms = now_ms;
                send_ping();
            }
        }
        ElrsConfigState::LoadingParam(id)
            if now_ms.wrapping_sub(CONFIG_ENGINE.last_req_ms) >= 350 =>
        {
            CONFIG_ENGINE.last_req_ms = now_ms;
            send_param_read(CONFIG_ENGINE.device_id, id, CONFIG_ENGINE.current_chunk);
        }
        _ => {}
    }

    // Process active command polling and timeouts
    match CONFIG_ENGINE.active_cmd {
        ActiveCommandState::Starting { timeout_ms, .. } => {
            if now_ms.wrapping_sub(timeout_ms) < 0x8000_0000 {
                CONFIG_ENGINE.active_cmd = ActiveCommandState::Idle;
            }
        }
        ActiveCommandState::Running {
            param_id,
            poll_timer_ms,
            timeout_ms,
        } => {
            if now_ms.wrapping_sub(timeout_ms) < 0x8000_0000 {
                CONFIG_ENGINE.active_cmd = ActiveCommandState::Idle;
            } else if now_ms.wrapping_sub(poll_timer_ms) < 0x8000_0000 {
                send_param_write(CONFIG_ENGINE.device_id, param_id, protocol::STATUS_POLL);
                CONFIG_ENGINE.active_cmd = ActiveCommandState::Running {
                    param_id,
                    poll_timer_ms: now_ms.wrapping_add(250),
                    timeout_ms,
                };
            }
        }
        _ => {}
    }
}

/// Start or refresh the native ELRS module configuration handshake.
pub fn start_config() {
    unsafe {
        CONFIG_ENGINE.device_id = protocol::CRSF_ADDRESS_CRSF_TRANSMITTER;
        CONFIG_ENGINE.state = ElrsConfigState::Discovering;
        CONFIG_ENGINE.params_len = 0;
        CONFIG_ENGINE.last_req_ms = 0;
        CONFIG_ENGINE.current_chunk = 0;
        CHUNK_LEN = 0;
        CHUNK_PARAM_ID = 0;
        CONFIG_ENGINE.active_cmd = ActiveCommandState::Idle;
        send_ping();
    }
}

/// Cycle option for a select parameter index and transmit write frame to module.
pub fn cycle_param(param_idx: usize) {
    unsafe {
        if param_idx < CONFIG_ENGINE.params_len {
            let p = &mut CONFIG_ENGINE.params[param_idx];
            if p.param_type == protocol::CRSF_TYPE_SELECT && p.max_value > 0 {
                p.value = (p.value + 1) % (p.max_value + 1);
                send_param_write(CONFIG_ENGINE.device_id, p.id, p.value);
            }
        }
    }
}

/// Trigger an action command on module: starts the command handshake with STATUS_START (1).
pub fn trigger_command(param_idx: usize) {
    unsafe {
        if param_idx < CONFIG_ENGINE.params_len {
            let p = &mut CONFIG_ENGINE.params[param_idx];
            if p.param_type == protocol::CRSF_TYPE_COMMAND
                && CONFIG_ENGINE.active_cmd == ActiveCommandState::Idle
            {
                let now = crate::time::millis();
                send_param_write(CONFIG_ENGINE.device_id, p.id, protocol::STATUS_START);
                CONFIG_ENGINE.active_cmd = ActiveCommandState::Starting {
                    param_id: p.id,
                    timeout_ms: now.wrapping_add(1500),
                };
            }
        }
    }
}

/// Confirm or cancel a command requiring pilot confirmation.
pub fn confirm_command(accept: bool) {
    unsafe {
        if let ActiveCommandState::WaitingConfirm { param_id } = CONFIG_ENGINE.active_cmd {
            let now = crate::time::millis();
            if accept {
                send_param_write(CONFIG_ENGINE.device_id, param_id, protocol::STATUS_CONFIRM);
                CONFIG_ENGINE.active_cmd = ActiveCommandState::Running {
                    param_id,
                    poll_timer_ms: now.wrapping_add(250),
                    timeout_ms: now.wrapping_add(8000),
                };
            } else {
                send_param_write(CONFIG_ENGINE.device_id, param_id, protocol::STATUS_CANCEL);
                CONFIG_ENGINE.active_cmd = ActiveCommandState::Idle;
            }
        }
    }
}

/// Get current configurator engine state and cached parameters.
pub fn get_config_engine() -> &'static ElrsConfigEngine {
    unsafe { &CONFIG_ENGINE }
}

/// Get latest decoded CRSF telemetry data.
pub fn get_telemetry() -> CrsfTelemetry {
    unsafe { TELEMETRY }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crsf::protocol::{
        crc8, CRSF_ADDRESS_CRSF_TRANSMITTER,
        CRSF_ADDRESS_RADIO_TRANSMITTER, CRSF_FRAMETYPE_DEVICE_INFO,
        CRSF_FRAMETYPE_LINK_STATISTICS, CRSF_FRAMETYPE_PARAMETER_SETTINGS_ENTRY,
        CRSF_TYPE_COMMAND, CRSF_TYPE_SELECT, STATUS_CONFIRMATION_NEEDED, STATUS_PROGRESS,
        STATUS_READY, STATUS_START,
    };
    use crate::time::set_millis;

    fn reset_state() {
        unsafe {
            CRSF_ENABLED = true;
            CURRENT_BAUD = 0;
            LAST_TX_MS = 0;
            RX_BUF = [0; 64];
            RX_LEN = 0;
            CHUNK_BUF = [0; 96];
            CHUNK_LEN = 0;
            CHUNK_PARAM_ID = 0;
            CONFIG_ENGINE = ElrsConfigEngine::new();
            TELEMETRY = CrsfTelemetry::new();
            uart::mock::clear();
        }
    }

    #[test]
    fn test_framing_garbage_rejection_and_invalid_length() {
        reset_state();

        // 1. Noise bytes before valid address must be discarded
        uart::mock::push_rx_bytes(&[0x00, 0x11, 0x22, 0x33, 0x55, 0xAA]);
        poll_telemetry(100);
        unsafe {
            assert_eq!(RX_LEN, 0, "Noise bytes should not initiate frame capture");
        }

        // 2. Sync byte followed by invalid length (< 2) must reset parser
        uart::mock::push_rx_bytes(&[CRSF_ADDRESS_RADIO_TRANSMITTER, 1, 0x14, 0x00]);
        poll_telemetry(100);
        unsafe {
            assert_eq!(RX_LEN, 0, "Frame length < 2 must be rejected");
        }

        // 3. Sync byte followed by oversized length (> 62) must reset parser
        uart::mock::push_rx_bytes(&[CRSF_ADDRESS_RADIO_TRANSMITTER, 63, 0x14]);
        poll_telemetry(100);
        unsafe {
            assert_eq!(RX_LEN, 0, "Frame length > 62 must be rejected");
        }
    }

    #[test]
    fn test_crc_failure_and_resynchronization() {
        reset_state();

        // Feed Link Statistics frame with corrupt CRC
        let bad_frame = [
            CRSF_ADDRESS_RADIO_TRANSMITTER,
            12,
            CRSF_FRAMETYPE_LINK_STATISTICS,
            80, 85, 99, 10, 0, 2, 3, 0, 0, 0,
            0x00, // Bad CRC
        ];
        uart::mock::push_rx_bytes(&bad_frame);
        poll_telemetry(1000);

        let telem = get_telemetry();
        assert!(!telem.connected, "Corrupt CRC frame must be dropped");

        // Immediately follow with valid Link Statistics frame
        let mut good_frame = bad_frame;
        good_frame[13] = crc8(&good_frame[2..13]);
        uart::mock::push_rx_bytes(&good_frame);
        poll_telemetry(1000);

        let telem_ok = get_telemetry();
        assert!(telem_ok.connected, "Parser must resync and accept subsequent valid frame");
        assert_eq!(telem_ok.uplink_link_quality, 99);
        assert_eq!(telem_ok.tx_power_mw, 100);
    }

    #[test]
    fn test_discovery_handshake_and_device_info() {
        reset_state();
        set_millis(1000);

        // 1. Start config handshake
        start_config();
        let engine = get_config_engine();
        assert_eq!(engine.state, ElrsConfigState::Discovering);

        // Verify Ping packet transmitted
        let tx = uart::mock::take_tx();
        assert_eq!(tx.len(), 1, "start_config must transmit ping packet");
        assert_eq!(tx[0][0], CRSF_ADDRESS_CRSF_TRANSMITTER);
        assert_eq!(tx[0][2], protocol::CRSF_FRAMETYPE_DEVICE_PING);

        // 2. Feed Device Info response frame (0x29)
        // Frame: [addr=0xEA, len, type=0x29, dest=0xEA, orig=0xEE, "ELRS 2.4G\0", serial(4), hw(4), fw(4), count=3, ver=1, crc]
        let mut frame = [0u8; 32];
        frame[0] = CRSF_ADDRESS_RADIO_TRANSMITTER; // 0xEA
        frame[2] = CRSF_FRAMETYPE_DEVICE_INFO;     // 0x29
        frame[3] = CRSF_ADDRESS_RADIO_TRANSMITTER; // 0xEA
        frame[4] = CRSF_ADDRESS_CRSF_TRANSMITTER;  // 0xEE (device_id)

        // Device name: "ELRS 2.4G\0" (10 bytes)
        let name = b"ELRS 2.4G\0";
        frame[5..5 + name.len()].copy_from_slice(name);
        let serial_pos = 5 + name.len();
        // Serial (4), HW (4), FW (4) -> 12 bytes
        let count_pos = serial_pos + 12;
        frame[count_pos] = 3; // 3 parameters to load
        frame[count_pos + 1] = 1; // version

        let total_size = count_pos + 3;
        frame[1] = (total_size - 2) as u8;
        frame[total_size - 1] = crc8(&frame[2..total_size - 1]);

        uart::mock::push_rx_bytes(&frame[..total_size]);
        poll_telemetry(1100);

        let engine = get_config_engine();
        assert_eq!(engine.device_id, CRSF_ADDRESS_CRSF_TRANSMITTER);
        assert_eq!(&engine.device_name[..9], b"ELRS 2.4G");
        assert_eq!(engine.param_count, 3);
        assert_eq!(engine.state, ElrsConfigState::LoadingParam(1));

        // Handset must have transmitted Parameter Read for Param 1, Chunk 0
        let tx = uart::mock::take_tx();
        assert_eq!(tx.len(), 1, "Must request Param 1 Chunk 0 after Device Info");
        assert_eq!(tx[0][2], protocol::CRSF_FRAMETYPE_PARAMETER_READ);
        assert_eq!(tx[0][5], 1, "Requested param_id must be 1");
        assert_eq!(tx[0][6], 0, "Requested chunk must be 0");
    }

    #[test]
    fn test_device_info_zero_params_transitions_to_ready() {
        reset_state();
        start_config();
        uart::mock::clear();

        // Feed Device Info frame with param_count = 0
        let mut frame = [0u8; 24];
        frame[0] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        frame[1] = 22;
        frame[2] = CRSF_FRAMETYPE_DEVICE_INFO;
        frame[3] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        frame[4] = CRSF_ADDRESS_CRSF_TRANSMITTER;
        frame[5..10].copy_from_slice(b"None\0");
        // Count pos: 10 + 12 = 22
        frame[22] = 0; // 0 params
        frame[23] = crc8(&frame[2..23]);

        uart::mock::push_rx_bytes(&frame);
        poll_telemetry(1200);

        let engine = get_config_engine();
        assert_eq!(engine.state, ElrsConfigState::Ready, "0 parameters must transition directly to Ready");
    }

    #[test]
    fn test_param_select_parsing_and_sequential_loading() {
        reset_state();
        // Setup engine expecting Param 1 of 2
        unsafe {
            CONFIG_ENGINE.device_id = CRSF_ADDRESS_CRSF_TRANSMITTER;
            CONFIG_ENGINE.param_count = 2;
            CONFIG_ENGINE.state = ElrsConfigState::LoadingParam(1);
        }

        // Construct Param 1 Entry: SELECT type, Name="Pkt Rate\0", Options="50Hz;150Hz;250Hz;500Hz\0", Value=2
        let mut frame = [0u8; 64];
        frame[0] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        frame[2] = CRSF_FRAMETYPE_PARAMETER_SETTINGS_ENTRY;
        frame[3] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        frame[4] = CRSF_ADDRESS_CRSF_TRANSMITTER;
        frame[5] = 1; // param_id
        frame[6] = 0; // chunks_remain
        frame[7] = 0; // parent
        frame[8] = CRSF_TYPE_SELECT; // 9

        let mut pos = 9;
        let name = b"Pkt Rate\0";
        frame[pos..pos + name.len()].copy_from_slice(name);
        pos += name.len();

        let opts = b"50Hz;150Hz;250Hz;500Hz\0";
        frame[pos..pos + opts.len()].copy_from_slice(opts);
        pos += opts.len();

        frame[pos] = 2; // Value = 2 (250Hz)
        pos += 1;

        let frame_len = pos - 1;
        frame[1] = frame_len as u8;
        frame[pos] = crc8(&frame[2..pos]);
        let total = pos + 1;

        uart::mock::push_rx_bytes(&frame[..total]);
        poll_telemetry(2000);

        let engine = get_config_engine();
        assert_eq!(engine.params_len, 1);
        let p = &engine.params[0];
        assert_eq!(p.id, 1);
        assert_eq!(p.param_type, CRSF_TYPE_SELECT);
        assert_eq!(&p.name[..8], b"Pkt Rate");
        assert_eq!(p.value, 2);
        assert_eq!(p.max_value, 3); // 4 options -> max_value = 3
        let mut buf = [0u8; 16];
        assert_eq!(p.current_option_str(&mut buf), "250Hz");

        // Advanced to loading Param 2
        assert_eq!(engine.state, ElrsConfigState::LoadingParam(2));
        let tx = uart::mock::take_tx();
        assert_eq!(tx.len(), 1, "Must request Param 2 Chunk 0");
        assert_eq!(tx[0][5], 2);
    }

    #[test]
    fn test_param_multi_frame_chunk_reassembly() {
        reset_state();
        unsafe {
            CONFIG_ENGINE.device_id = CRSF_ADDRESS_CRSF_TRANSMITTER;
            CONFIG_ENGINE.param_count = 1;
            CONFIG_ENGINE.state = ElrsConfigState::LoadingParam(1);
        }

        // Chunk 0: chunks_remain = 1
        let mut chunk0 = [0u8; 32];
        chunk0[0] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        chunk0[2] = CRSF_FRAMETYPE_PARAMETER_SETTINGS_ENTRY;
        chunk0[3] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        chunk0[4] = CRSF_ADDRESS_CRSF_TRANSMITTER;
        chunk0[5] = 1; // param_id
        chunk0[6] = 1; // chunks_remain = 1!
        chunk0[7] = 0; // parent
        chunk0[8] = CRSF_TYPE_SELECT; // type
        chunk0[9..13].copy_from_slice(b"Pwr\0");
        chunk0[13..23].copy_from_slice(b"10mW;25mW;"); // First half of options
        let total0 = 24;
        chunk0[1] = (total0 - 2) as u8;
        chunk0[total0 - 1] = crc8(&chunk0[2..total0 - 1]);

        uart::mock::push_rx_bytes(&chunk0[..total0]);
        poll_telemetry(3000);

        let engine = get_config_engine();
        assert_eq!(engine.current_chunk, 1, "Must advance to chunk 1");
        assert_eq!(engine.params_len, 0, "Parameter not parsed until final chunk");

        // Verify request for Chunk 1 sent
        let tx = uart::mock::take_tx();
        assert_eq!(tx.len(), 1);
        assert_eq!(tx[0][5], 1); // param_id 1
        assert_eq!(tx[0][6], 1); // chunk 1

        // Chunk 1: chunks_remain = 0 (final chunk)
        let mut chunk1 = [0u8; 32];
        chunk1[0] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        chunk1[2] = CRSF_FRAMETYPE_PARAMETER_SETTINGS_ENTRY;
        chunk1[3] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        chunk1[4] = CRSF_ADDRESS_CRSF_TRANSMITTER;
        chunk1[5] = 1;
        chunk1[6] = 0; // chunks_remain = 0!
        chunk1[7..19].copy_from_slice(b"100mW;250mW\0"); // Second half of options
        chunk1[19] = 3; // value = 3 (250mW)
        let total1 = 21;
        chunk1[1] = (total1 - 2) as u8;
        chunk1[total1 - 1] = crc8(&chunk1[2..total1 - 1]);

        uart::mock::push_rx_bytes(&chunk1[..total1]);
        poll_telemetry(3100);

        let engine = get_config_engine();
        assert_eq!(engine.params_len, 1, "Reassembled parameter must be stored");
        let p = &engine.params[0];
        assert_eq!(&p.name[..3], b"Pwr");
        assert_eq!(p.value, 3);
        assert_eq!(p.max_value, 3); // 4 options total: 10mW, 25mW, 100mW, 250mW
        let mut buf = [0u8; 16];
        assert_eq!(p.current_option_str(&mut buf), "250mW");

        // Since param_count was 1, state must transition to Ready
        assert_eq!(engine.state, ElrsConfigState::Ready);
    }

    #[test]
    fn test_command_action_lifecycle_and_confirmations() {
        reset_state();
        set_millis(5000);

        // Setup ready engine with 1 command param
        unsafe {
            CONFIG_ENGINE.device_id = CRSF_ADDRESS_CRSF_TRANSMITTER;
            CONFIG_ENGINE.state = ElrsConfigState::Ready;
            let mut name = [0u8; 16];
            name[..4].copy_from_slice(b"Bind");
            CONFIG_ENGINE.params[0] = Parameter {
                id: 1,
                parent: 0,
                param_type: CRSF_TYPE_COMMAND,
                name,
                name_len: 4,
                value: STATUS_READY,
                max_value: 0,
                options: [0; 48],
                options_len: 0,
                status: STATUS_READY,
            };
            CONFIG_ENGINE.params_len = 1;
        }

        // 1. Trigger command
        trigger_command(0);
        let engine = get_config_engine();
        assert!(matches!(engine.active_cmd, ActiveCommandState::Starting { param_id: 1, .. }));
        let tx = uart::mock::take_tx();
        assert_eq!(tx.len(), 1);
        assert_eq!(tx[0][2], protocol::CRSF_FRAMETYPE_PARAMETER_WRITE);
        assert_eq!(tx[0][6], STATUS_START);

        // 2. Module responds requiring confirmation (STATUS_CONFIRMATION_NEEDED = 3)
        // Payload has: parent(0), type(CRSF_TYPE_COMMAND), "Bind\0", status
        let mut frame = [0u8; 20];
        frame[0] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        frame[2] = CRSF_FRAMETYPE_PARAMETER_SETTINGS_ENTRY;
        frame[3] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        frame[4] = CRSF_ADDRESS_CRSF_TRANSMITTER;
        frame[5] = 1; // param_id
        frame[6] = 0; // chunks_remain
        frame[7] = 0; // parent
        frame[8] = CRSF_TYPE_COMMAND;
        frame[9..14].copy_from_slice(b"Bind\0");
        frame[14] = STATUS_CONFIRMATION_NEEDED; // status
        let total = 16;
        frame[1] = (total - 2) as u8;
        frame[total - 1] = crc8(&frame[2..total - 1]);

        uart::mock::push_rx_bytes(&frame[..total]);
        poll_telemetry(5100);

        let engine = get_config_engine();
        assert_eq!(engine.active_cmd, ActiveCommandState::WaitingConfirm { param_id: 1 });

        // 3. User accepts confirmation
        confirm_command(true);
        let engine = get_config_engine();
        assert!(matches!(engine.active_cmd, ActiveCommandState::Running { param_id: 1, .. }));
        let tx = uart::mock::take_tx();
        assert_eq!(tx.len(), 1);
        assert_eq!(tx[0][6], protocol::STATUS_CONFIRM);

        // 4. Module responds with STATUS_PROGRESS (2)
        frame[14] = STATUS_PROGRESS;
        frame[total - 1] = crc8(&frame[2..total - 1]);
        uart::mock::push_rx_bytes(&frame[..total]);
        poll_telemetry(5200);

        let engine = get_config_engine();
        assert!(matches!(engine.active_cmd, ActiveCommandState::Running { param_id: 1, .. }));

        // 5. Module completes with STATUS_READY (0)
        frame[14] = STATUS_READY;
        frame[total - 1] = crc8(&frame[2..total - 1]);
        uart::mock::push_rx_bytes(&frame[..total]);
        poll_telemetry(5300);

        let engine = get_config_engine();
        assert_eq!(engine.active_cmd, ActiveCommandState::Idle, "Completed command returns to Idle");
    }

    #[test]
    fn test_telemetry_disconnect_timeout() {
        reset_state();

        // Feed link statistics at t = 1000
        let mut frame = [0u8; 14];
        frame[0] = CRSF_ADDRESS_RADIO_TRANSMITTER;
        frame[1] = 12;
        frame[2] = CRSF_FRAMETYPE_LINK_STATISTICS;
        frame[3] = 80;
        frame[4] = 85;
        frame[5] = 99;
        frame[6] = 10;
        frame[7] = 0;
        frame[8] = 2;
        frame[9] = 3;
        frame[13] = crc8(&frame[2..13]);

        uart::mock::push_rx_bytes(&frame);
        poll_telemetry(1000);
        assert!(get_telemetry().connected);

        // Poll at t = 1500 (500 ms later): still connected
        poll_telemetry(1500);
        assert!(get_telemetry().connected);

        // Poll at t = 2050 (>1000 ms timeout): disconnected!
        poll_telemetry(2050);
        assert!(!get_telemetry().connected, "Must mark disconnected after 1000 ms silence");
    }
}
