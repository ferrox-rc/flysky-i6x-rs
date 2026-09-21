#![no_std]
#![no_main]

use panic_halt as _;
use stm32f0xx_hal as _;

use cortex_m_rt::entry;
use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, ascii::FONT_6X10, MonoTextStyle},
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
mod crsf;
mod curve;
mod display;
mod input;
mod menu;
mod mixer;
mod rf;
mod storage;
mod time;
mod trim;
mod usb;

use display::St7567;

const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

#[allow(dead_code)]
fn u16_to_hex(val: u16, buf: &mut [u8; 4]) {
    buf[0] = HEX_CHARS[((val >> 12) & 0x0F) as usize];
    buf[1] = HEX_CHARS[((val >> 8) & 0x0F) as usize];
    buf[2] = HEX_CHARS[((val >> 4) & 0x0F) as usize];
    buf[3] = HEX_CHARS[(val & 0x0F) as usize];
}

fn u32_to_hex(val: u32, buf: &mut [u8; 8]) {
    for i in 0..8 {
        buf[7 - i] = HEX_CHARS[((val >> (i * 4)) & 0x0F) as usize];
    }
}

fn u16_to_dec_4(val: u16, buf: &mut [u8; 4]) {
    buf[0] = b'0' + ((val / 1000) % 10) as u8;
    buf[1] = b'0' + ((val / 100) % 10) as u8;
    buf[2] = b'0' + ((val / 10) % 10) as u8;
    buf[3] = b'0' + (val % 10) as u8;
}

