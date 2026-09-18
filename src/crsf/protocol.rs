//! CRSF / ExpressLRS protocol framing, channel encoding, and telemetry parser.
//!
//! Conforms to TBS Crossfire Protocol Specification Rev 08 & ExpressLRS implementation.

#![allow(dead_code)]

pub const CRSF_ADDRESS_CRSF_TRANSMITTER: u8 = 0xEE;
pub const CRSF_ADDRESS_RADIO_TRANSMITTER: u8 = 0xEA;
pub const CRSF_ADDRESS_FLIGHT_CONTROLLER: u8 = 0xC8;

// Frame types
pub const CRSF_FRAMETYPE_GPS: u8 = 0x02;
pub const CRSF_FRAMETYPE_BATTERY_SENSOR: u8 = 0x08;
pub const CRSF_FRAMETYPE_HEARTBEAT: u8 = 0x0B;
pub const CRSF_FRAMETYPE_LINK_STATISTICS: u8 = 0x14;
pub const CRSF_FRAMETYPE_RC_CHANNELS_PACKED: u8 = 0x16;
pub const CRSF_FRAMETYPE_DEVICE_PING: u8 = 0x28;
pub const CRSF_FRAMETYPE_DEVICE_INFO: u8 = 0x29;

pub const CRSF_FRAME_MAX_SIZE: usize = 64;
pub const CRSF_RC_FRAME_SIZE: usize = 26; // 1 (addr) + 1 (len) + 1 (type) + 22 (payload) + 1 (crc)

/// Incoming telemetry data decoded from CRSF link
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CrsfTelemetry {
    pub connected: bool,
    pub uplink_rssi_1: i8,      // dBm (-130..0)
    pub uplink_rssi_2: i8,      // dBm (-130..0)
    pub uplink_link_quality: u8,// 0..100%
    pub uplink_snr: i8,         // dB (-30..30)
    pub active_antenna: u8,     // 0 or 1
    pub rf_mode: u8,            // ExpressLRS / CRSF rate mode index
    pub tx_power_mw: u16,       // Transmit power in milliwatts
    pub rx_battery_mv: u16,     // Flight pack voltage in millivolts
    pub rx_current_ma: u16,     // Current in 100mA units
    pub rx_capacity_mah: u32,   // Capacity consumed in mAh
    pub rx_battery_pct: u8,     // Battery remaining percentage (0..100)
    pub last_telemetry_ms: u32, // System tick when last valid frame was received
}

impl CrsfTelemetry {
    pub const fn new() -> Self {
        Self {
            connected: false,
            uplink_rssi_1: 0,
            uplink_rssi_2: 0,
            uplink_link_quality: 0,
            uplink_snr: 0,
            active_antenna: 0,
            rf_mode: 0,
            tx_power_mw: 0,
            rx_battery_mv: 0,
            rx_current_ma: 0,
            rx_capacity_mah: 0,
            rx_battery_pct: 0,
            last_telemetry_ms: 0,
        }
    }
}

