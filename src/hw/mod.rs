//! Hardware Abstraction Layer (HAL/PAC) for FlySky FS-i6X.
//!
//! Isolates all FS-i6X board-specific peripherals, pin mappings, key matrix,
//! external switches (PC12/PC15), P7 analog header (PC2..PC5), and setup rows.

pub mod adc;
pub mod keys;
pub mod uart;

use crate::buzzer::Buzzer;
use crate::display::St7567;
use crate::input::{InputState, SwitchPos};
use crate::storage::{AdcInputMode, RadioConfig, RadioStorage};
use crate::trim::TrimController;
use crate::ui::glyphs::draw_switch_arrow;
use crate::ui::menu::{MenuController, MenuState};
use crate::ui::widgets;

pub const FIRMWARE_ID: &str = "flysky-i6x-rs";
pub const DEFAULT_SB_MODE: AdcInputMode = AdcInputMode::TwoPos;
pub const HAS_JOG_WHEELS: bool = false;

pub const USB_PRODUCT_JOYSTICK: &str = "FS-i6X Joystick";
pub const USB_SERIAL_JOYSTICK: &str = "FS-I6X-SIM";
pub const USB_PRODUCT_SERIAL: &str = "FS-i6X Serial";
pub const USB_SERIAL_SERIAL: &str = "FS-I6X-VCP";
pub const USB_PRODUCT_COMPOSITE: &str = "FS-i6X Radio";
pub const USB_SERIAL_COMPOSITE: &str = "FS-I6X-COMP";

pub const CLI_PROMPT: &[u8] = b"i6x> ";
pub const CLI_PAUSE_PROMPT: &[u8] = b"\r\n[Stream paused]\r\ni6x> ";
pub const STATUS_BANNER: &[u8] = concat!(
    "FlySky FS-i6X Rust Firmware ",
    env!("FIRMWARE_VERSION"),
    " (",
    env!("GIT_HASH"),
    ")\r\n"
)
.as_bytes();
pub const CRSF_UART_STATUS: &[u8] = b"Protocol: CRSF / ExpressLRS (USART2 PD5/PA15 active)\r\n";

pub const AUX_NAMES: [&str; 10] = [
    "SA (PA4):",
    "SB (PA5):",
    "SC (PB0):",
    "SD (PB1):",
    "VRA (PA6):",
    "VRB (PA7):",
    "VRC (PC2):",
    "VRD (PC3):",
    "VRE (PC4):",
    "VRF (PC5):",
];

pub const ADAPTIVE_CH_NAMES: [&str; 10] = ["A", "B", "C", "D", "1", "2", "3", "4", "5", "6"];

pub const AUX_SOURCES: &[u8] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 31, 32, 35, 36, 37, 38,
];

pub const SETUP_ITEMS: usize = 17;
pub const INPUT_SETUP_RETURN: (usize, usize) = (13, 10);
pub const TELEM_SETUP_RETURN: (usize, usize) = (14, 10);
pub const PPM_OUT_SETUP_IDX: usize = 15;

/// Early board initialization (no-op on FS-i6X, physical power switch).
#[inline(always)]
pub fn pre_init() {}

/// Initialize touch controller (no-op on FS-i6X).
#[inline(always)]
pub fn init_touch() {}

/// Initialize board-specific switch/button pins after loading config.
#[inline(always)]
pub fn init_board_keys(_radio: &RadioConfig) {}

/// Poll navigation keys from physical 3x4 key matrix + PF2 Bind button.
#[inline(always)]
pub fn poll_keys(_storage: &RadioStorage, _menu_active: bool) -> u16 {
    keys::scan_keys()
}

/// Update dynamic flight switches (no-op on FS-i6X; SE/SF are decoded in `decode_board_switches`).
#[inline(always)]
pub fn update_flight_switches(
    _state: &mut InputState,
    _storage: &RadioStorage,
    _menu_active: bool,
) {
}

