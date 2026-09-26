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
