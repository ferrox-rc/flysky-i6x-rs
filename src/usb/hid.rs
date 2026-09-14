//! USB HID Gamepad / Joystick driver for FlySky FS-i6X.
//!
//! Exposes an 8-axis, 16-button standard USB Gamepad compatible with all FPV
//! and RC flight simulators (Liftoff, Velocidrone, RealFlight, AccuRC, DCL, etc.).

use crate::input::{SwitchPos, Switches};

/// USB HID Report Descriptor for an 8-Axis, 16-Button Gamepad (18 bytes input report).
/// Matches OpenI6X and EdgeTX 11-bit simulator conventions.
pub const GAMEPAD_REPORT_DESC: &[u8] = &[
    0x05, 0x01,        // Usage Page (Generic Desktop Ctrls)
    0x09, 0x05,        // Usage (Game Pad)
    0xA1, 0x01,        // Collection (Application)
    0xA1, 0x00,        //   Collection (Physical)
    0x05, 0x09,        //     Usage Page (Button)
    0x19, 0x01,        //     Usage Minimum (Button 1)
    0x29, 0x10,        //     Usage Maximum (Button 16)
    0x15, 0x00,        //     Logical Minimum (0)
    0x25, 0x01,        //     Logical Maximum (1)
    0x95, 0x10,        //     Report Count (16)
    0x75, 0x01,        //     Report Size (1)
    0x81, 0x02,        //     Input (Data,Var,Abs) - 2 bytes buttons
    0x05, 0x01,        //     Usage Page (Generic Desktop Ctrls)
    0x09, 0x30,        //     Usage (X)
    0x09, 0x31,        //     Usage (Y)
    0x09, 0x32,        //     Usage (Z)
    0x09, 0x33,        //     Usage (Rx)
    0x09, 0x34,        //     Usage (Ry)
    0x09, 0x35,        //     Usage (Rz)
    0x09, 0x36,        //     Usage (Slider)
    0x09, 0x37,        //     Usage (Slider)
    0x16, 0x00, 0x00,  //     Logical Minimum (0)
    0x26, 0xFF, 0x07,  //     Logical Maximum (2047) - 11-bit resolution
    0x75, 0x10,        //     Report Size (16)
    0x95, 0x08,        //     Report Count (8)
    0x81, 0x02,        //     Input (Data,Var,Abs) - 8 x 16-bit axes = 16 bytes
    0xC0,              //   End Collection
    0xC0,              // End Collection
];

pub const REPORT_SIZE: usize = 18;

/// Map a microsecond channel pulse width (1000..2000 µs) to 11-bit USB axis value (0..2047).
#[inline(always)]
fn us_to_axis(us: u16) -> u16 {
    let clamped = us.clamp(1000, 2000) as u32;
    (((clamped - 1000) * 2047) / 1000) as u16
}

/// Build standard 18-byte HID Gamepad report from live flight mixer channels and physical switches.
pub fn build_gamepad_report(
    rf_chs: &[u16; 14],
    switches: &Switches,
    out: &mut [u8; REPORT_SIZE],
) {
    // 1. Digital Buttons (16 bits)
    let mut btns = 0u16;

    // SA (2-pos): Up = 0, Down = Button 1
    if switches.sa == SwitchPos::Down {
        btns |= 1 << 0;
    }

    // SB (3-pos): Up = Button 2, Mid = Button 3, Down = Button 4
    match switches.sb {
        SwitchPos::Up => btns |= 1 << 1,
        SwitchPos::Mid => btns |= 1 << 2,
        SwitchPos::Down => btns |= 1 << 3,
    }

    // SC (3-pos): Up = Button 5, Mid = Button 6, Down = Button 7
    match switches.sc {
        SwitchPos::Up => btns |= 1 << 4,
        SwitchPos::Mid => btns |= 1 << 5,
        SwitchPos::Down => btns |= 1 << 6,
    }

    // SD (2-pos): Up = 0, Down = Button 8
    if switches.sd == SwitchPos::Down {
        btns |= 1 << 7;
    }

    // Aux Channels 9..14 mapped to Buttons 9..14 (active high if > 1500 µs)
    for ch in 8..14 {
        if rf_chs[ch] > 1500 {
            btns |= 1 << (ch - 8 + 8);
        }
    }

    out[0] = (btns & 0xFF) as u8;
    out[1] = ((btns >> 8) & 0xFF) as u8;

    // 2. Analog Axes (8 x 16-bit, little endian)
    // CH1: Roll / Aileron -> X
    let a1 = us_to_axis(rf_chs[0]);
    out[2] = (a1 & 0xFF) as u8;
    out[3] = ((a1 >> 8) & 0xFF) as u8;

    // CH2: Pitch / Elevator -> Y
    let a2 = us_to_axis(rf_chs[1]);
    out[4] = (a2 & 0xFF) as u8;
    out[5] = ((a2 >> 8) & 0xFF) as u8;

    // CH3: Throttle -> Z
    let a3 = us_to_axis(rf_chs[2]);
    out[6] = (a3 & 0xFF) as u8;
    out[7] = ((a3 >> 8) & 0xFF) as u8;

    // CH4: Yaw / Rudder -> Rx
    let a4 = us_to_axis(rf_chs[3]);
    out[8] = (a4 & 0xFF) as u8;
    out[9] = ((a4 >> 8) & 0xFF) as u8;

    // CH5: Potentiometer VRA -> Ry
    let a5 = us_to_axis(rf_chs[4]);
    out[10] = (a5 & 0xFF) as u8;
    out[11] = ((a5 >> 8) & 0xFF) as u8;

    // CH6: Potentiometer VRB -> Rz
    let a6 = us_to_axis(rf_chs[5]);
    out[12] = (a6 & 0xFF) as u8;
    out[13] = ((a6 >> 8) & 0xFF) as u8;

    // CH7: Aux 7 -> Slider 1
    let a7 = us_to_axis(rf_chs[6]);
    out[14] = (a7 & 0xFF) as u8;
    out[15] = ((a7 >> 8) & 0xFF) as u8;

    // CH8: Aux 8 -> Slider 2
    let a8 = us_to_axis(rf_chs[7]);
    out[16] = (a8 & 0xFF) as u8;
    out[17] = ((a8 >> 8) & 0xFF) as u8;
}
