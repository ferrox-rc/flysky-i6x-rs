#![no_std]
#![no_main]

use panic_halt as _;
use stm32f0xx_hal as _;

use cortex_m_rt::entry;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};

mod adc;
mod boot;
mod chip;
mod display;
mod input;

use display::St7567;

const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

fn u16_to_hex(val: u16, buf: &mut [u8; 4]) {
    buf[0] = HEX_CHARS[((val >> 12) & 0x0F) as usize];
    buf[1] = HEX_CHARS[((val >> 8) & 0x0F) as usize];
    buf[2] = HEX_CHARS[((val >> 4) & 0x0F) as usize];
    buf[3] = HEX_CHARS[(val & 0x0F) as usize];
}

/// Format voltage in mV to "X.YYV" or "XX.YYV"
fn format_vbat(mv: u16, buf: &mut [u8; 6]) -> &str {
    let v = mv / 1000;
    let rem = mv % 1000;
    let d1 = rem / 100;
    let d2 = (rem % 100) / 10;

    if v >= 10 {
        buf[0] = b'0' + ((v / 10) as u8);
        buf[1] = b'0' + ((v % 10) as u8);
        buf[2] = b'.';
        buf[3] = b'0' + (d1 as u8);
        buf[4] = b'0' + (d2 as u8);
        buf[5] = b'V';
        core::str::from_utf8(buf).unwrap_or("0.00V")
    } else {
        buf[0] = b'0' + (v as u8);
        buf[1] = b'.';
        buf[2] = b'0' + (d1 as u8);
        buf[3] = b'0' + (d2 as u8);
        buf[4] = b'V';
        core::str::from_utf8(&buf[0..5]).unwrap_or("0.00V")
    }
}

/// Format stick value (-1000..+1000) to percentage string "-100%" .. "+100%"
fn format_percent(val: i16, buf: &mut [u8; 5]) -> &str {
    let pct = val / 10; // -100 .. +100
    let abs_pct = pct.unsigned_abs();

    if pct < 0 {
        buf[0] = b'-';
    } else if pct > 0 {
        buf[0] = b'+';
    } else {
        buf[0] = b' ';
    }

    if abs_pct >= 100 {
        buf[1] = b'1';
        buf[2] = b'0';
        buf[3] = b'0';
        buf[4] = b'%';
        core::str::from_utf8(buf).unwrap_or(" 0%")
    } else if abs_pct >= 10 {
        buf[1] = b' ';
        buf[2] = b'0' + ((abs_pct / 10) as u8);
        buf[3] = b'0' + ((abs_pct % 10) as u8);
        buf[4] = b'%';
        core::str::from_utf8(&buf[0..5]).unwrap_or(" 0%")
    } else {
        buf[1] = b' ';
        buf[2] = b' ';
        buf[3] = b'0' + (abs_pct as u8);
        buf[4] = b'%';
        core::str::from_utf8(&buf[0..5]).unwrap_or(" 0%")
    }
}

/// Format throttle value (-1000..+1000) to unipolar percentage string "  0%" .. "100%"
fn format_throttle_percent(val: i16, buf: &mut [u8; 5]) -> &str {
    let pct = (((val as i32 + 1000) * 100) / 2000).clamp(0, 100) as u8;

    if pct >= 100 {
        buf[0] = b'1';
        buf[1] = b'0';
        buf[2] = b'0';
        buf[3] = b'%';
        core::str::from_utf8(&buf[0..4]).unwrap_or("100%")
    } else if pct >= 10 {
        buf[0] = b' ';
        buf[1] = b'0' + (pct / 10);
        buf[2] = b'0' + (pct % 10);
        buf[3] = b'%';
        core::str::from_utf8(&buf[0..4]).unwrap_or(" 0%")
    } else {
        buf[0] = b' ';
        buf[1] = b' ';
        buf[2] = b'0' + pct;
        buf[3] = b'%';
        core::str::from_utf8(&buf[0..4]).unwrap_or(" 0%")
    }
}

