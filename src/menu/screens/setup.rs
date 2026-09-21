//! Radio settings and Protocol configuration screens.

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};

use crate::buzzer::Buzzer;
use crate::display::St7567;
use crate::menu::format::u32_to_hex;
use crate::menu::widgets;
use crate::menu::{MenuController, MenuState, NavKeys};
use crate::storage::{self, RadioStorage};
use crate::trim::TrimController;

pub fn update_radio_setup(
    ctrl: &mut MenuController,
    lcd: &mut St7567,
    keys: &NavKeys,
    storage: &mut RadioStorage,
    trims: &mut TrimController,
    buzzer: &mut Buzzer,
) {
    if keys.cancel {
        ctrl.state = MenuState::MainMenu;
        ctrl.selected_item = 7;
        ctrl.scroll_offset = 4;
        ctrl.waiting_release = true;
        buzzer.click();
        return;
    }
    const SETUP_ITEMS: usize = 8;

    widgets::navigate_4slot_list(
        &mut ctrl.selected_item,
        &mut ctrl.scroll_offset,
        SETUP_ITEMS,
        keys.up,
        keys.down,
        buzzer,
    );

    if keys.ok {
        buzzer.click();
        match ctrl.selected_item {
            0 => {
                storage.radio.throttle_trim = (storage.radio.throttle_trim + 1) % 3;
                trims.throttle_enabled = storage.radio.throttle_trim != 0;
                storage::save_storage(storage);
            }
            1 => {
                storage.radio.audio_enabled = if storage.radio.audio_enabled == 0 { 1 } else { 0 };
                buzzer.enabled = storage.radio.audio_enabled != 0;
                storage::save_storage(storage);
            }
            2 => {
                storage.radio.backlight_timeout = (storage.radio.backlight_timeout + 1) % 4;
                storage::save_storage(storage);
            }
            3 => {
                storage.radio.backlight_brightness = if storage.radio.backlight_brightness >= 10 {
                    1
                } else {
                    storage.radio.backlight_brightness + 1
                };
                lcd.set_backlight_level(storage.radio.backlight_brightness * 10);
                storage::save_storage(storage);
            }
            4 => {
                storage.radio.lcd_contrast = if storage.radio.lcd_contrast >= 50 {
                    20
                } else {
                    (storage.radio.lcd_contrast + 3).min(50)
                };
                lcd.set_contrast(storage.radio.lcd_contrast);
                storage::save_storage(storage);
            }
            5 => {
                storage.radio.vbat_warn_deci = if storage.radio.vbat_warn_deci >= 50 {
                    40
                } else {
                    storage.radio.vbat_warn_deci + 1
                };
                storage::save_storage(storage);
            }
            6 => {
                storage.radio.usb_mode = (storage.radio.usb_mode + 1) % 4;
                storage::save_storage(storage);
                crate::usb::init(storage.radio.usb_mode);
            }
            7 => {
                storage.radio.ext_module_pwr = if storage.radio.ext_module_pwr == 0 { 1 } else { 0 };
                crate::crsf::set_power_polarity(storage.radio.ext_module_pwr == 0);
                storage::save_storage(storage);
            }
            _ => {}
        }
    }

    widgets::draw_header(lcd, "RADIO SETUP");

    for slot in 0..4 {
        let idx = ctrl.scroll_offset + slot;
        if idx >= SETUP_ITEMS {
            break;
        }
        let is_sel = idx == ctrl.selected_item;

        match idx {
            0 => {
                let val_str = match storage.radio.throttle_trim {
                    1 => "IDLE",
                    2 => "LINEAR",
                    _ => "OFF (Lock)",
                };
                widgets::draw_list_row(lcd, slot, is_sel, "Thr Trim:", Some(val_str), 62);
            }
            1 => {
                let beeper_str = if storage.radio.audio_enabled != 0 { "ENABLED" } else { "MUTED" };
                widgets::draw_list_row(lcd, slot, is_sel, "Beeper:", Some(beeper_str), 62);
            }
            2 => {
                let timer_str = match storage.radio.backlight_timeout {
                    1 => "15 SEC",
                    2 => "30 SEC",
                    3 => "60 SEC",
                    _ => "ALWAYS ON",
                };
                widgets::draw_list_row(lcd, slot, is_sel, "BL Timer:", Some(timer_str), 62);
            }
            3 => {
                let mut b_buf = *b"   %";
                let pct = (storage.radio.backlight_brightness * 10).min(100);
                if pct == 100 {
                    b_buf = *b"100%";
                } else {
                    b_buf[0] = b' ';
                    b_buf[1] = b'0' + (pct / 10);
                    b_buf[2] = b'0';
                    b_buf[3] = b'%';
                }
                let b_str = core::str::from_utf8(&b_buf).unwrap_or("100%");
                widgets::draw_list_row(lcd, slot, is_sel, "BL Level:", Some(b_str), 62);
            }
            4 => {
                let mut c_buf = *b"00";
                c_buf[0] = b'0' + (storage.radio.lcd_contrast / 10);
                c_buf[1] = b'0' + (storage.radio.lcd_contrast % 10);
                let c_str = core::str::from_utf8(&c_buf).unwrap_or("37");
                widgets::draw_list_row(lcd, slot, is_sel, "Contrast:", Some(c_str), 62);
            }
            5 => {
                let mut v_buf = *b"0.0V";
                v_buf[0] = b'0' + (storage.radio.vbat_warn_deci / 10);
                v_buf[2] = b'0' + (storage.radio.vbat_warn_deci % 10);
                let v_str = core::str::from_utf8(&v_buf).unwrap_or("4.4V");
                widgets::draw_list_row(lcd, slot, is_sel, "Bat Warn:", Some(v_str), 62);
            }
            6 => {
                let usb_str = match storage.radio.usb_mode {
                    1 => "JOYSTICK",
                    2 => "SERIAL",
                    3 => "COMPOSITE",
                    _ => "OFF",
                };
                widgets::draw_list_row(lcd, slot, is_sel, "USB Mode:", Some(usb_str), 62);
            }
            7 => {
                let pwr_str = if storage.radio.ext_module_pwr == 0 { "HIGH (N)" } else { "LOW (P)" };
                widgets::draw_list_row(lcd, slot, is_sel, "PC13 Pwr:", Some(pwr_str), 62);
            }
            _ => {}
        }
    }

    widgets::draw_footer(lcd, "[OK] Toggle/Cycle   [ESC] Back");
}