/// Calculate battery voltage in millivolts using the OpenI6X calibrated formula.
/// Accounts for the 1/2 resistor divider and 0.20V series protection diode.
#[inline]
pub fn calculate_battery_mv(raw: u16) -> u16 {
    let vbat_10mv = ((raw as u32 * 100) / 421) + 20;
    (vbat_10mv * 10) as u16
}

/// Decode board-specific switches (SE..SWL) from raw ADC and GPIO states.
#[inline]
pub fn decode_board_switches(
    raw: &[u16; adc::NUM_CHANNELS],
    ext_switches: bool,
    ext_adc: bool,
    adc_modes: &[u8; 10],
) -> (
    SwitchPos,
    SwitchPos,
    SwitchPos,
    SwitchPos,
    SwitchPos,
    SwitchPos,
    SwitchPos,
    SwitchPos,
) {
    let (se, sf) = if ext_switches {
        #[cfg(all(feature = "stm32", not(test)))]
        {
            let gpioc = unsafe { &*stm32f0xx_hal::pac::GPIOC::ptr() };
            let idr = gpioc.idr.read().bits();
            let sw_e = if (idr & (1 << 12)) == 0 {
                SwitchPos::Down
            } else {
                SwitchPos::Up
            };
            let sw_f = if (idr & (1 << 15)) == 0 {
                SwitchPos::Down
            } else {
                SwitchPos::Up
            };
            (sw_e, sw_f)
        }
        #[cfg(any(not(feature = "stm32"), test))]
        {
            (SwitchPos::Up, SwitchPos::Up)
        }
    } else {
        (SwitchPos::Up, SwitchPos::Up)
    };

    let (swi, swj, swk, swl) = if ext_adc {
        let swi = match AdcInputMode::resolve(6, adc_modes[6]) {
            AdcInputMode::ThreePos => crate::input::decode_switch_3pos(raw[11]),
            AdcInputMode::TwoPos => crate::input::decode_switch_2pos(raw[11]),
            _ => SwitchPos::Up,
        };
        let swj = match AdcInputMode::resolve(7, adc_modes[7]) {
            AdcInputMode::ThreePos => crate::input::decode_switch_3pos(raw[12]),
            AdcInputMode::TwoPos => crate::input::decode_switch_2pos(raw[12]),
            _ => SwitchPos::Up,
        };
        let swk = match AdcInputMode::resolve(8, adc_modes[8]) {
            AdcInputMode::ThreePos => crate::input::decode_switch_3pos(raw[13]),
            AdcInputMode::TwoPos => crate::input::decode_switch_2pos(raw[13]),
            _ => SwitchPos::Up,
        };
        let swl = match AdcInputMode::resolve(9, adc_modes[9]) {
            AdcInputMode::ThreePos => crate::input::decode_switch_3pos(raw[14]),
            AdcInputMode::TwoPos => crate::input::decode_switch_2pos(raw[14]),
            _ => SwitchPos::Up,
        };
        (swi, swj, swk, swl)
    } else {
        (SwitchPos::Up, SwitchPos::Up, SwitchPos::Up, SwitchPos::Up)
    };

    (
        se,
        sf,
        SwitchPos::Up,
        SwitchPos::Up,
        swi,
        swj,
        swk,
        swl,
    )
}

/// Number of visible items in Input Setup menu.
#[inline]
pub fn input_setup_items(storage: &RadioStorage) -> usize {
    if storage.radio.ext_adc != 0 {
        10
    } else {
        6
    }
}