/// CRC8 lookup table using polynomial 0xD5 (DVB-S2).
const CRC8_TABLE: [u8; 256] = [
    0x00, 0xD5, 0x7F, 0xAA, 0xFE, 0x2B, 0x81, 0x54, 0x29, 0xFC, 0x56, 0x83, 0xD7, 0x02, 0xA8, 0x7D,
    0x52, 0x87, 0x2D, 0xF8, 0xAC, 0x79, 0xD3, 0x06, 0x7B, 0xAE, 0x04, 0xD1, 0x85, 0x50, 0xFA, 0x2F,
    0xA4, 0x71, 0xDB, 0x0E, 0x5A, 0x8F, 0x25, 0xF0, 0x8D, 0x58, 0xF2, 0x27, 0x73, 0xA6, 0x0C, 0xD9,
    0xF6, 0x23, 0x89, 0x5C, 0x08, 0xDD, 0x77, 0xA2, 0xDF, 0x0A, 0xA0, 0x75, 0x21, 0xF4, 0x5E, 0x8B,
    0x9D, 0x48, 0xE2, 0x37, 0x63, 0xB6, 0x1C, 0xC9, 0xB4, 0x61, 0xCB, 0x1E, 0x4A, 0x9F, 0x35, 0xE0,
    0xCF, 0x1A, 0xB0, 0x65, 0x31, 0xE4, 0x4E, 0x9B, 0xE6, 0x33, 0x99, 0x4C, 0x18, 0xCD, 0x67, 0xB2,
    0x39, 0xEC, 0x46, 0x93, 0xC7, 0x12, 0xB8, 0x6D, 0x10, 0xC5, 0x6F, 0xBA, 0xEE, 0x3B, 0x91, 0x44,
    0x6B, 0xBE, 0x14, 0xC1, 0x95, 0x40, 0xEA, 0x3F, 0x42, 0x97, 0x3D, 0xE8, 0xBC, 0x69, 0xC3, 0x16,
    0xEF, 0x3A, 0x90, 0x45, 0x11, 0xC4, 0x6E, 0xBB, 0xC6, 0x13, 0xB9, 0x6C, 0x38, 0xED, 0x47, 0x92,
    0xBD, 0x68, 0xC2, 0x17, 0x43, 0x96, 0x3C, 0xE9, 0x94, 0x41, 0xEB, 0x3E, 0x6A, 0xBF, 0x15, 0xC0,
    0x4B, 0x9E, 0x34, 0xE1, 0xB5, 0x60, 0xCA, 0x1F, 0x62, 0xB7, 0x1D, 0xC8, 0x9C, 0x49, 0xE3, 0x36,
    0x19, 0xCC, 0x66, 0xB3, 0xE7, 0x32, 0x98, 0x4D, 0x30, 0xE5, 0x4F, 0x9A, 0xCE, 0x1B, 0xB1, 0x64,
    0x72, 0xA7, 0x0D, 0xD8, 0x8C, 0x59, 0xF3, 0x26, 0x5B, 0x8E, 0x24, 0xF1, 0xA5, 0x70, 0xDA, 0x0F,
    0x20, 0xF5, 0x5F, 0x8A, 0xDE, 0x0B, 0xA1, 0x74, 0x09, 0xDC, 0x76, 0xA3, 0xF7, 0x22, 0x88, 0x5D,
    0xD6, 0x03, 0xA9, 0x7C, 0x28, 0xFD, 0x57, 0x82, 0xFF, 0x2A, 0x80, 0x55, 0x01, 0xD4, 0x7E, 0xAB,
    0x84, 0x51, 0xFB, 0x2E, 0x7A, 0xAF, 0x05, 0xD0, 0xAD, 0x78, 0xD2, 0x07, 0x53, 0x86, 0x2C, 0xF9,
];

/// Calculate CRC8-DVB over a slice of bytes.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc = CRC8_TABLE[(crc ^ b) as usize];
    }
    crc
}

/// Convert a microsecond pulse width (1000..2000 µs, center 1500)
/// into standard 11-bit CRSF channel counts (0..2047, where 1000µs = 172, 1500µs = 992, 2000µs = 1811).
#[inline(always)]
pub fn us_to_crsf(us: u16) -> u16 {
    let clamped = us.clamp(988, 2012) as i32;
    // CRSF standard formula: ((us - 1000) * 1600 / 1000) + 172
    // = ((us - 1000) * 8 / 5) + 172
    let val = (((clamped - 1000) * 8) / 5) + 172;
    val.clamp(0, 2047) as u16
}

