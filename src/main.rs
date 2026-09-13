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
mod buzzer;
mod calib;
mod chip;
mod display;
mod input;
mod menu;
mod rf;
mod storage;
mod trim;

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

/// Draw a horizontal channel gauge (-1000..+1000) with center ticks, trim marker, and a sliding 3px cursor.
fn draw_channel_gauge(
    lcd: &mut St7567,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    val: i16,
    trim: i8,
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

    // If trim is non-zero, draw a 1-pixel trim indicator tick
    if trim != 0 {
        let trim_x = center_x + ((trim as i32 * (travel / 2)) / 25);
        Line::new(Point::new(trim_x, y + 2), Point::new(trim_x, y + height as i32 - 3))
            .into_styled(border_style)
            .draw(lcd)
            .ok();
    }

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

/// Format active trim status to "TRM X:+00"
fn format_trim(axis: trim::ActiveTrim, val: i8, buf: &mut [u8; 9]) -> &str {
    let name = match axis {
        trim::ActiveTrim::Roll => b'A',
        trim::ActiveTrim::Pitch => b'E',
        trim::ActiveTrim::Throttle => b'T',
        trim::ActiveTrim::Yaw => b'R',
        trim::ActiveTrim::None => b' ',
    };
    buf[0] = b'T';
    buf[1] = b'R';
    buf[2] = b'M';
    buf[3] = b' ';
    buf[4] = name;
    buf[5] = b':';
    buf[6] = if val < 0 { b'-' } else if val > 0 { b'+' } else { b' ' };
    let abs = val.unsigned_abs();
    buf[7] = b'0' + (abs / 10);
    buf[8] = b'0' + (abs % 10);
    core::str::from_utf8(buf).unwrap_or("TRIM")
}

#[entry]
fn main() -> ! {
    // 1. MCU Profile & Fast DFU Bootloader Check
    let mcu_profile = chip::get_mcu_profile();
    boot::check_dfu_entry(&mcu_profile);

    // 2. Initialize System Clock to 48 MHz using external 8 MHz crystal (HSE) + PLL
    chip::init_system_clock();

    // 3. Initialize ST7567 128×64 LCD & Backlight immediately
    let mut lcd = St7567::new();

    // 4. Initialize ADC1 + DMA1 autonomous continuous scanner
    adc::init();

    // 5. Initialize input calibration and capture resting stick centers
    input::init();

    // 5. Initialize A7105 RF transceiver & AFHDS 2A stack
    let uid = chip::read_uid(&mcu_profile);
    let w0 = u32::from_le_bytes([uid[0], uid[1], uid[2], uid[3]]);
    let w1 = u32::from_le_bytes([uid[4], uid[5], uid[6], uid[7]]);
    let w2 = u32::from_le_bytes([uid[8], uid[9], uid[10], uid[11]]);
    let tx_id = w0 ^ w1 ^ w2;

    let initial_keys = boot::scan_keys();
    let bind_on_boot = (initial_keys & (1 << 12)) != 0;

    let rf_ok = rf::init(tx_id);
    if bind_on_boot {
        rf::set_bind_mode(true);
    }

    // 6. Initialize Buzzer & Digital Trims
    let mut buzzer = buzzer::Buzzer::new();
    buzzer.init();

    // 7. Load persistent radio configuration
    let mut config = storage::load_config();
    buzzer.enabled = config.audio_enabled != 0;
    buzzer.click(); // Power-on audible confirmation

    let mut trims = trim::TrimController::new();
    trims.throttle_enabled = config.throttle_trim != 0;

    // Apply saved backlight brightness level
    lcd.set_backlight_level(config.backlight_brightness * 10);

    let mut calib_wizard = calib::CalibWizard::new();
    let mut menu_controller = menu::MenuController::new();

    // Check if OK button held at power-on to launch calibration directly
    if (initial_keys & (1 << 10)) != 0 {
        calib_wizard.start(&mut buzzer);
    }

    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let sep_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    let mut dfu_confirm_count = 0u8;
    let mut ok_hold_ms = 0u16;
    let mut bl_timer_ms: u32 = 30_000;
    let mut prev_stick_sample = 2048u16;

    loop {
        // Poll continuous DMA inputs
        let state = input::poll();

        // Check keys and update trims and buzzer
        let keys = boot::scan_keys();
        buzzer.tick(20);
        trims.update(keys, 20, &mut buzzer);

        // Backlight activity reset (keys pressed or stick moved > 30 counts)
        let stick_moved = (state.raw[0] as i32 - prev_stick_sample as i32).abs() > 30;
        prev_stick_sample = state.raw[0];

        if keys != 0 || stick_moved {
            let timeout_ms: u32 = match config.backlight_timeout {
                1 => 15_000,
                2 => 30_000,
                3 => 60_000,
                _ => 0,
            };
            bl_timer_ms = timeout_ms;
            lcd.set_backlight_level(config.backlight_brightness * 10);
        } else if config.backlight_timeout != 0 {
            if bl_timer_ms > 20 {
                bl_timer_ms -= 20;
            } else {
                bl_timer_ms = 0;
                lcd.set_backlight_level(0);
            }
        }

        // Map inputs to 14 AFHDS 2A channels with digital trims (1000..2000 µs)
        let mut rf_chs = [1500u16; 14];
        let ch1_raw = ((state.sticks.roll / 2) + 1500).clamp(1000, 2000) as u16;
        let ch2_raw = ((state.sticks.pitch / 2) + 1500).clamp(1000, 2000) as u16;
        let ch3_raw = ((state.sticks.throttle / 2) + 1500).clamp(1000, 2000) as u16;
        let ch4_raw = ((state.sticks.yaw / 2) + 1500).clamp(1000, 2000) as u16;

        rf_chs[0] = trim::TrimController::apply(ch1_raw, trims.values.roll);
        rf_chs[1] = trim::TrimController::apply(ch2_raw, trims.values.pitch);
        rf_chs[2] = if trims.throttle_enabled {
            trim::TrimController::apply(ch3_raw, trims.values.throttle)
        } else {
            ch3_raw
        };
        rf_chs[3] = trim::TrimController::apply(ch4_raw, trims.values.yaw);
        rf_chs[4] = if state.switches.sa == input::SwitchPos::Up { 1000 } else { 2000 };
        rf_chs[5] = match state.switches.sb {
            input::SwitchPos::Up => 1000,
            input::SwitchPos::Mid => 1500,
            input::SwitchPos::Down => 2000,
        };
        rf_chs[6] = ((state.pots.vr1 / 2) + 1500).clamp(1000, 2000) as u16;
        rf_chs[7] = ((state.pots.vr2 / 2) + 1500).clamp(1000, 2000) as u16;
        rf_chs[8] = match state.switches.sc {
            input::SwitchPos::Up => 1000,
            input::SwitchPos::Mid => 1500,
            input::SwitchPos::Down => 2000,
        };
        rf_chs[9] = if state.switches.sd == input::SwitchPos::Up { 1000 } else { 2000 };
        rf::set_channels(&rf_chs);

        let telem = rf::get_telemetry();
        let is_binding = rf::is_binding();

        // Check dedicated Bind key (PF2) and Cancel key (bit 11) for binding control
        let bind_key = (keys & (1 << 12)) != 0;
        let cancel_key = (keys & (1 << 11)) != 0;

        if is_binding {
            if cancel_key {
                rf::set_bind_mode(false);
                buzzer.click();
            }
        } else if bind_key {
            rf::set_bind_mode(true);
            buzzer.click();
        }

        // Long-press OK (1.2s) from flight dashboard opens Settings Menu
        if !menu_controller.is_active() && !calib_wizard.is_active() {
            if (keys & (1 << 10)) != 0 {
                ok_hold_ms = ok_hold_ms.saturating_add(20);
                if ok_hold_ms >= 1200 {
                    menu_controller.open(&mut buzzer);
                    ok_hold_ms = 0;
                }
            } else {
                ok_hold_ms = 0;
            }
        }

        // If Settings Menu is active, update menu and loop
        if menu_controller.is_active() {
            menu_controller.update(
                &mut lcd,
                keys,
                &mut config,
                &mut trims,
                &state.raw,
                &rf_chs,
                &mut buzzer,
            );

            if menu_controller.request_calibration {
                calib_wizard.start(&mut buzzer);
                menu_controller.request_calibration = false;
            }

            lcd.flush();
            for _ in 0..160_000 {
                cortex_m::asm::nop();
            }
            continue;
        }

        // If calibration wizard is active, update wizard, flush display, and loop
        if calib_wizard.is_active() {
            calib_wizard.update(&mut lcd, &state.raw, keys, 20, &mut buzzer);
            lcd.flush();

            for _ in 0..160_000 {
                cortex_m::asm::nop();
            }
            continue;
        }

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

        // Center RF status
        if !rf_ok {
            let id = rf::get_last_chip_id();
            let mut err_buf = [b'E', b':', b'0', b'0'];
            err_buf[2] = HEX_CHARS[((id >> 4) & 0x0F) as usize];
            err_buf[3] = HEX_CHARS[(id & 0x0F) as usize];
            let err_str = core::str::from_utf8(&err_buf).unwrap_or("E:??");
            Text::new(err_str, Point::new(48, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else if is_binding {
            Text::new("BINDING", Point::new(46, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else if telem.connected {
            let mut rssi_buf = [b'R', b':', b' ', b' ', b'%'];
            let r = telem.rssi.min(100);
            if r >= 100 {
                rssi_buf[2] = b'1';
                rssi_buf[3] = b'0';
                rssi_buf[4] = b'0';
            } else {
                rssi_buf[2] = b'0' + (r / 10);
                rssi_buf[3] = b'0' + (r % 10);
            }
            let r_str = core::str::from_utf8(&rssi_buf).unwrap_or("R:--%");
            Text::new(r_str, Point::new(48, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else if rf::is_bound() {
            Text::new("RF:OK", Point::new(50, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else {
            Text::new("NO BIND", Point::new(44, 9), text_style)
                .draw(&mut lcd)
                .ok();
        }

        // Battery voltage
        let mut vbat_buf = [0u8; 6];
        let vbat_str = format_vbat(state.battery_mv, &mut vbat_buf);

        Text::new(vbat_str, Point::new(94, 9), text_style)
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
        draw_channel_gauge(&mut lcd, 12, 13, 76, 7, state.sticks.roll, trims.values.roll);
        let p1 = format_percent(state.sticks.roll, &mut pct_buf);
        Text::new(p1, Point::new(92, 19), text_style).draw(&mut lcd).ok();

        // CH2: Pitch / Elevator
        Text::new("E", Point::new(2, 27), text_style).draw(&mut lcd).ok();
        draw_channel_gauge(&mut lcd, 12, 21, 76, 7, state.sticks.pitch, trims.values.pitch);
        let p2 = format_percent(state.sticks.pitch, &mut pct_buf);
        Text::new(p2, Point::new(92, 27), text_style).draw(&mut lcd).ok();

        // CH3: Throttle
        Text::new("T", Point::new(2, 35), text_style).draw(&mut lcd).ok();
        draw_progress_bar(&mut lcd, 12, 29, 76, 7, state.sticks.throttle);
        let p3 = format_throttle_percent(state.sticks.throttle, &mut pct_buf);
        Text::new(p3, Point::new(92, 35), text_style).draw(&mut lcd).ok();

        // CH4: Yaw / Rudder
        Text::new("R", Point::new(2, 43), text_style).draw(&mut lcd).ok();
        draw_channel_gauge(&mut lcd, 12, 37, 76, 7, state.sticks.yaw, trims.values.yaw);
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
        if is_binding {
            Text::new("[ESC] Abort Bind", Point::new(16, 63), text_style)
                .draw(&mut lcd)
                .ok();
        } else {
            if trims.last_active != trim::ActiveTrim::None {
                let mut trm_buf = [0u8; 9];
                let val = match trims.last_active {
                    trim::ActiveTrim::Roll => trims.values.roll,
                    trim::ActiveTrim::Pitch => trims.values.pitch,
                    trim::ActiveTrim::Throttle => trims.values.throttle,
                    trim::ActiveTrim::Yaw => trims.values.yaw,
                    trim::ActiveTrim::None => 0,
                };
                let trm_str = format_trim(trims.last_active, val, &mut trm_buf);
                Text::new(trm_str, Point::new(2, 63), text_style).draw(&mut lcd).ok();
            } else {
                let mut key_buf = [b'0'; 4];
                u16_to_hex(keys, &mut key_buf);
                let key_str = core::str::from_utf8(&key_buf).unwrap_or("0000");

                Text::new("KEY:", Point::new(2, 63), text_style).draw(&mut lcd).ok();
                Text::new(key_str, Point::new(28, 63), text_style).draw(&mut lcd).ok();
            }

            if telem.connected && telem.rx_voltage_mv > 0 {
                let mut rxv_buf = [0u8; 6];
                let rxv_str = format_vbat(telem.rx_voltage_mv, &mut rxv_buf);
                Text::new("RX:", Point::new(58, 63), text_style).draw(&mut lcd).ok();
                Text::new(rxv_str, Point::new(76, 63), text_style).draw(&mut lcd).ok();
            } else if (keys & (1 << 12)) != 0 {
                Text::new("BIND", Point::new(58, 63), text_style).draw(&mut lcd).ok();
            } else {
                Text::new("Hold OK:Menu", Point::new(54, 63), text_style).draw(&mut lcd).ok();
            }
        }

        // Flush frame to ST7567 LCD
        lcd.flush();
    }
}
