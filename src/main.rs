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
mod watchdog;

use display::St7567;


/// High-rate flight pipeline state and outputs.
struct FlightSnapshot {
    state: input::InputState,
    rf_chs: [u16; 14],
    telem: rf::afhds2a::TelemetryData,
    is_binding: bool,
}

/// Flight pipeline context: manages stick sampling, mixing, and RF/USB publication.
struct FlightPipeline {
    prev_armed: bool,
    prev_active_model: u8,
}

impl FlightPipeline {
    fn new(storage: &storage::RadioStorage, init_state: &input::InputState) -> Self {
        let active = storage.active_model();
        let prev_armed = if active.arm_switch > 0 && active.arm_switch <= 10 {
            mixer::is_switch_active(active.arm_switch, &init_state.switches)
        } else {
            false
        };
        Self {
            prev_armed,
            prev_active_model: storage.radio.active_model,
        }
    }

    /// High-rate flight pipeline execution tick (multi-kHz execution speed).
    /// Polls physical sticks, evaluates throttle curves, computes matrix mixer,
    /// checks arm status, and publishes channels to RF (AFHDS 2A / CRSF) and USB.
    #[inline(always)]
    fn tick(
        &mut self,
        now: u32,
        storage: &storage::RadioStorage,
        trims: &trim::TrimController,
        menu_active: bool,
        buzzer: &mut buzzer::Buzzer,
    ) -> FlightSnapshot {
        // 1. Poll continuous DMA analog and digital inputs (sub-microsecond)
        let state = input::poll();
        let active_model = storage.active_model();

        // 2. Resynchronize arm state when active model changes or when exiting settings menu
        if storage.radio.active_model != self.prev_active_model {
            self.prev_active_model = storage.radio.active_model;
            self.prev_armed = if active_model.arm_switch > 0 && active_model.arm_switch <= 10 {
                mixer::is_switch_active(active_model.arm_switch, &state.switches)
            } else {
                false
            };
        }

        // 3. Check configured Arm Switch condition and play Armed/Disarmed chimes
        if active_model.arm_switch > 0 && active_model.arm_switch <= 10 {
            let is_armed = mixer::is_switch_active(active_model.arm_switch, &state.switches);
            if is_armed != self.prev_armed {
                self.prev_armed = is_armed;
                if !menu_active {
                    if is_armed {
                        buzzer.chime_armed();
                    } else {
                        buzzer.chime_disarmed();
                    }
                }
            }
        }

        // 4. Evaluate active model throttle curve (normalized 0..1000)
        let thr_input = ((state.sticks.throttle + 1000) / 2).clamp(0, 1000) as u16;
        let thr_curved = curve::evaluate_curve(
            thr_input,
            active_model.thr_curve_pts,
            active_model.thr_curve_smooth != 0,
            &active_model.thr_curve,
        );

        // 5. Compute all 14 channels via 4-stage pipeline (D/R, Expo, Matrix Mixer, Trims, Reversing)
        let rf_chs = mixer::compute_channels(
            state.sticks.roll,
            state.sticks.pitch,
            thr_curved,
            state.sticks.yaw,
            &[state.pots.vr1, state.pots.vr2],
            &state.switches,
            active_model,
            trims,
            storage.radio.throttle_trim,
        );

        // 6. Publish channels to active RF subsystem
        let is_crsf = active_model.rf_protocol == 1;
        let sim_mode = usb::is_sim_mode();

        if is_crsf {
            let crsf_active = !sim_mode;
            crsf::set_enabled(crsf_active, active_model.crsf_baud);
            if crsf_active {
                crsf::update_channels(now, &rf_chs);
                crsf::poll_telemetry(now);
            }
            rf::set_silenced(true);
        } else {
            crsf::set_enabled(false, 0);
            rf::set_silenced(sim_mode);
            if !sim_mode {
                rf::set_channels(&rf_chs);
            }
        }

        // 7. Map telemetry data based on active protocol
        let telem = if is_crsf {
            let ct = crsf::get_telemetry();
            rf::afhds2a::TelemetryData {
                connected: ct.connected,
                rssi: ct.uplink_link_quality,
                rx_voltage_mv: ct.rx_battery_mv,
                packets_sent: 0,
                packets_received: 0,
            }
        } else {
            rf::get_telemetry()
        };
        let is_binding = !is_crsf && rf::is_binding();

        // 8. Poll USB subsystem (Joystick HID @ 100Hz, Serial CLI / Telemetry)
        usb::poll(now, &rf_chs, &state.switches, &telem, state.battery_mv);

        FlightSnapshot {
            state,
            rf_chs,
            telem,
            is_binding,
        }
    }
}