fn u32_to_dec_5(val: u32, buf: &mut [u8; 5]) {
    let v = val.min(99999);
    buf[0] = if v >= 10000 { b'0' + ((v / 10000) % 10) as u8 } else { b' ' };
    buf[1] = if v >= 1000 { b'0' + ((v / 1000) % 10) as u8 } else { b' ' };
    buf[2] = if v >= 100 { b'0' + ((v / 100) % 10) as u8 } else { b' ' };
    buf[3] = if v >= 10 { b'0' + ((v / 10) % 10) as u8 } else { b' ' };
    buf[4] = b'0' + (v % 10) as u8;
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
    trim: i8,
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

    if trim != 0 {
        // Trim tick: -25..+25 steps maps to ±10% (±100 µs) of full travel (2000 counts)
        let trim_offset = (trim as i32 * max_fill) / 250;
        let trim_x = (x + 1 + trim_offset.max(0)).min(x + max_fill);
        let tick_color = if (trim_x - (x + 1)) < fill_len as i32 {
            BinaryColor::Off
        } else {
            BinaryColor::On
        };
        let tick_style = PrimitiveStyle::with_stroke(tick_color, 1);
        Line::new(Point::new(trim_x, y + 1), Point::new(trim_x, y + height as i32 - 2))
            .into_styled(tick_style)
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

    // Initialize SysTick 1.000 ms hardware monotonic timekeeper
    time::init();

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

    // 7. Load persistent radio storage and 20-model configuration
    let mut storage = storage::load_storage();
    buzzer.enabled = storage.radio.audio_enabled != 0;
    buzzer.tone_style = buzzer::ToneStyle::from_u8(storage.radio.tone_style);
    buzzer.chime_welcome(); // Power-on audible confirmation

    // Initialize USB peripheral (Joystick / Serial / Composite / Off)
    usb::init(storage.radio.usb_mode);

    // Initialize CRSF / ExpressLRS expansion bay peripheral (USART2 & PC13)
    crsf::init(storage.radio.ext_module_pwr == 0);

    let mut trims = trim::TrimController::new();
    trims.throttle_enabled = storage.radio.throttle_trim != 0;

    // Load active model trims and receiver ID
    let active = storage.active_model();
    trims.values.roll = active.trims[0];
    trims.values.pitch = active.trims[1];
    trims.values.throttle = active.trims[2];
    trims.values.yaw = active.trims[3];
    rf::set_rx_id(active.rx_id);

    // Apply saved backlight brightness level & LCD contrast
    lcd.set_backlight_level(storage.radio.backlight_brightness * 10);
    lcd.set_contrast(storage.radio.lcd_contrast);

    let mut calib_wizard = calib::CalibWizard::new();
    let mut menu_controller = menu::MenuController::new();

    // Check if OK button held at power-on to launch calibration directly
    if (initial_keys & (1 << 10)) != 0 {
        calib_wizard.start(&mut buzzer);
    }

    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
    let sep_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    let mut ok_hold_ms = 0u16;
    let mut bl_timer_ms: u32 = 30_000;
    let mut prev_stick_samples = [2048u16; 6];
    let mut prev_bind_key = bind_on_boot;
    let mut flight_page: usize = 0;
    let mut bind_hold_ms: u32 = 0;
    let mut bind_was_held: bool = false;
    let mut vbat_alarm_timer: u32 = 7000;
    let mut rssi_alarm_timer: u32 = 0;
    let mut min_rssi: u8 = 100;
    let mut min_rx_v_mv: u16 = 0xFFFF;
    let mut telem_seen: bool = false;
    let mut inactivity_timer_ms: u32 = 0;
    let mut inactivity_beep_timer: u32 = 0;
    let mut blink_phase: u8 = 0;
    let mut prev_armed: bool = false;
    let mut prev_active_model: u8 = storage.radio.active_model;
    let mut menu_was_active: bool = false;
    let mut last_display_ms: u32 = 0;
    let mut last_tick_ms: u32 = 0;

    // 8. Pre-flight Startup Safety Check: Throttle at idle and switches in safe (UP) positions
    if !calib_wizard.is_active() {
        let mut preflight_beep_timer: u32 = 0;
        let mut preflight_last_render: u32 = 0;

        let mut warned = false;

        loop {
            let now = time::millis();
            let state = input::poll();
            let keys = boot::scan_keys();

            let thr_unsafe = state.sticks.throttle > -900;
            let sa_unsafe = state.switches.sa != input::SwitchPos::Up;
            let sb_unsafe = state.switches.sb != input::SwitchPos::Up;
            let sc_unsafe = state.switches.sc != input::SwitchPos::Up;
            let sd_unsafe = state.switches.sd != input::SwitchPos::Up;
            let sw_unsafe = sa_unsafe || sb_unsafe || sc_unsafe || sd_unsafe;

            // Cancel key (Bit 11: KEY_CANCEL) allows pilot to bypass warning
            let cancel_pressed = (keys & (1 << 11)) != 0;

            if (!thr_unsafe && !sw_unsafe) || cancel_pressed {
                if warned {
                    buzzer.play_tone(2200, 40);
                }
                break;
            }
            warned = true;

        // Lock RF transmission to safe idle/failsafe during warning
        rf::set_channels(&[1500, 1500, 1000, 1500, 1000, 1000, 1500, 1500, 1000, 1000, 1500, 1500, 1500, 1500]);

        // Service USB subsystem so host enumeration and connection succeed during preflight safety hold
        usb::poll(now, &[1500, 1500, 1000, 1500, 1000, 1000, 1500, 1500, 1000, 1000, 1500, 1500, 1500, 1500], &state.switches, &rf::get_telemetry(), state.battery_mv);

        // Beep alarm every 800 ms
        if now.wrapping_sub(preflight_beep_timer) >= 800 {
            preflight_beep_timer = now;
            buzzer.warn_preflight();
        }

        // Render Safety Warning Screen at ~30 Hz
        if now.wrapping_sub(preflight_last_render) >= 33 {
            let dt = (now.wrapping_sub(preflight_last_render)).min(100) as u16;
            preflight_last_render = now;
            buzzer.tick(dt);

            lcd.clear(BinaryColor::Off).ok();
            Text::new("SAFETY WARNING!", Point::new(16, 9), text_style).draw(&mut lcd).ok();
            Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(sep_style).draw(&mut lcd).ok();

            if thr_unsafe {
                Text::new("THROTTLE NOT AT IDLE!", Point::new(2, 23), text_style).draw(&mut lcd).ok();
            }

            if sw_unsafe {
                Text::new("SWITCH WARNING:", Point::new(2, 34), text_style).draw(&mut lcd).ok();
                let mut sw_warn = *b"                ";
                let mut col = 0;
                if sa_unsafe && col + 4 <= 16 {
                    sw_warn[col..col + 4].copy_from_slice(b"[SA]");
                    col += 4;
                    if col < 16 { sw_warn[col] = b' '; col += 1; }
                }
                if sb_unsafe && col + 4 <= 16 {
                    sw_warn[col..col + 4].copy_from_slice(b"[SB]");
                    col += 4;
                    if col < 16 { sw_warn[col] = b' '; col += 1; }
                }
                if sc_unsafe && col + 4 <= 16 {
                    sw_warn[col..col + 4].copy_from_slice(b"[SC]");
                    col += 4;
                    if col < 16 { sw_warn[col] = b' '; col += 1; }
                }
                if sd_unsafe && col + 4 <= 16 {
                    sw_warn[col..col + 4].copy_from_slice(b"[SD]");
                    col += 4;
                }
                let sw_str = core::str::from_utf8(&sw_warn[..col.min(16)]).unwrap_or("CHECK SWITCHES");
                Text::new(sw_str, Point::new(2, 44), text_style).draw(&mut lcd).ok();
            }

            Line::new(Point::new(0, 55), Point::new(127, 55)).into_styled(sep_style).draw(&mut lcd).ok();
            Text::new("Lower Thr/Safe SW  [ESC]Skip", Point::new(2, 62), text_style_small).draw(&mut lcd).ok();

            lcd.flush();
        }
    }
}

    // Initialize previous switch snapshot and armed state so startup does not spuriously chirp
    let init_state = input::poll();
    let mut prev_switches = init_state.switches;
    if storage.active_model().arm_switch > 0 && storage.active_model().arm_switch <= 10 {
        prev_armed = mixer::is_switch_active(storage.active_model().arm_switch, &init_state.switches);
    }

    loop {
        let now = time::millis();
        let dt_ms = (now.wrapping_sub(last_tick_ms)).min(100) as u16;
        let run_display = now.wrapping_sub(last_display_ms) >= 33; // ~30 Hz frame rate

        // Poll continuous DMA inputs (sub-microsecond)
        let state = input::poll();

        // Check keys and update trims and buzzer
        let keys = boot::scan_keys();
        if dt_ms > 0 {
            last_tick_ms = now;
            buzzer.tick(dt_ms);
            trims.update(keys, dt_ms, &mut buzzer);
        }

        // Backlight & inactivity activity tracking across all physical controls
        let stick_moved = (state.raw[0] as i32 - prev_stick_samples[0] as i32).abs() > 30   // Roll / Aileron
            || (state.raw[1] as i32 - prev_stick_samples[1] as i32).abs() > 30              // Pitch / Elevator
            || (state.raw[2] as i32 - prev_stick_samples[2] as i32).abs() > 30              // Throttle
            || (state.raw[3] as i32 - prev_stick_samples[3] as i32).abs() > 30              // Yaw / Rudder
            || (state.raw[6] as i32 - prev_stick_samples[4] as i32).abs() > 40              // Pot VRA
            || (state.raw[7] as i32 - prev_stick_samples[5] as i32).abs() > 40;             // Pot VRB
        prev_stick_samples[0] = state.raw[0];
        prev_stick_samples[1] = state.raw[1];
        prev_stick_samples[2] = state.raw[2];
        prev_stick_samples[3] = state.raw[3];
        prev_stick_samples[4] = state.raw[6];
        prev_stick_samples[5] = state.raw[7];

        let sw_changed = state.switches != prev_switches;
        prev_switches = state.switches;

        let user_active = keys != 0 || stick_moved || sw_changed;

        if user_active {
            let timeout_ms: u32 = match storage.radio.backlight_timeout {
                1 => 15_000,
                2 => 30_000,
                3 => 60_000,
                _ => 0,
            };
            bl_timer_ms = timeout_ms;
            lcd.set_backlight_level(storage.radio.backlight_brightness * 10);
        } else if storage.radio.backlight_timeout != 0 && dt_ms > 0 {
            if bl_timer_ms > dt_ms as u32 {
                bl_timer_ms -= dt_ms as u32;
            } else {
                bl_timer_ms = 0;
                lcd.set_backlight_level(0);
            }
        }

        // Radio Inactivity Alarm (10 minutes without physical control activity)
        if user_active {
            inactivity_timer_ms = 0;
            inactivity_beep_timer = 0;
        } else if dt_ms > 0 {
            inactivity_timer_ms = inactivity_timer_ms.saturating_add(dt_ms as u32);
            if inactivity_timer_ms >= 600_000 {
                if inactivity_beep_timer >= 30_000 {
                    inactivity_beep_timer = 0;
                    buzzer.warn_inactivity();
                } else {
                    inactivity_beep_timer += dt_ms as u32;
                }
            }
        }

        // Map inputs to 14 AFHDS 2A channels with digital trims (1000..2000 µs)
        let active_model = storage.active_model();

        // Resynchronize arm state when active model changes or when exiting settings menu
        let menu_active = menu_controller.is_active() || calib_wizard.is_active();
        if storage.radio.active_model != prev_active_model || (menu_was_active && !menu_active) {
            prev_active_model = storage.radio.active_model;
            prev_armed = if active_model.arm_switch > 0 && active_model.arm_switch <= 10 {
                mixer::is_switch_active(active_model.arm_switch, &state.switches)
            } else {
                false
            };
        }
        menu_was_active = menu_active;

        // Check configured Arm Switch condition and play Armed/Disarmed chimes
        if active_model.arm_switch > 0 && active_model.arm_switch <= 10 {
            let is_armed = mixer::is_switch_active(active_model.arm_switch, &state.switches);
            if is_armed != prev_armed {
                prev_armed = is_armed;
                if !menu_active {
                    if is_armed {
                        buzzer.chime_armed();
                    } else {
                        buzzer.chime_disarmed();
                    }
                }
            }
        }

        // Evaluate active model throttle curve (normalized 0..1000)
        let thr_input = ((state.sticks.throttle + 1000) / 2).clamp(0, 1000) as u16;
        let thr_curved = curve::evaluate_curve(
            thr_input,
            active_model.thr_curve_pts,
            active_model.thr_curve_smooth != 0,
            &active_model.thr_curve,
        );

        // Compute all 14 channels via 4-stage pipeline (D/R, Expo, Templates, Matrix Mixer, Trims, Reversing)
        let rf_chs = mixer::compute_channels(
            state.sticks.roll,
            state.sticks.pitch,
            thr_curved,
            state.sticks.yaw,
            &[state.pots.vr1, state.pots.vr2],
            &state.switches,
            active_model,
            &trims,
            storage.radio.throttle_trim,
        );

        let is_crsf = active_model.rf_protocol == 1;
        let sim_mode = usb::is_sim_mode();

        if is_crsf {
            // Enable CRSF UART and PC13 power switch (unless in USB Simulator mode)
            let crsf_active = !sim_mode;
            crsf::set_enabled(crsf_active, active_model.crsf_baud);
            if crsf_active {
                crsf::update_channels(now, &rf_chs);
                crsf::poll_telemetry(now);
            }
            // Silence internal A7105 transceiver
            rf::set_silenced(true);
        } else {
            // AFHDS 2A mode: disable CRSF external module and power switch
            crsf::set_enabled(false, 0);
            rf::set_silenced(sim_mode);
            if !sim_mode {
                rf::set_channels(&rf_chs);
            }
        }

        // Map telemetry data for USB and display based on active protocol
        let telem = if is_crsf {
            let ct = crsf::get_telemetry();
            rf::afhds2a::TelemetryData {
                connected: ct.connected,
                rssi: ct.uplink_link_quality, // Display Link Quality (0..100%) as primary link indicator
                rx_voltage_mv: ct.rx_battery_mv,
                packets_sent: 0,
                packets_received: 0,
            }
        } else {
            rf::get_telemetry()
        };
        let is_binding = !is_crsf && rf::is_binding();

        // Poll USB subsystem (Joystick HID @ 100Hz, Serial CLI / Telemetry)
        usb::poll(now, &rf_chs, &state.switches, &telem, state.battery_mv);

        // Check dedicated Bind key (PF2) and Cancel key (bit 11) for binding and page navigation
        let bind_key_raw = (keys & (1 << 12)) != 0;
        let bind_pressed = bind_key_raw && !prev_bind_key;
        prev_bind_key = bind_key_raw;

        let cancel_key = (keys & (1 << 11)) != 0;

        if is_binding {
            if cancel_key || bind_pressed {
                rf::set_bind_mode(false);
                buzzer.click();
            }
        } else if menu_controller.is_active() || calib_wizard.is_active() {
            // Inside menus: bind key is handled by the menu without side effects
            bind_hold_ms = 0;
            bind_was_held = false;
        } else {
            // Flight dashboard active:
            // - Hold BIND (PF2) >= 1000ms: trigger AFHDS 2A binding mode
            // - Tap BIND (release 40..1000ms): cycle flight display page (0 -> 1 -> 2 -> 0)
            if bind_key_raw {
                bind_hold_ms = bind_hold_ms.saturating_add(dt_ms as u32);
                if bind_hold_ms >= 1000 && !bind_was_held {
                    rf::set_bind_mode(true);
                    buzzer.play_tone(2400, 150);
                    bind_was_held = true;
                }
            } else {
                if !bind_was_held && bind_hold_ms >= 40 {
                    flight_page = (flight_page + 1) % 4;
                    buzzer.play_tone(2200, 30);
                }
                bind_hold_ms = 0;
                bind_was_held = false;
            }
        }

        // Persist newly bound RX ID safely to Flash outside ISR
        if let Some(new_rx_id) = rf::take_pending_rx_save() {
            if new_rx_id != 0 && new_rx_id != 0xFFFF_FFFF && storage.active_model().rx_id != new_rx_id {
                storage.active_model_mut().rx_id = new_rx_id;
                storage::save_storage(&storage);
                buzzer.play_tone_pattern(2400, 70, 50, 2);
            }
        }

        // Long-press OK (1.2s) from flight dashboard opens Settings Menu
        if !menu_controller.is_active() && !calib_wizard.is_active() {
            if (keys & (1 << 10)) != 0 {
                ok_hold_ms = ok_hold_ms.saturating_add(dt_ms);
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
            if run_display {
                last_display_ms = now;
                menu_controller.update(
                    &mut lcd,
                    keys,
                    &mut storage,
                    &mut trims,
                    &state.raw,
                    &rf_chs,
                    &mut buzzer,
                );

                if menu_controller.request_calibration {
                    calib_wizard.start(&mut buzzer);
                    menu_controller.request_calibration = false;
                }

                if menu_controller.request_bind {
                    rf::set_bind_mode(true);
                    menu_controller.request_bind = false;
                    buzzer.play_tone(2400, 150);
                }

                lcd.flush();
            }
            continue;
        }

        // If calibration wizard is active, update wizard, flush display, and loop
        if calib_wizard.is_active() {
            if run_display {
                last_display_ms = now;
                calib_wizard.update(&mut lcd, &state.raw, keys, dt_ms.max(20), &mut buzzer);
                lcd.flush();
            }
            continue;
        }

        // Throttle flight display rendering to ~30 Hz.
        // On non-display passes, loop immediately so stick polling and RF channel
        // updates run at kHz rates without any LCD latency!
        if !run_display {
            continue;
        }
        last_display_ms = now;

        // Render live flight screen
        lcd.clear(BinaryColor::Off).ok();

        // --- Top Status Bar (y = 0..10) ---
        let act_idx = storage.radio.active_model as usize;
        let raw_name = core::str::from_utf8(&storage.active_model().name).unwrap_or("").trim();
        let mut m_buf = *b"M00";
        m_buf[1] = b'0' + ((act_idx + 1) / 10) as u8;
        m_buf[2] = b'0' + ((act_idx + 1) % 10) as u8;
        let m_fallback = core::str::from_utf8(&m_buf).unwrap_or("M01");
        let m_display = if raw_name.is_empty() { m_fallback } else { raw_name };
        Text::new(m_display, Point::new(2, 9), text_style)
            .draw(&mut lcd)
            .ok();

        // RF status pushed over to x = 65..95
        if !rf_ok && !is_crsf {
            let id = rf::get_last_chip_id();
            let mut err_buf = *b"E:00";
            err_buf[2] = HEX_CHARS[((id >> 4) & 0x0F) as usize];
            err_buf[3] = HEX_CHARS[(id & 0x0F) as usize];
            let err_str = core::str::from_utf8(&err_buf).unwrap_or("E:??");
            Text::new(err_str, Point::new(68, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else if is_binding {
            Text::new("BIND", Point::new(68, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else if usb::is_sim_mode() {
            Text::new("U:SIM", Point::new(65, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else if telem.connected {
            let mut rssi_buf = *b"R:  %";
            let r = telem.rssi.min(100);
            if r >= 100 {
                Text::new("R:100", Point::new(65, 9), text_style)
                    .draw(&mut lcd)
                    .ok();
            } else {
                rssi_buf[2] = b'0' + (r / 10);
                rssi_buf[3] = b'0' + (r % 10);
                let r_str = core::str::from_utf8(&rssi_buf).unwrap_or("R:--%");
                Text::new(r_str, Point::new(65, 9), text_style)
                    .draw(&mut lcd)
                    .ok();
            }
        } else if is_crsf {
            Text::new("CRSF", Point::new(66, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else if rf::is_bound() {
            Text::new("RF:OK", Point::new(65, 9), text_style)
                .draw(&mut lcd)
                .ok();
        } else {
            Text::new("NO RF", Point::new(65, 9), text_style)
                .draw(&mut lcd)
                .ok();
        }

        // Battery voltage alarm & display (x = 98)
        let mut vbat_buf = [0u8; 6];
        let vbat_str = format_vbat(state.battery_mv, &mut vbat_buf);

        let vbat_warn_mv = (storage.radio.vbat_warn_deci as u16) * 100;
        let vbat_is_low = state.battery_mv < vbat_warn_mv;

        if vbat_is_low {
            if vbat_alarm_timer >= 8000 {
                vbat_alarm_timer = 0;
                buzzer.warn_battery();
            } else {
                vbat_alarm_timer += 33;
            }

            // Invert/blink badge every ~320 ms
            if (blink_phase & 0x10) != 0 {
                Rectangle::new(Point::new(97, 0), Size::new(31, 10))
                    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                    .draw(&mut lcd)
                    .ok();
                let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                Text::new(vbat_str, Point::new(98, 9), inv_style).draw(&mut lcd).ok();
            } else {
                Text::new(vbat_str, Point::new(98, 9), text_style).draw(&mut lcd).ok();
            }
        } else {
            vbat_alarm_timer = 7000;
            Text::new(vbat_str, Point::new(98, 9), text_style).draw(&mut lcd).ok();
        }

        blink_phase = blink_phase.wrapping_add(1);

        // Downlink Telemetry RSSI Range Alarms & Stats Tracking
        if telem.connected {
            if !telem_seen {
                telem_seen = true;
                min_rssi = telem.rssi;
                if telem.rx_voltage_mv > 0 {
                    min_rx_v_mv = telem.rx_voltage_mv;
                }
            } else {
                if telem.rssi < min_rssi {
                    min_rssi = telem.rssi;
                }
                if telem.rx_voltage_mv > 0 && telem.rx_voltage_mv < min_rx_v_mv {
                    min_rx_v_mv = telem.rx_voltage_mv;
                }
            }

            if telem.rssi < 20 {
                if rssi_alarm_timer >= 3000 {
                    rssi_alarm_timer = 0;
                    buzzer.warn_rssi_critical();
                } else {
                    rssi_alarm_timer += 33;
                }
            } else if telem.rssi < 40 {
                if rssi_alarm_timer >= 6000 {
                    rssi_alarm_timer = 0;
                    buzzer.warn_rssi_low();
                } else {
                    rssi_alarm_timer += 33;
                }
            } else {
                rssi_alarm_timer = 0;
            }
        } else {
            rssi_alarm_timer = 0;
        }

        // Header separator line
        Line::new(Point::new(0, 11), Point::new(127, 11))
            .into_styled(sep_style)
            .draw(&mut lcd)
            .ok();

        match flight_page {
            0 => {
                // --- Page 0: Primary Gimbals & Trims (y = 13..44) ---
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
                let thr_trim = if storage.radio.throttle_trim != 0 { trims.values.throttle } else { 0 };
                draw_progress_bar(&mut lcd, 12, 29, 76, 7, state.sticks.throttle, thr_trim);
                let p3 = format_throttle_percent(state.sticks.throttle, &mut pct_buf);
                Text::new(p3, Point::new(92, 35), text_style).draw(&mut lcd).ok();

                // CH4: Yaw / Rudder
                Text::new("R", Point::new(2, 43), text_style).draw(&mut lcd).ok();
                draw_channel_gauge(&mut lcd, 12, 37, 76, 7, state.sticks.yaw, trims.values.yaw);
                let p4 = format_percent(state.sticks.yaw, &mut pct_buf);
                Text::new(p4, Point::new(92, 43), text_style).draw(&mut lcd).ok();

                // Switches & Pots Line (y = 47..53)
                let mut sw_buf = *b"A:U B:U C:U D:U";
                sw_buf[2] = state.switches.sa.as_char() as u8;
                sw_buf[6] = state.switches.sb.as_char() as u8;
                sw_buf[10] = state.switches.sc.as_char() as u8;
                sw_buf[14] = state.switches.sd.as_char() as u8;
                let sw_str = core::str::from_utf8(&sw_buf).unwrap_or("SW");
                Text::new(sw_str, Point::new(2, 53), text_style).draw(&mut lcd).ok();

                // Pots: V1 / V2 on right (scaled 0..9 across full turn)
                let mut pot_buf = *b"V:0/0";
                let p1 = (((state.pots.vr1 as i32 + 1000) * 9) / 2000).clamp(0, 9) as u8;
                let p2 = (((state.pots.vr2 as i32 + 1000) * 9) / 2000).clamp(0, 9) as u8;
                pot_buf[2] = b'0' + p1;
                pot_buf[4] = b'0' + p2;
                let pot_str = core::str::from_utf8(&pot_buf).unwrap_or("V:0/0");
                Text::new(pot_str, Point::new(96, 53), text_style).draw(&mut lcd).ok();

                // Separator above footer
                Line::new(Point::new(0, 55), Point::new(127, 55))
                    .into_styled(sep_style)
                    .draw(&mut lcd)
                    .ok();

                // Bottom Diagnostic / Key Line (y = 56..63)
                if is_binding {
                    Text::new("[ESC] Finish Bind", Point::new(26, 62), text_style_small).draw(&mut lcd).ok();
                } else if trims.last_active != trim::ActiveTrim::None {
                    let mut trm_buf = [0u8; 9];
                    let val = match trims.last_active {
                        trim::ActiveTrim::Roll => trims.values.roll,
                        trim::ActiveTrim::Pitch => trims.values.pitch,
                        trim::ActiveTrim::Throttle => trims.values.throttle,
                        trim::ActiveTrim::Yaw => trims.values.yaw,
                        trim::ActiveTrim::None => 0,
                    };
                    let trm_str = format_trim(trims.last_active, val, &mut trm_buf);
                    Text::new(trm_str, Point::new(2, 62), text_style_small).draw(&mut lcd).ok();
                    Text::new("Hold OK:Menu", Point::new(50, 62), text_style_small).draw(&mut lcd).ok();
                } else {
                    Text::new("P1/4", Point::new(2, 62), text_style_small).draw(&mut lcd).ok();
                    Text::new("Hold OK:Menu", Point::new(50, 62), text_style_small).draw(&mut lcd).ok();
                }
            }

            1 => {
                // --- Page 1: 14-Channel Dual Column Monitor (y = 12..53) ---
                let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
                let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
                let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);

                // Col 0 (CH 1..7) at x = 2..62, Col 1 (CH 8..14) at x = 66..126
                for col in 0..2 {
                    let col_x = if col == 0 { 2 } else { 66 };
                    let start_ch = col * 7;

                    for row in 0..7 {
                        let ch = start_ch + row;
                        let y = 12 + (row as i32 * 6);

                        // Label: " 1:" .. "14:"
                        let mut lbl_buf = *b"  :";
                        if ch + 1 >= 10 {
                            lbl_buf[0] = b'1';
                            lbl_buf[1] = b'0' + ((ch + 1) % 10) as u8;
                        } else {
                            lbl_buf[0] = b' ';
                            lbl_buf[1] = b'1' + ch as u8;
                        }
                        let lbl_str = core::str::from_utf8(&lbl_buf).unwrap_or("??:");
                        Text::new(lbl_str, Point::new(col_x, y + 5), text_style_small).draw(&mut lcd).ok();

                        // Bar box: width 22, height 5 (occupies y+1 .. y+5, aligned with text)
                        let us = rf_chs[ch].clamp(1000, 2000);
                        Rectangle::new(Point::new(col_x + 13, y + 1), Size::new(22, 5))
                            .into_styled(border_style)
                            .draw(&mut lcd)
                            .ok();
                        let fill_w = (((us - 1000) as u32 * 20) / 1000).min(20);
                        if fill_w > 0 {
                            Rectangle::new(Point::new(col_x + 14, y + 2), Size::new(fill_w, 3))
                                .into_styled(fill_style)
                                .draw(&mut lcd)
                                .ok();
                        }

                        // Value: "1500"
                        let mut val_buf = [0u8; 4];
                        u16_to_dec_4(us, &mut val_buf);
                        let val_str = core::str::from_utf8(&val_buf).unwrap_or("1500");
                        Text::new(val_str, Point::new(col_x + 37, y + 5), text_style_small).draw(&mut lcd).ok();
                    }
                }

                // Vertical divider line between columns
                Line::new(Point::new(64, 12), Point::new(64, 53))
                    .into_styled(sep_style)
                    .draw(&mut lcd)
                    .ok();

                // Separator above footer (at y = 55, giving 2px clearance below row 6 which ends at y = 53)
                Line::new(Point::new(0, 55), Point::new(127, 55))
                    .into_styled(sep_style)
                    .draw(&mut lcd)
                    .ok();

                // Footer (using FONT_4X6 at baseline 62, giving 2px clearance below line 55)
                if is_binding {
                    Text::new("[ESC] Finish Bind", Point::new(26, 62), text_style_small).draw(&mut lcd).ok();
                } else {
                    Text::new("P2/4", Point::new(2, 62), text_style_small).draw(&mut lcd).ok();
                    Text::new("14-CH MONITOR", Point::new(40, 62), text_style_small).draw(&mut lcd).ok();
                }
            }

            2 => {
                // --- Page 2: Model Dashboard (y = 13..53) ---
                let active = storage.active_model();

                // Line 1 (y = 21): Model Name & Type
                let m_name = core::str::from_utf8(&active.name).unwrap_or("MODEL");
                Text::new(m_name, Point::new(2, 21), text_style).draw(&mut lcd).ok();

                let type_str = match active.model_type {
                    0 => "AIRPLANE",
                    1 => "GLIDER",
                    2 => "HELI",
                    _ => "QUAD",
                };
                Text::new(type_str, Point::new(74, 21), text_style).draw(&mut lcd).ok();

                // Line 2 (y = 31): Receiver ID & AFHDS 2A
                let mut rx_buf = [b'0'; 8];
                u32_to_hex(active.rx_id, &mut rx_buf);
                let rx_str = core::str::from_utf8(&rx_buf).unwrap_or("00000000");
                Text::new("RxID:", Point::new(2, 31), text_style).draw(&mut lcd).ok();
                Text::new(rx_str, Point::new(36, 31), text_style).draw(&mut lcd).ok();

                // Line 3 (y = 41): Throttle Curve info
                let c_pts = if active.thr_curve_pts == 9 { "9-PT" } else { "5-PT" };
                let c_sm = if active.thr_curve_smooth != 0 { "SMOOTH" } else { "LINEAR" };
                Text::new("TCrv:", Point::new(2, 41), text_style).draw(&mut lcd).ok();
                Text::new(c_pts, Point::new(36, 41), text_style).draw(&mut lcd).ok();
                Text::new(c_sm, Point::new(74, 41), text_style).draw(&mut lcd).ok();

                // Line 4 (y = 51): Telemetry Voltage & RSSI
                if telem.connected {
                    let mut rxv_buf = [0u8; 6];
                    let rxv_str = format_vbat(telem.rx_voltage_mv, &mut rxv_buf);
                    Text::new("RX:", Point::new(2, 51), text_style).draw(&mut lcd).ok();
                    Text::new(rxv_str, Point::new(22, 51), text_style).draw(&mut lcd).ok();

                    let mut r_buf = *b"RSSI:   %";
                    let r = telem.rssi.min(100);
                    if r >= 10 {
                        r_buf[5] = b'0' + (r / 10);
                        r_buf[6] = b'0' + (r % 10);
                    } else {
                        r_buf[5] = b' ';
                        r_buf[6] = b'0' + r;
                    }
                    let r_str = core::str::from_utf8(&r_buf).unwrap_or("RSSI:--%");
                    Text::new(r_str, Point::new(64, 51), text_style).draw(&mut lcd).ok();
                } else if is_crsf {
                    Text::new("CRSF: DISCONNECTED", Point::new(2, 51), text_style).draw(&mut lcd).ok();
                } else {
                    Text::new("AFHDS2A: DISCONNECTED", Point::new(2, 51), text_style).draw(&mut lcd).ok();
                }

                // Separator above footer
                Line::new(Point::new(0, 55), Point::new(127, 55))
                    .into_styled(sep_style)
                    .draw(&mut lcd)
                    .ok();

                // Footer
                if is_binding {
                    Text::new("[ESC] Finish Bind", Point::new(26, 62), text_style_small).draw(&mut lcd).ok();
                } else {
                    Text::new("P3/4", Point::new(2, 62), text_style_small).draw(&mut lcd).ok();
                    Text::new("MODEL DASHBOARD", Point::new(36, 62), text_style_small).draw(&mut lcd).ok();
                }
            }

            _ => {
                // --- Page 3: Telemetry & RF Diagnostics (y = 12..53) ---
                let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);

                // Left Column (x = 2..62)
                // Row 1 (y = 21): RSSI
                if telem.connected {
                    let mut r_buf = *b"RSSI:   %";
                    let r = telem.rssi.min(100);
                    if r >= 10 {
                        r_buf[5] = b'0' + (r / 10);
                        r_buf[6] = b'0' + (r % 10);
                    } else {
                        r_buf[5] = b' ';
                        r_buf[6] = b'0' + r;
                    }
                    let r_str = core::str::from_utf8(&r_buf).unwrap_or("RSSI:--%");
                    Text::new(r_str, Point::new(2, 21), text_style).draw(&mut lcd).ok();
                } else {
                    Text::new("RSSI: --%", Point::new(2, 21), text_style).draw(&mut lcd).ok();
                }

                // Row 2 (y = 31): Link Status
                let link_str = if telem.connected { "LINK: OK" } else { "LINK: DISC" };
                Text::new(link_str, Point::new(2, 31), text_style).draw(&mut lcd).ok();

                // Row 3 (y = 41): Packets Sent
                let mut tx_p_buf = [b' '; 5];
                u32_to_dec_5(telem.packets_sent, &mut tx_p_buf);
                let tx_p_str = core::str::from_utf8(&tx_p_buf).unwrap_or("    0");
                Text::new("TX:", Point::new(2, 41), text_style).draw(&mut lcd).ok();
                Text::new(tx_p_str, Point::new(24, 41), text_style).draw(&mut lcd).ok();

                // Row 4 (y = 51): Packets Received
                let mut rx_p_buf = [b' '; 5];
                u32_to_dec_5(telem.packets_received, &mut rx_p_buf);
                let rx_p_str = core::str::from_utf8(&rx_p_buf).unwrap_or("    0");
                Text::new("RX:", Point::new(2, 51), text_style).draw(&mut lcd).ok();
                Text::new(rx_p_str, Point::new(24, 51), text_style).draw(&mut lcd).ok();

                // Vertical divider line between columns
                Line::new(Point::new(64, 12), Point::new(64, 53))
                    .into_styled(sep_style)
                    .draw(&mut lcd)
                    .ok();

                // Right Column (x = 66..126)
                // Row 1 (y = 21): RX Battery Voltage
                Text::new("RX:", Point::new(66, 21), text_style).draw(&mut lcd).ok();
                if telem.connected {
                    let mut rxv_buf = [0u8; 6];
                    let rxv_str = format_vbat(telem.rx_voltage_mv, &mut rxv_buf);
                    Text::new(rxv_str, Point::new(88, 21), text_style).draw(&mut lcd).ok();
                } else {
                    Text::new("----", Point::new(88, 21), text_style).draw(&mut lcd).ok();
                }

                // Row 2 (y = 31): TX Battery Voltage
                Text::new("TX:", Point::new(66, 31), text_style).draw(&mut lcd).ok();
                let mut txv_buf = [0u8; 6];
                let txv_str = format_vbat(state.battery_mv, &mut txv_buf);
                Text::new(txv_str, Point::new(88, 31), text_style).draw(&mut lcd).ok();

                // Row 3 (y = 41): Min Session RSSI
                Text::new("mRSS:", Point::new(66, 41), text_style).draw(&mut lcd).ok();
                if telem_seen {
                    let mut mr_buf = *b"    ";
                    let r = min_rssi.min(100);
                    if r == 100 {
                        mr_buf = *b"100%";
                    } else if r >= 10 {
                        mr_buf[0] = b' ';
                        mr_buf[1] = b'0' + (r / 10);
                        mr_buf[2] = b'0' + (r % 10);
                        mr_buf[3] = b'%';
                    } else {
                        mr_buf[0] = b' ';
                        mr_buf[1] = b' ';
                        mr_buf[2] = b'0' + r;
                        mr_buf[3] = b'%';
                    }
                    let mr_str = core::str::from_utf8(&mr_buf).unwrap_or(" --%");
                    Text::new(mr_str, Point::new(98, 41), text_style).draw(&mut lcd).ok();
                } else {
                    Text::new(" --%", Point::new(98, 41), text_style).draw(&mut lcd).ok();
                }

                // Row 4 (y = 51): Min Session RX Voltage
                Text::new("mRX:", Point::new(66, 51), text_style).draw(&mut lcd).ok();
                if min_rx_v_mv != 0xFFFF && min_rx_v_mv > 0 {
                    let mut mrxv_buf = [0u8; 6];
                    let mrxv_str = format_vbat(min_rx_v_mv, &mut mrxv_buf);
                    Text::new(mrxv_str, Point::new(92, 51), text_style).draw(&mut lcd).ok();
                } else {
                    Text::new("----", Point::new(92, 51), text_style).draw(&mut lcd).ok();
                }

                // Separator above footer
                Line::new(Point::new(0, 55), Point::new(127, 55))
                    .into_styled(sep_style)
                    .draw(&mut lcd)
                    .ok();

                // Footer
                if is_binding {
                    Text::new("[ESC] Finish Bind", Point::new(26, 62), text_style_small).draw(&mut lcd).ok();
                } else {
                    Text::new("P4/4", Point::new(2, 62), text_style_small).draw(&mut lcd).ok();
                    Text::new("TELEMETRY SENSORS", Point::new(28, 62), text_style_small).draw(&mut lcd).ok();
                }
            }
        }

        // Flush frame to ST7567 LCD
        lcd.flush();
    }
}