/// Handle OK key press for board-specific Radio Setup items (indices >= 8).
pub fn handle_board_setup_ok(
    idx: usize,
    ctrl: &mut MenuController,
    storage: &mut RadioStorage,
    _trims: &mut TrimController,
    buzzer: &mut Buzzer,
) {
    match idx {
        8 => {
            buzzer.click();
            storage.radio.ext_module_pwr = if storage.radio.ext_module_pwr == 0 {
                1
            } else {
                0
            };
            crate::crsf::set_power_polarity(storage.radio.ext_module_pwr == 0);
            crate::storage::save_radio_config(storage);
        }
        9 => {
            buzzer.click();
            storage.radio.ext_switches = if storage.radio.ext_switches == 0 {
                1
            } else {
                0
            };
            crate::input::set_ext_switches_enabled(storage.radio.ext_switches != 0);
            crate::storage::save_radio_config(storage);
        }
        10 => {
            buzzer.click();
            storage.radio.ext_adc = if storage.radio.ext_adc == 0 { 1 } else { 0 };
            adc::set_ext_adc_enabled(storage.radio.ext_adc != 0);
            crate::input::set_ext_adc_enabled(storage.radio.ext_adc != 0);
            crate::storage::save_radio_config(storage);
        }
        11 => {
            buzzer.click();
            storage.radio.stick_mode = (storage.radio.stick_mode + 1) % 4;
            crate::storage::save_radio_config(storage);
        }
        12 => {
            buzzer.click();
            storage.radio.gimbal_type = if storage.radio.gimbal_type == 0 { 1 } else { 0 };
            crate::input::apply_calibration(&storage.radio);
            crate::storage::save_radio_config(storage);
        }
        13 => {
            buzzer.click();
            ctrl.state = MenuState::InputSetup;
            ctrl.selected_item = 0;
            ctrl.scroll_offset = 0;
        }
        14 => {
            buzzer.click();
            ctrl.state = MenuState::TelemSetup;
            ctrl.selected_item = 0;
            ctrl.scroll_offset = 0;
        }
        15 => {
            buzzer.click();
            let en = !storage.radio.is_ppm_out_enabled();
            storage.radio.set_ppm_out_enabled(en);
            crate::trainer::set_ppm_out(en);
            crate::storage::save_radio_config(storage);
        }
        16 => {
            buzzer.click();
            let en = !storage.radio.is_ppm_in_enabled();
            storage.radio.set_ppm_in_enabled(en);
            crate::trainer::set_ppm_in(en);
            crate::storage::save_radio_config(storage);
        }
        _ => {}
    }
}

/// Render board-specific Radio Setup rows (indices >= 8).
pub fn draw_board_setup_row(
    lcd: &mut St7567,
    slot: usize,
    idx: usize,
    is_sel: bool,
    storage: &RadioStorage,
) {
    match idx {
        8 => {
            let pwr_str = if storage.radio.ext_module_pwr == 0 {
                "HIGH (N)"
            } else {
                "LOW (P)"
            };
            widgets::draw_list_row_right(lcd, slot, is_sel, "PC13 Pwr:", Some(pwr_str));
        }
        9 => {
            let sw_str = if storage.radio.ext_switches != 0 {
                "PC12+PC15"
            } else {
                "OFF"
            };
            widgets::draw_list_row_right(lcd, slot, is_sel, "Ext Sw:", Some(sw_str));
        }
        10 => {
            let adc_str = if storage.radio.ext_adc != 0 {
                "AD12-15"
            } else {
                "OFF"
            };
            widgets::draw_list_row_right(lcd, slot, is_sel, "P7 Analog:", Some(adc_str));
        }
        11 => {
            let mode_str = match storage.radio.stick_mode {
                0 => "MODE 1",
                1 => "MODE 2",
                2 => "MODE 3",
                _ => "MODE 4",
            };
            widgets::draw_list_row_right(lcd, slot, is_sel, "Stick Mode:", Some(mode_str));
        }
        12 => {
            let gmb_str = if storage.radio.is_hall_gimbals() {
                "HALL"
            } else {
                "POT"
            };
            widgets::draw_list_row_right(lcd, slot, is_sel, "Gimbals:", Some(gmb_str));
        }
        13 => {
            widgets::draw_list_row_right(lcd, slot, is_sel, "Input Setup", Some(">"));
        }
        14 => {
            widgets::draw_list_row_right(lcd, slot, is_sel, "Telem Setup", Some(">"));
        }
        15 => {
            let ppm_str = if storage.radio.is_ppm_out_enabled() {
                "ON (PF10)"
            } else {
                "OFF"
            };
            widgets::draw_list_row_right(lcd, slot, is_sel, "PPM Out:", Some(ppm_str));
        }
        16 => {
            let trn_str = if storage.radio.is_ppm_in_enabled() {
                "ON (SD DN)"
            } else {
                "OFF"
            };
            widgets::draw_list_row_right(lcd, slot, is_sel, "Trainer In:", Some(trn_str));
        }
        _ => {}
    }
}