/// Background idle and UI context: tracks inactivity, backlight timers, user input, and screen rendering.
struct BackgroundIdleManager {
    prev_stick_samples: [u16; 6],
    prev_switches: input::Switches,
    prev_bind_key: bool,
    bind_hold_ms: u32,
    bind_was_held: bool,
    ok_hold_ms: u16,
    bl_timer_ms: u32,
    inactivity_timer_ms: u32,
    inactivity_beep_timer: u32,
    last_display_ms: u32,
    menu_was_active: bool,
}

impl BackgroundIdleManager {
    fn new(init_state: &input::InputState, bind_on_boot: bool) -> Self {
        Self {
            prev_stick_samples: [
                init_state.raw[0],
                init_state.raw[1],
                init_state.raw[2],
                init_state.raw[3],
                init_state.raw[6],
                init_state.raw[7],
            ],
            prev_switches: init_state.switches,
            prev_bind_key: bind_on_boot,
            bind_hold_ms: 0,
            bind_was_held: false,
            ok_hold_ms: 0,
            bl_timer_ms: 30_000,
            inactivity_timer_ms: 0,
            inactivity_beep_timer: 0,
            last_display_ms: 0,
            menu_was_active: false,
        }
    }

    /// Background idle execution tick: manages timers, power save, bind key gestures, Flash saves, and display frames.
    fn tick(
        &mut self,
        now: u32,
        dt_ms: u16,
        keys: u16,
        flight: &FlightSnapshot,
        pipeline: &mut FlightPipeline,
        storage: &mut storage::RadioStorage,
        trims: &mut trim::TrimController,
        lcd: &mut St7567,
        buzzer: &mut buzzer::Buzzer,
        dashboard: &mut ui::dashboard::DashboardController,
        menu_controller: &mut menu::MenuController,
        calib_wizard: &mut calib::CalibWizard,
        rf_ok: bool,
    ) {
        let menu_active = menu_controller.is_active() || calib_wizard.is_active();

        // 1. Resynchronize arm tracking when exiting settings menu
        if self.menu_was_active && !menu_active {
            let active = storage.active_model();
            pipeline.prev_armed = if active.arm_switch > 0 && active.arm_switch <= 10 {
                mixer::is_switch_active(active.arm_switch, &flight.state.switches)
            } else {
                false
            };
        }
        self.menu_was_active = menu_active;

        // 2. Physical activity & inactivity tracking
        let stick_moved = (flight.state.raw[0] as i32 - self.prev_stick_samples[0] as i32).abs() > 30
            || (flight.state.raw[1] as i32 - self.prev_stick_samples[1] as i32).abs() > 30
            || (flight.state.raw[2] as i32 - self.prev_stick_samples[2] as i32).abs() > 30
            || (flight.state.raw[3] as i32 - self.prev_stick_samples[3] as i32).abs() > 30
            || (flight.state.raw[6] as i32 - self.prev_stick_samples[4] as i32).abs() > 40
            || (flight.state.raw[7] as i32 - self.prev_stick_samples[5] as i32).abs() > 40;
        self.prev_stick_samples[0] = flight.state.raw[0];
        self.prev_stick_samples[1] = flight.state.raw[1];
        self.prev_stick_samples[2] = flight.state.raw[2];
        self.prev_stick_samples[3] = flight.state.raw[3];
        self.prev_stick_samples[4] = flight.state.raw[6];
        self.prev_stick_samples[5] = flight.state.raw[7];

        let sw_changed = flight.state.switches != self.prev_switches;
        self.prev_switches = flight.state.switches;

        let user_active = keys != 0 || stick_moved || sw_changed;

        // 3. Backlight auto-dim timeout tracking
        if user_active {
            let timeout_ms: u32 = match storage.radio.backlight_timeout {
                1 => 15_000,
                2 => 30_000,
                3 => 60_000,
                _ => 0,
            };
            self.bl_timer_ms = timeout_ms;
            lcd.set_backlight_level(storage.radio.backlight_brightness * 10);
        } else if storage.radio.backlight_timeout != 0 && dt_ms > 0 {
            if self.bl_timer_ms > dt_ms as u32 {
                self.bl_timer_ms -= dt_ms as u32;
            } else {
                self.bl_timer_ms = 0;
                lcd.set_backlight_level(0);
            }
        }

        // 4. Radio Inactivity Alarm (10 minutes without physical control activity)
        if user_active {
            self.inactivity_timer_ms = 0;
            self.inactivity_beep_timer = 0;
        } else if dt_ms > 0 {
            self.inactivity_timer_ms = self.inactivity_timer_ms.saturating_add(dt_ms as u32);
            if self.inactivity_timer_ms >= 600_000 {
                if self.inactivity_beep_timer >= 30_000 {
                    self.inactivity_beep_timer = 0;
                    buzzer.warn_inactivity();
                } else {
                    self.inactivity_beep_timer += dt_ms as u32;
                }
            }
        }

        // 5. Bind Key (PF2) and Cancel key handling
        let bind_key_raw = (keys & (1 << 12)) != 0;
        let bind_pressed = bind_key_raw && !self.prev_bind_key;
        self.prev_bind_key = bind_key_raw;

        let cancel_key = (keys & (1 << 11)) != 0;

        if flight.is_binding {
            if cancel_key || bind_pressed {
                rf::set_bind_mode(false);
                buzzer.click();
            }
        } else if menu_active {
            self.bind_hold_ms = 0;
            self.bind_was_held = false;
        } else {
            if bind_key_raw {
                self.bind_hold_ms = self.bind_hold_ms.saturating_add(dt_ms as u32);
                if self.bind_hold_ms >= 1000 && !self.bind_was_held {
                    rf::set_bind_mode(true);
                    buzzer.play_tone(2400, 150);
                    self.bind_was_held = true;
                }
            } else {
                if !self.bind_was_held && self.bind_hold_ms >= 40 {
                    dashboard.next_page(buzzer);
                }
                self.bind_hold_ms = 0;
                self.bind_was_held = false;
            }
        }

        // 6. Persist newly bound RX ID safely to Flash outside ISR (inhibit while armed)
        if let Some(new_rx_id) = rf::take_pending_rx_save() {
            if new_rx_id != 0 && new_rx_id != 0xFFFF_FFFF && storage.active_model().rx_id != new_rx_id {
                storage.active_model_mut().rx_id = new_rx_id;
                if !pipeline.prev_armed {
                    storage::save_active_model(storage);
                    buzzer.play_tone_pattern(2400, 70, 50, 2);
                }
            }
        }

        // 7. Long-press OK (1.2s) from flight dashboard opens Settings Menu
        if !menu_active {
            if (keys & (1 << 10)) != 0 {
                self.ok_hold_ms = self.ok_hold_ms.saturating_add(dt_ms);
                if self.ok_hold_ms >= 1200 {
                    menu_controller.open(buzzer);
                    self.ok_hold_ms = 0;
                }
            } else {
                self.ok_hold_ms = 0;
            }
        }

        // 8. Display Frame Rendering (~30 Hz)
        let run_display = now.wrapping_sub(self.last_display_ms) >= 33;
        if !run_display {
            return;
        }
        self.last_display_ms = now;

        if menu_controller.is_active() {
            menu_controller.update(
                lcd,
                keys,
                storage,
                trims,
                &flight.state.raw,
                &flight.rf_chs,
                buzzer,
            );

            if menu_controller.request_calibration {
                calib_wizard.start(buzzer);
                menu_controller.request_calibration = false;
            }

            if menu_controller.request_bind {
                rf::set_bind_mode(true);
                menu_controller.request_bind = false;
                buzzer.play_tone(2400, 150);
            }

            lcd.flush();
        } else if calib_wizard.is_active() {
            calib_wizard.update(lcd, storage, &flight.state.raw, keys, dt_ms.max(20), buzzer);
            lcd.flush();
        } else {
            dashboard.render(
                lcd,
                &flight.state,
                storage,
                trims,
                &flight.rf_chs,
                rf_ok,
                flight.is_binding,
                &flight.telem,
                buzzer,
            );
            lcd.flush();
        }
    }
}