/// Build a packed 16-channel CRSF RC packet (Type 0x16) into `out_frame`.
/// Returns the number of bytes written (always 26 bytes).
pub fn build_channels_frame(channels: &[u16; 14], out_frame: &mut [u8; CRSF_RC_FRAME_SIZE]) -> usize {
    // Header
    out_frame[0] = CRSF_ADDRESS_CRSF_TRANSMITTER; // 0xEE
    out_frame[1] = 24;                            // Length: Type (1) + Payload (22) + CRC (1) = 24
    out_frame[2] = CRSF_FRAMETYPE_RC_CHANNELS_PACKED; // 0x16

    // Convert 14 radio channels to 16 CRSF 11-bit values (fill channels 15 & 16 with neutral 992)
    let mut ch11 = [992u16; 16];
    for i in 0..14 {
        ch11[i] = us_to_crsf(channels[i]);
    }

    // 16 channels * 11 bits = 176 bits = 22 bytes
    let p = &mut out_frame[3..25];
    p[0] = (ch11[0] & 0x07FF) as u8;
    p[1] = ((ch11[0] >> 8) | (ch11[1] << 3)) as u8;
    p[2] = ((ch11[1] >> 5) | (ch11[2] << 6)) as u8;
    p[3] = (ch11[2] >> 2) as u8;
    p[4] = ((ch11[2] >> 10) | (ch11[3] << 1)) as u8;
    p[5] = ((ch11[3] >> 7) | (ch11[4] << 4)) as u8;
    p[6] = ((ch11[4] >> 4) | (ch11[5] << 7)) as u8;
    p[7] = (ch11[5] >> 1) as u8;
    p[8] = ((ch11[5] >> 9) | (ch11[6] << 2)) as u8;
    p[9] = ((ch11[6] >> 6) | (ch11[7] << 5)) as u8;
    p[10] = (ch11[7] >> 3) as u8;
    p[11] = (ch11[8] & 0x07FF) as u8;
    p[12] = ((ch11[8] >> 8) | (ch11[9] << 3)) as u8;
    p[13] = ((ch11[9] >> 5) | (ch11[10] << 6)) as u8;
    p[14] = (ch11[10] >> 2) as u8;
    p[15] = ((ch11[10] >> 10) | (ch11[11] << 1)) as u8;
    p[16] = ((ch11[11] >> 7) | (ch11[12] << 4)) as u8;
    p[17] = ((ch11[12] >> 4) | (ch11[13] << 7)) as u8;
    p[18] = (ch11[13] >> 1) as u8;
    p[19] = ((ch11[13] >> 9) | (ch11[14] << 2)) as u8;
    p[20] = ((ch11[14] >> 6) | (ch11[15] << 5)) as u8;
    p[21] = (ch11[15] >> 3) as u8;

    // CRC8 is calculated over Type (byte 2) and Payload (bytes 3..25)
    out_frame[25] = crc8(&out_frame[2..25]);

    CRSF_RC_FRAME_SIZE
}

/// Parse incoming telemetry frame and update telemetry state.
pub fn parse_telemetry_frame(frame: &[u8], telem: &mut CrsfTelemetry, now_ms: u32) -> bool {
    if frame.len() < 4 {
        return false;
    }
    let frame_len = frame[1] as usize;
    if frame_len + 2 > frame.len() || frame_len < 2 {
        return false;
    }

    // Check CRC
    let expected_crc = frame[1 + frame_len];
    let calc = crc8(&frame[2..1 + frame_len]);
    if expected_crc != calc {
        return false;
    }

    let frame_type = frame[2];
    let payload = &frame[3..1 + frame_len];

    match frame_type {
        CRSF_FRAMETYPE_LINK_STATISTICS => {
            if payload.len() >= 10 {
                telem.uplink_rssi_1 = -(payload[0] as i8);
                telem.uplink_rssi_2 = -(payload[1] as i8);
                telem.uplink_link_quality = payload[2];
                telem.uplink_snr = payload[3] as i8;
                telem.active_antenna = payload[4];
                telem.rf_mode = payload[5];
                let pwr_code = payload[6];
                telem.tx_power_mw = match pwr_code {
                    0 => 0,
                    1 => 10,
                    2 => 25,
                    3 => 100,
                    4 => 500,
                    5 => 1000,
                    6 => 2000,
                    7 => 250,
                    8 => 50,
                    _ => 0,
                };
                telem.connected = true;
                telem.last_telemetry_ms = now_ms;
                return true;
            }
        }
        CRSF_FRAMETYPE_BATTERY_SENSOR if payload.len() >= 8 => {
            // Voltage in 0.1V units (big endian)
            let v_deci = u16::from_be_bytes([payload[0], payload[1]]);
            telem.rx_battery_mv = v_deci * 100;
            let c_deci = u16::from_be_bytes([payload[2], payload[3]]);
            telem.rx_current_ma = c_deci * 100;
            let cap = ((payload[4] as u32) << 16) | ((payload[5] as u32) << 8) | (payload[6] as u32);
            telem.rx_capacity_mah = cap;
            telem.rx_battery_pct = payload[7];
            telem.connected = true;
            telem.last_telemetry_ms = now_ms;
            return true;
        }
        _ => {}
    }

    false
}