/// Draw a horizontal channel gauge (-1000..+1000) with center ticks and a sliding 3px cursor.
fn draw_channel_gauge(
    lcd: &mut St7567,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    val: i16,
) {
    let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);

    // Frame
    Rectangle::new(Point::new(x, y), Size::new(width, height))
        .into_styled(border_style)
        .draw(lcd)
        .ok();

    let center_x = x + (width as i32 / 2);

    // Center tick line (top and bottom 2 pixels)
    Line::new(Point::new(center_x, y), Point::new(center_x, y + 1))
        .into_styled(border_style)
        .draw(lcd)
        .ok();
    Line::new(
        Point::new(center_x, y + height as i32 - 2),
        Point::new(center_x, y + height as i32 - 1),
    )
    .into_styled(border_style)
    .draw(lcd)
    .ok();

    // Map -1000..+1000 to inner cursor center position:
    // Leftmost cursor center: x + 2 (spans x+1..x+3)
    // Rightmost cursor center: x + width - 3 (spans x+width-4..x+width-2)
    let min_pos = x + 2;
    let max_pos = x + width as i32 - 3;
    let travel = max_pos - min_pos;
    let cursor_center = min_pos + (((val as i32 + 1000) * travel + 1000) / 2000);

    Rectangle::new(
        Point::new(cursor_center - 1, y + 1),
        Size::new(3, height - 2),
    )
    .into_styled(fill_style)
    .draw(lcd)
    .ok();
}

/// Draw a left-to-right throttle progress bar (-1000 is 0%, +1000 is 100%).
fn draw_progress_bar(
    lcd: &mut St7567,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    val: i16,
) {
    let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);

    // Frame
    Rectangle::new(Point::new(x, y), Size::new(width, height))
        .into_styled(border_style)
        .draw(lcd)
        .ok();

    // Map -1000..+1000 to 0..max_fill
    let max_fill = (width - 2) as i32;
    let normalized = (val as i32 + 1000).clamp(0, 2000);
    let fill_len = ((normalized * max_fill) / 2000) as u32;

    if fill_len > 0 {
        Rectangle::new(Point::new(x + 1, y + 1), Size::new(fill_len, height - 2))
            .into_styled(fill_style)
            .draw(lcd)
            .ok();
    }
}