pub fn update_rx_setup(
    ctrl: &mut MenuController,
    lcd: &mut St7567,
    keys: &NavKeys,
    storage: &mut RadioStorage,
    buzzer: &mut Buzzer,
) {
    if keys.cancel {
        ctrl.state = MenuState::MainMenu;
        ctrl.selected_item = 8;
        ctrl.scroll_offset = 5;
        ctrl.waiting_release = true;
        buzzer.click();
        return;
    }

    let active_idx = storage.radio.active_model as usize;
    let proto = storage.models[active_idx].rf_protocol;

    if proto == 0 {
        // AFHDS 2A view
        if keys.up || keys.down {
            storage.models[active_idx].rf_protocol = 1;
            ctrl.selected_item = 0;
            storage::save_storage(storage);
            buzzer.play_tone(2200, 30);
        }

        if keys.ok {
            buzzer.click();
            ctrl.request_bind = true;
            ctrl.state = MenuState::Closed;
            return;
        }
    } else {
        // CRSF / ELRS view: selected_item 0 = Proto, 1 = Baud Rate, 2 = Configure Module
        if keys.ok {
            if ctrl.selected_item == 2 {
                ctrl.state = MenuState::ElrsSetup;
                ctrl.selected_item = 0;
                ctrl.scroll_offset = 0;
                crate::crsf::start_config();
                buzzer.click();
                return;
            } else {
                ctrl.selected_item = (ctrl.selected_item + 1) % 3;
                buzzer.click();
            }
        }

        if ctrl.selected_item == 0 {
            if keys.up || keys.down {
                storage.models[active_idx].rf_protocol = 0;
                ctrl.selected_item = 0;
                storage::save_storage(storage);
                buzzer.play_tone(2200, 30);
            }
        } else if ctrl.selected_item == 1 {
            if keys.up {
                storage.models[active_idx].crsf_baud = if storage.models[active_idx].crsf_baud > 0 {
                    storage.models[active_idx].crsf_baud - 1
                } else {
                    3
                };
                storage::save_storage(storage);
                buzzer.play_tone(2200, 30);
            }
            if keys.down {
                storage.models[active_idx].crsf_baud = (storage.models[active_idx].crsf_baud + 1) % 4;
                storage::save_storage(storage);
                buzzer.play_tone(2200, 30);
            }
        } else if ctrl.selected_item == 2 {
            if keys.up {
                ctrl.selected_item = 1;
                buzzer.play_tone(2200, 30);
            } else if keys.down {
                ctrl.selected_item = 0;
                buzzer.play_tone(2200, 30);
            }
        }
    }

    let proto = storage.models[active_idx].rf_protocol;
    widgets::draw_header(lcd, "PROTOCOL SETUP");

    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    if proto == 0 {
        Text::new("Proto: AFHDS 2A", Point::new(4, 22), text_style).draw(lcd).ok();

        let mut name_buf = *b"M00: ";
        name_buf[1] = b'0' + ((active_idx + 1) / 10) as u8;
        name_buf[2] = b'0' + ((active_idx + 1) % 10) as u8;
        let pre_str = core::str::from_utf8(&name_buf).unwrap_or("M??: ");
        Text::new(pre_str, Point::new(4, 32), text_style).draw(lcd).ok();
        let m_str = core::str::from_utf8(&storage.models[active_idx].name).unwrap_or("MODEL");
        Text::new(m_str, Point::new(34, 32), text_style).draw(lcd).ok();

        let mut rx_buf = [b'0'; 8];
        u32_to_hex(storage.models[active_idx].rx_id, &mut rx_buf);
        Text::new("Rx ID: ", Point::new(4, 42), text_style).draw(lcd).ok();
        let rx_str = core::str::from_utf8(&rx_buf).unwrap_or("00000000");
        Text::new(rx_str, Point::new(46, 42), text_style).draw(lcd).ok();

        widgets::draw_footer_small(lcd, "[OK] Bind  [UP/DN] Proto");
    } else {
        let sel_proto = ctrl.selected_item == 0;
        let sel_baud = ctrl.selected_item == 1;
        let sel_cfg = ctrl.selected_item == 2;

        let p_arrow = if sel_proto { ">" } else { " " };
        let b_arrow = if sel_baud { ">" } else { " " };
        let c_arrow = if sel_cfg { ">" } else { " " };

        let mut p_buf = [b' '; 20];
        p_buf[0] = p_arrow.as_bytes()[0];
        p_buf[1..18].copy_from_slice(b"Proto: CRSF/ELRS ");
        let p_str = core::str::from_utf8(&p_buf[..18]).unwrap_or(">Proto: CRSF/ELRS");
        Text::new(p_str, Point::new(2, 22), text_style).draw(lcd).ok();

        let baud_str = match storage.models[active_idx].crsf_baud {
            0 => "Baud: 420k (ELRS)",
            1 => "Baud: 416.6k (TBS)",
            2 => "Baud: 115.2k (Low)",
            3 => "Baud: 921.6k (Fast)",
            _ => "Baud: 420k (ELRS)",
        };
        let mut b_buf = [b' '; 22];
        b_buf[0] = b_arrow.as_bytes()[0];
        let b_bytes = baud_str.as_bytes();
        b_buf[1..1 + b_bytes.len()].copy_from_slice(b_bytes);
        let full_b_str = core::str::from_utf8(&b_buf[..1 + b_bytes.len()]).unwrap_or(" Baud: 420k");
        Text::new(full_b_str, Point::new(2, 32), text_style).draw(lcd).ok();

        let mut c_buf = [b' '; 22];
        c_buf[0] = c_arrow.as_bytes()[0];
        c_buf[1..19].copy_from_slice(b"[Configure Module]");
        let full_c_str = core::str::from_utf8(&c_buf[..19]).unwrap_or(" [Configure Module]");
        Text::new(full_c_str, Point::new(2, 42), text_style).draw(lcd).ok();

        if sel_cfg {
            widgets::draw_footer_small(lcd, "[OK] Config   [UP/DN] Select");
        } else {
            widgets::draw_footer_small(lcd, "[OK] Next   [UP/DN] Change");
        }
    }
}
