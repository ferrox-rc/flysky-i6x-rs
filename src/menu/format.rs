//! Numeric and string formatters for zero-allocation menu display rendering.

const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

pub fn u32_to_hex(val: u32, buf: &mut [u8; 8]) {
    for i in 0..8 {
        buf[7 - i] = HEX_CHARS[((val >> (i * 4)) & 0x0F) as usize];
    }
}

pub fn u16_to_dec_4(val: u16, buf: &mut [u8; 4]) {
    buf[0] = b'0' + ((val / 1000) % 10) as u8;
    buf[1] = b'0' + ((val / 100) % 10) as u8;
    buf[2] = b'0' + ((val / 10) % 10) as u8;
    buf[3] = b'0' + (val % 10) as u8;
}

pub fn next_ascii(c: u8) -> u8 {
    match c {
        b' ' => b'A',
        b'A'..=b'Y' => c + 1,
        b'Z' => b'0',
        b'0'..=b'8' => c + 1,
        b'9' => b'-',
        b'-' => b'_',
        _ => b' ',
    }
}

pub fn prev_ascii(c: u8) -> u8 {
    match c {
        b' ' => b'_',
        b'_' => b'-',
        b'-' => b'9',
        b'1'..=b'9' => c - 1,
        b'0' => b'Z',
        b'B'..=b'Z' => c - 1,
        b'A' => b' ',
        _ => b' ',
    }
}

pub const SOURCE_NAMES: [&str; 26] = [
    "None", "Roll", "Pitch", "Thr", "Yaw",
    "VRA", "VRB", "SA", "SB", "SC", "SD", "MAX",
    "CH1", "CH2", "CH3", "CH4", "CH5", "CH6", "CH7",
    "CH8", "CH9", "CH10", "CH11", "CH12", "CH13", "CH14",
];

pub const SWITCH_COND_NAMES: [&str; 11] = [
    "ON", "SA^", "SAv", "SB^", "SB-", "SBv",
    "SC^", "SC-", "SCv", "SD^", "SDv",
];

pub const MODE_NAMES: [&str; 3] = ["ADD (+)", "MULT (*)", "REPL (:=)"];
pub const TEMPLATE_NAMES: [&str; 4] = ["NORMAL", "ELEVON/DELTA", "V-TAIL", "FLAPERON"];
pub const DR_SWITCH_NAMES: [&str; 5] = ["None", "SA", "SB", "SC", "SD"];
pub const AXIS_NAMES: [&str; 3] = ["Roll", "Pitch", "Yaw"];

pub fn i8_to_dec(val: i8, buf: &mut [u8; 6]) -> &str {
    let mut i = 0;
    let abs_val = if val < 0 {
        buf[i] = b'-';
        i += 1;
        (-val) as u8
    } else {
        buf[i] = b'+';
        i += 1;
        val as u8
    };
    if abs_val >= 100 {
        buf[i] = b'0' + (abs_val / 100);
        i += 1;
    }
    if abs_val >= 10 {
        buf[i] = b'0' + ((abs_val / 10) % 10);
        i += 1;
    }
    buf[i] = b'0' + (abs_val % 10);
    i += 1;
    buf[i] = b'%';
    i += 1;
    core::str::from_utf8(&buf[..i]).unwrap_or("+0%")
}

pub fn u8_to_dec(val: u8, buf: &mut [u8; 5]) -> &str {
    let mut i = 0;
    if val >= 100 {
        buf[i] = b'0' + (val / 100);
        i += 1;
    }
    if val >= 10 {
        buf[i] = b'0' + ((val / 10) % 10);
        i += 1;
    }
    buf[i] = b'0' + (val % 10);
    i += 1;
    buf[i] = b'%';
    i += 1;
    core::str::from_utf8(&buf[..i]).unwrap_or("0%")
}
