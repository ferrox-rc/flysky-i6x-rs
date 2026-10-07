//! Analog diagnostics and System Information screens.

use crate::adc;
use crate::buzzer::Buzzer;
use crate::chip;
use crate::display::St7567;
use crate::menu::format::{ascii_as_str, u16_to_dec_4};
use crate::menu::widgets;
use crate::menu::{MenuController, NavKeys};

#[inline(never)]
pub fn update_diag_anas(
    ctrl: &mut MenuController,
    lcd: &mut St7567,
    keys: &NavKeys,
    raw_adc: &[u16; adc::NUM_CHANNELS],
    switches: &crate::input::Switches,
    buzzer: &mut Buzzer,
) {
    if keys.cancel {
        ctrl.return_to_main_menu();
        buzzer.click();
        return;
    }

    if keys.up {
        ctrl.page_idx = if ctrl.page_idx == 0 { 3 } else { ctrl.page_idx - 1 };
        buzzer.play_tone(2200, 30);
    } else if keys.down {
        ctrl.page_idx = (ctrl.page_idx + 1) % 4;
        buzzer.play_tone(2200, 30);
    }

    let title = match ctrl.page_idx {
        0 => "ANALOG (1-6)",
        1 => "ANALOG (7-11)",
        2 => "EXT SWITCHES (SE/SF)",
        _ => "EXT ANALOG (P7)",
    };
    widgets::draw_header(lcd, title);

    if ctrl.page_idx < 2 {
        let start_idx = if ctrl.page_idx == 0 { 0 } else { 6 };
        let names: &[&str] = if ctrl.page_idx == 0 {
            &["RH:AIL", "RV:ELE", "LV:THR", "LH:RUD", "SW:SA ", "SW:SB "]
        } else {
            &["POT:V1", "POT:V2", "SW:SC ", "SW:SD ", "VBAT  "]
        };

        for (i, &name) in names.iter().enumerate() {
            let adc_idx = start_idx + i;
            let y = 12 + (i as i32 * 7);
            lcd.draw_str_4x6(2, y, name, false);

            let raw = raw_adc[adc_idx].min(4095);
            let fill_w = ((raw as u32 * 38) / 4095).min(38);
            widgets::draw_bar_gauge(lcd, 44, y + 1, 40, 5, fill_w);

            let mut val_buf = [0u8; 4];
            u16_to_dec_4(raw, &mut val_buf);
            let val_str = ascii_as_str(&val_buf);
            lcd.draw_str_4x6(90, y, val_str, false);
        }
    } else if ctrl.page_idx == 2 {
        // Page 2: SE and SF Switch Diagnostics
        let ext_active = crate::input::is_ext_switches_enabled();

        let cfg_label = if ext_active {
            "EXT SW: ENABLED (PC12/15)"
        } else {
            "EXT SW: DISABLED (OFF)"
        };
        lcd.draw_str_4x6(2, 13, cfg_label, false);

        // SE (PC12)
        lcd.draw_str_4x6(2, 23, "SW:SE", false);
        let se_state = if switches.se == crate::input::SwitchPos::Down {
            "DN (LOW)"
        } else {
            "UP (HIGH)"
        };
        let fill_se = if switches.se == crate::input::SwitchPos::Down { 30 } else { 0 };
        widgets::draw_bar_gauge(lcd, 32, 23, 30, 6, fill_se);
        lcd.draw_str_4x6(66, 23, se_state, false);

        // SF (PC15)
        lcd.draw_str_4x6(2, 33, "SW:SF", false);
        let sf_state = if switches.sf == crate::input::SwitchPos::Down {
            "DN (LOW)"
        } else {
            "UP (HIGH)"
        };
        let fill_sf = if switches.sf == crate::input::SwitchPos::Down { 30 } else { 0 };
        widgets::draw_bar_gauge(lcd, 32, 33, 30, 6, fill_sf);
        lcd.draw_str_4x6(66, 33, sf_state, false);

        lcd.draw_str_4x6(2, 44, "Active-LOW mod to GND", false);
    } else {
        // Page 3: P7 Header AD12..AD15 Analog Diagnostics
        let ext_active = crate::input::is_ext_adc_enabled();

        let cfg_label = if ext_active {
            "P7 ADC: ENABLED (15-CH)"
        } else {
            "P7 ADC: DISABLED (OFF)"
        };
        lcd.draw_str_4x6(2, 13, cfg_label, false);

        let names: [&str; 4] = ["VRC:AD12", "VRD:AD13", "VRE:AD14", "VRF:AD15"];
        for (i, &name) in names.iter().enumerate() {
            let adc_idx = 11 + i;
            let y = 25 + (i as i32 * 7);
            lcd.draw_str_4x6(2, y, name, false);

            let raw = raw_adc[adc_idx].min(4095);
            let fill_w = if ext_active {
                ((raw as u32 * 38) / 4095).min(38)
            } else {
                0
            };
            widgets::draw_bar_gauge(lcd, 44, y + 1, 40, 5, fill_w);

            let mut val_buf = [0u8; 4];
            if ext_active {
                u16_to_dec_4(raw, &mut val_buf);
            } else {
                val_buf = *b" OFF";
            }
            let val_str = ascii_as_str(&val_buf);
            lcd.draw_str_4x6(90, y, val_str, false);
        }
    }

    widgets::draw_footer(lcd, "[UP/DN] Page  [ESC] Back");
}

#[inline(never)]
pub fn update_system_info(
    ctrl: &mut MenuController,
    lcd: &mut St7567,
    keys: &NavKeys,
    buzzer: &mut Buzzer,
) {
    if keys.cancel {
        ctrl.return_to_main_menu();
        buzzer.click();
        return;
    }

    let profile = chip::get_mcu_profile();

    widgets::draw_header(lcd, "SYSTEM INFORMATION");

    lcd.draw_str_4x6(4, 13, "MCU:      ", false);
    lcd.draw_str_4x6(48, 13, profile.name, false);

    lcd.draw_str_4x6(4, 20, "Firmware: ", false);
    lcd.draw_str_4x6(
        48,
        20,
        concat!(env!("FIRMWARE_VERSION"), " (", env!("GIT_HASH"), ")"),
        false,
    );

    lcd.draw_str_4x6(4, 27, "Flash:    128KB (64 Pages)", false);
    lcd.draw_str_4x6(4, 34, "SRAM:     16KB (Parity)", false);
    lcd.draw_str_4x6(4, 41, "Profiles: 20 Models", false);

    widgets::draw_footer(lcd, "[ESC] Back");
}