/// Render the default switch and dual pot layout (y = 46..53) for FS-i6X.
pub fn render_default_layout(lcd: &mut St7567, state: &InputState, storage: &RadioStorage) {
    let ext_sw = storage.radio.ext_switches != 0;

    if ext_sw {
        draw_switch_slot(lcd, 2, 7, "A", state.switches.sa);
        draw_switch_slot(lcd, 13, 18, "B", state.switches.sb);
        draw_switch_slot(lcd, 24, 29, "C", state.switches.sc);
        draw_switch_slot(lcd, 35, 40, "D", state.switches.sd);
        draw_switch_slot(lcd, 46, 51, "E", state.switches.se);
        draw_switch_slot(lcd, 57, 62, "F", state.switches.sf);
    } else {
        draw_switch_slot(lcd, 2, 9, "A", state.switches.sa);
        draw_switch_slot(lcd, 20, 27, "B", state.switches.sb);
        draw_switch_slot(lcd, 38, 45, "C", state.switches.sc);
        draw_switch_slot(lcd, 56, 63, "D", state.switches.sd);
    }

    lcd.draw_str_4x6(74, 47, "VR", false);
    widgets::draw_split_pot_bar(lcd, 84, 46, 42, state.pots.vr1, state.pots.vr2);
}

#[inline]
fn draw_switch_slot(lcd: &mut St7567, label_x: i32, val_x: i32, label: &str, pos: SwitchPos) {
    lcd.draw_str_4x6(label_x, 47, label, false);
    draw_switch_arrow(lcd, val_x, 46, pos);
}