#[entry]
fn main() -> ! {
    // 0. Standalone feed immediately refreshes any watchdog running across soft reboot
    watchdog::feed();

    // 1. MCU Profile & Fast DFU Bootloader Check
    let mcu_profile = chip::get_mcu_profile();
    boot::check_dfu_entry(&mcu_profile);

    // 2. Initialize System Clock to 48 MHz using external 8 MHz crystal (HSE) + PLL
    // and wait for 40 kHz LSI oscillator to stabilize
    chip::init_system_clock();

    // Initialize SysTick 1.000 ms hardware monotonic timekeeper
    time::init();

    // 3. Initialize ST7567 128×64 LCD (clears framebuffer and flushes before turning on backlight)
    let mut lcd = St7567::new();

    // 4. Start deterministic 2.0s Hardware Watchdog (IWDG) via PAC
    watchdog::start();

    // 5. Initialize ADC1 + DMA1 autonomous continuous scanner
    adc::init();
    watchdog::feed();

    // 6. Initialize input calibration and capture resting stick centers
    input::init();
    watchdog::feed();

    // 7. Initialize A7105 RF transceiver & AFHDS 2A stack
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
    watchdog::feed();

    // 8. Initialize Buzzer & Digital Trims
    let mut buzzer = buzzer::Buzzer::new();
    buzzer.init();
    watchdog::feed();

    // 9. Load persistent radio storage and 20-model configuration
    let mut storage = storage::RadioStorage::empty();
    storage::load_storage_into(&mut storage);
    buzzer.enabled = storage.radio.audio_enabled != 0;
    buzzer.tone_style = buzzer::ToneStyle::from_u8(storage.radio.tone_style);
    buzzer.chime_welcome();
    watchdog::feed();

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
    watchdog::feed();

    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
    let sep_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    // 10. Pre-flight Startup Safety Check: Throttle at idle and switches in safe (UP) positions
    if !calib_wizard.is_active() {
        let mut preflight_beep_timer: u32 = 0;
        let mut preflight_last_render: u32 = 0;
        let mut warned = false;

        loop {
            watchdog::feed();

            let now = time::millis();
            let state = input::poll();
            let keys = boot::scan_keys();

            // Adaptive throttle idle check:
            // When calibrated, state.sticks.throttle is normalized (-1000..+1000). Idle is < -900.
            // When uncalibrated (default factory endpoints min <= 200), check raw ADC channel 2 (Throttle vertical).
            // FlySky gimbals rest at idle below ~1400 ADC counts (ADC range 0..4095).
            let is_calibrated = storage.radio.sticks[2].min > 200;
            let thr_unsafe = if is_calibrated {
                state.sticks.throttle > -900
            } else {
                state.raw[2] > 1400
            };

            let sa_unsafe = state.switches.sa != input::SwitchPos::Up;
            let sb_unsafe = state.switches.sb != input::SwitchPos::Up;
            let sc_unsafe = state.switches.sc != input::SwitchPos::Up;
            let sd_unsafe = state.switches.sd != input::SwitchPos::Up;
            let sw_unsafe = sa_unsafe || sb_unsafe || sc_unsafe || sd_unsafe;

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
            usb::poll(now, &[1500, 1500, 1000, 1500, 1000, 1000, 1500, 1500, 1000, 1000, 1500, 1500, 1500, 1500], &state.switches, &rf::get_telemetry(), state.battery_mv);

            if now.wrapping_sub(preflight_beep_timer) >= 800 {
                preflight_beep_timer = now;
                buzzer.warn_preflight();
            }

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

    // Initialize execution tiers
    let init_state = input::poll();
    let mut pipeline = FlightPipeline::new(&storage, &init_state);
    let mut idle_manager = BackgroundIdleManager::new(&init_state, bind_on_boot);
    let mut dashboard = ui::dashboard::DashboardController::new();
    let mut last_tick_ms: u32 = 0;

    // Main event loop: decoupled into high-rate flight pipeline and 30 Hz background idle tasks
    loop {
        // Kick watchdog at loop start to guarantee active execution
        watchdog::feed();

        let now = time::millis();
        let dt_ms = (now.wrapping_sub(last_tick_ms)).min(100) as u16;

        let keys = boot::scan_keys();
        if dt_ms > 0 {
            last_tick_ms = now;
            buzzer.tick(dt_ms);
            trims.update(keys, dt_ms, &mut buzzer);
        }

        let menu_active = menu_controller.is_active() || calib_wizard.is_active();

        // Tier 1: High-Rate Flight Pipeline Tick (multi-kHz)
        let flight_snapshot = pipeline.tick(now, &storage, &trims, menu_active, &mut buzzer);

        // Tier 2: Background Idle & UI Tick (30 Hz rate-governed)
        idle_manager.tick(
            now,
            dt_ms,
            keys,
            &flight_snapshot,
            &mut pipeline,
            &mut storage,
            &mut trims,
            &mut lcd,
            &mut buzzer,
            &mut dashboard,
            &mut menu_controller,
            &mut calib_wizard,
            rf_ok,
        );
    }
}
