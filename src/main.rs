#![no_std]
#![no_main]

use panic_halt as _;
use stm32f0xx_hal as _;

use cortex_m_rt::entry;
use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle},
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
mod ui;
pub use ui::menu;
mod mixer;
mod rf;
mod storage;
mod time;
mod trim;
mod usb;

use display::St7567;


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
    let mut storage = storage::RadioStorage::empty();
    storage::load_storage_into(&mut storage);
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
    let mut dashboard = ui::dashboard::DashboardController::new();
    let mut bind_hold_ms: u32 = 0;
    let mut bind_was_held: bool = false;
    let mut inactivity_timer_ms: u32 = 0;
    let mut inactivity_beep_timer: u32 = 0;
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
                    dashboard.next_page(&mut buzzer);
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
                calib_wizard.update(&mut lcd, &mut storage, &state.raw, keys, dt_ms.max(20), &mut buzzer);
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
        dashboard.render(
            &mut lcd,
            &state,
            &storage,
            &trims,
            &rf_chs,
            rf_ok,
            is_binding,
            &telem,
            &mut buzzer,
        );

        // Flush frame to ST7567 LCD
        lcd.flush();
    }
}