/// Append board-specific digital switches (SE, SF on FS-i6X) to the adaptive dashboard switch list.
pub fn append_extra_switches(
    state: &InputState,
    storage: &RadioStorage,
    sw_list: &mut [(&'static str, SwitchPos, bool, u8); 16],
    sw_count: &mut usize,
) {
    if storage.radio.ext_switches != 0 {
        if *sw_count < sw_list.len() {
            sw_list[*sw_count] = ("E", state.switches.se, false, 0);
            *sw_count += 1;
        }
        if *sw_count < sw_list.len() {
            sw_list[*sw_count] = ("F", state.switches.sf, false, 0);
            *sw_count += 1;
        }
    }
}

/// Diagnostics page header title for page index 0..4.
pub fn diag_page_title(page_idx: usize) -> &'static str {
    match page_idx {
        0 => "ANALOG (1-6)",
        1 => "ANALOG (7-11)",
        2 => "EXT SWITCHES (SE/SF)",
        3 => "EXT ANALOG (P7)",
        _ => "STORAGE (EEPROM)",
    }
}

/// Render board-specific Diagnostics pages (pages 2, 3, and 4).
pub fn draw_diag_ext_page(
    lcd: &mut St7567,
    page_idx: usize,
    raw_adc: &[u16; adc::NUM_CHANNELS],
    switches: &crate::input::Switches,
) {
    use crate::ui::format::{ascii_as_str, u16_to_dec_4};

    if page_idx == 2 {
        let ext_active = crate::input::is_ext_switches_enabled();

        let cfg_label = if ext_active {
            "EXT SW: ENABLED (PC12/15)"
        } else {
            "EXT SW: DISABLED (OFF)"
        };
        lcd.draw_str_4x6(2, 13, cfg_label, false);

        // SE (PC12)
        lcd.draw_str_4x6(2, 23, "SW:SE", false);
        let se_state = if switches.se == SwitchPos::Down {
            "DN (LOW)"
        } else {
            "UP (HIGH)"
        };
        let fill_se = if switches.se == SwitchPos::Down { 30 } else { 0 };
        widgets::draw_bar_gauge(lcd, 32, 23, 30, 6, fill_se);
        lcd.draw_str_4x6(66, 23, se_state, false);

        // SF (PC15)
        lcd.draw_str_4x6(2, 33, "SW:SF", false);
        let sf_state = if switches.sf == SwitchPos::Down {
            "DN (LOW)"
        } else {
            "UP (HIGH)"
        };
        let fill_sf = if switches.sf == SwitchPos::Down { 30 } else { 0 };
        widgets::draw_bar_gauge(lcd, 32, 33, 30, 6, fill_sf);
        lcd.draw_str_4x6(66, 33, sf_state, false);

        lcd.draw_str_4x6(2, 44, "Active-LOW mod to GND", false);
    } else if page_idx == 3 {
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
    } else {
        let status = crate::storage::get_eeprom_status();
        if status.detected {
            let bus_str = match status.bus {
                2 => "BUS: I2C2 (PB10/PB11 Fm+)",
                1 => "BUS: I2C1 (PB8/PB9)",
                _ => "BUS: Unknown",
            };
            lcd.draw_str_4x6(2, 13, bus_str, false);

            let mut addr_buf = *b"ADDR: 0x50 (FT24C128A 16KB)";
            addr_buf[8] = b"0123456789ABCDEF"[(status.addr >> 4) as usize];
            addr_buf[9] = b"0123456789ABCDEF"[(status.addr & 0x0F) as usize];
            lcd.draw_str_4x6(2, 23, ascii_as_str(&addr_buf), false);

            let state_str = if status.write_err {
                "STATUS: WRITE ERR (FAIL)"
            } else if status.read_ok {
                "STATUS: READY (HEALTHY)"
            } else {
                "STATUS: READ ERR (FAIL)"
            };
            lcd.draw_str_4x6(2, 33, state_str, false);

            lcd.draw_str_4x6(2, 43, "SLOTS: 2xRadio  20xModels", false);
        } else {
            lcd.draw_str_4x6(2, 13, "BUS: I2C2 (PB10/PB11)", false);
            lcd.draw_str_4x6(2, 23, "STATUS: NOT DETECTED", false);
            lcd.draw_str_4x6(2, 33, "CHECK: Pull-ups / Wiring", false);
            lcd.draw_str_4x6(2, 43, "FALLBACK: Factory Defaults", false);
        }
    }
}

/// Board-specific background idle state for FS-i6X.
#[derive(Default)]
pub struct BoardIdleState;

impl BoardIdleState {
    pub const fn new() -> Self {
        Self
    }

    /// Update board-specific background tasks and return `(ok_held_for_menu, timer_reset_clicked, instant_trim_clicked)`.
    #[inline(always)]
    pub fn update(
        &mut self,
        _now: u32,
        _dt_ms: u16,
        keys: u16,
        menu_active: bool,
        _storage: &mut RadioStorage,
        _buzzer: &mut Buzzer,
    ) -> (bool, bool, bool) {
        let ok_held = !menu_active && (keys & (1 << 10)) != 0;
        (ok_held, false, false)
    }

    #[inline(always)]
    pub fn clear_menu_open_state(&mut self) {}

    #[inline(always)]
    pub fn take_frame_keys(&mut self, current_keys: u16, _menu_or_calib_active: bool) -> u16 {
        current_keys
    }

    #[inline(always)]
    pub fn render_overlay(&self, _lcd: &mut St7567) {}
}