#[entry]
fn main() -> ! {
    // 1. MCU Profile & Fast DFU Bootloader Check
    let mcu_profile = chip::get_mcu_profile();
    boot::check_dfu_entry(&mcu_profile);

    // 2. Initialize ST7567 128×64 LCD & Backlight immediately
    let mut lcd = St7567::new();

    // 3. Initialize ADC1 + DMA1 autonomous continuous scanner
    adc::init();

    // 4. Initialize input calibration and capture resting stick centers
    input::init();

    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let sep_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    let mut dfu_confirm_count = 0u8;

    loop {
        // Poll continuous DMA inputs
        let state = input::poll();

        // Check keys and runtime DFU trigger
        let keys = boot::scan_keys();
        if boot::is_dfu_requested(keys) {
            dfu_confirm_count += 1;
            if dfu_confirm_count >= 5 {
                lcd.clear(BinaryColor::Off).ok();
                Text::new("ENTERING DFU...", Point::new(18, 32), text_style)
                    .draw(&mut lcd)
                    .ok();
                lcd.flush();

                for _ in 0..300_000 {
                    cortex_m::asm::nop();
                }

                chip::enter_dfu_bootloader(&mcu_profile);
            }
        } else {
            dfu_confirm_count = 0;
        }

        // Render live flight screen
        lcd.clear(BinaryColor::Off).ok();

        // --- Top Status Bar (y = 0..10) ---
        Text::new("FS-i6X", Point::new(2, 9), text_style)
            .draw(&mut lcd)
            .ok();

        // Battery voltage
        let mut vbat_buf = [0u8; 6];
        let vbat_str = format_vbat(state.battery_mv, &mut vbat_buf);

        Text::new(vbat_str, Point::new(76, 9), text_style)
            .draw(&mut lcd)
            .ok();

        // Header separator line
        Line::new(Point::new(0, 11), Point::new(127, 11))
            .into_styled(sep_style)
            .draw(&mut lcd)
            .ok();

        // --- Stick Channel Bar Graphs (y = 13..44) ---
        let mut pct_buf = [0u8; 5];

        // CH1: Roll / Aileron
        Text::new("A", Point::new(2, 19), text_style).draw(&mut lcd).ok();
        draw_channel_gauge(&mut lcd, 12, 13, 76, 7, state.sticks.roll);
        let p1 = format_percent(state.sticks.roll, &mut pct_buf);
        Text::new(p1, Point::new(92, 19), text_style).draw(&mut lcd).ok();

        // CH2: Pitch / Elevator
        Text::new("E", Point::new(2, 27), text_style).draw(&mut lcd).ok();
        draw_channel_gauge(&mut lcd, 12, 21, 76, 7, state.sticks.pitch);
        let p2 = format_percent(state.sticks.pitch, &mut pct_buf);
        Text::new(p2, Point::new(92, 27), text_style).draw(&mut lcd).ok();

        // CH3: Throttle
        Text::new("T", Point::new(2, 35), text_style).draw(&mut lcd).ok();
        draw_progress_bar(&mut lcd, 12, 29, 76, 7, state.sticks.throttle);
        let p3 = format_throttle_percent(state.sticks.throttle, &mut pct_buf);
        Text::new(p3, Point::new(92, 35), text_style).draw(&mut lcd).ok();

        // CH4: Yaw / Rudder
        Text::new("R", Point::new(2, 43), text_style).draw(&mut lcd).ok();
        draw_channel_gauge(&mut lcd, 12, 37, 76, 7, state.sticks.yaw);
        let p4 = format_percent(state.sticks.yaw, &mut pct_buf);
        Text::new(p4, Point::new(92, 43), text_style).draw(&mut lcd).ok();

        // Separator above lower status
        Line::new(Point::new(0, 46), Point::new(127, 46))
            .into_styled(sep_style)
            .draw(&mut lcd)
            .ok();

        // --- Switches & Pots Line (y = 48..54) ---
        // SA..SD states
        let mut sw_buf = [b'A', b':', b'U', b' ', b'B', b':', b'U', b' ', b'C', b':', b'U', b' ', b'D', b':', b'U'];
        sw_buf[2] = state.switches.sa.as_char() as u8;
        sw_buf[6] = state.switches.sb.as_char() as u8;
        sw_buf[10] = state.switches.sc.as_char() as u8;
        sw_buf[14] = state.switches.sd.as_char() as u8;
        let sw_str = core::str::from_utf8(&sw_buf).unwrap_or("SW");
        Text::new(sw_str, Point::new(2, 54), text_style).draw(&mut lcd).ok();

        // Pots: V1 / V2 on right (scaled 0..9 across full turn)
        let mut pot_buf = [b'V', b':', b'0', b'/', b'0'];
        let p1 = (((state.pots.vr1 as i32 + 1000) * 9) / 2000).clamp(0, 9) as u8;
        let p2 = (((state.pots.vr2 as i32 + 1000) * 9) / 2000).clamp(0, 9) as u8;
        pot_buf[2] = b'0' + p1;
        pot_buf[4] = b'0' + p2;
        let pot_str = core::str::from_utf8(&pot_buf).unwrap_or("V:0/0");
        Text::new(pot_str, Point::new(96, 54), text_style).draw(&mut lcd).ok();

        // --- Bottom Diagnostic / Key Line (y = 56..63) ---
        let mut key_buf = [b'0'; 4];
        u16_to_hex(keys, &mut key_buf);
        let key_str = core::str::from_utf8(&key_buf).unwrap_or("0000");

        Text::new("KEY:", Point::new(2, 63), text_style).draw(&mut lcd).ok();
        Text::new(key_str, Point::new(28, 63), text_style).draw(&mut lcd).ok();

        if (keys & (1 << 12)) != 0 {
            Text::new("BIND", Point::new(58, 63), text_style).draw(&mut lcd).ok();
        } else {
            Text::new("DFU:Trims", Point::new(58, 63), text_style).draw(&mut lcd).ok();
        }

        // Flush frame to ST7567 LCD
        lcd.flush();

        // Frame pacing (~30 Hz refresh rate at 8 MHz)
        for _ in 0..50_000 {
            cortex_m::asm::nop();
        }
    }
}
