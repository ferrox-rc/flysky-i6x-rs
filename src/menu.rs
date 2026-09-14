//! Settings and Diagnostics Menu Subsystem for FlySky FS-i6X.
//!
//! Provides navigation, 20-model memory management, channel reversing,
//! 5/9-point throttle curve editing with real-time spline visualization,
//! configuration editing with Flash persistence, live channel monitoring,
//! and raw ADC diagnostics.

use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::adc;
use crate::buzzer::Buzzer;
use crate::chip;
use crate::curve;
use crate::display::St7567;
use crate::rf;
use crate::storage::{self, ModelConfig, RadioStorage, NUM_MODELS};
use crate::trim::TrimController;

const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

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

fn next_ascii(c: u8) -> u8 {
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

fn prev_ascii(c: u8) -> u8 {
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

fn i8_to_dec(val: i8, buf: &mut [u8; 6]) -> &str {
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

fn u8_to_dec(val: u8, buf: &mut [u8; 5]) -> &str {
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MenuState {
    Closed,
    MainMenu,
    ModelSelect,
    ModelSetup,
    DualRateExpo,
    ThrottleCurve,
    WingMixer,
    MixerLineEdit,
    AuxChannels,
    ChannelReverse,
    RadioSetup,
    RxSetup,
    ChannelMonitor,
    DiagAnas,
    SystemInfo,
}

pub struct MenuController {
    pub state: MenuState,
    pub selected_item: usize,
    pub scroll_offset: usize,
    pub page_idx: usize,
    pub sub_idx: usize,
    pub editing: bool,
    pub request_calibration: bool,
    pub request_bind: bool,
    prev_keys: u16,
    waiting_release: bool,
    up_hold_ms: u16,
    down_hold_ms: u16,
    repeat_timer_ms: u16,
}

impl MenuController {
    pub const fn new() -> Self {
        Self {
            state: MenuState::Closed,
            selected_item: 0,
            scroll_offset: 0,
            page_idx: 0,
            sub_idx: 0,
            editing: false,
            request_calibration: false,
            request_bind: false,
            prev_keys: 0xFFFF,
            waiting_release: false,
            up_hold_ms: 0,
            down_hold_ms: 0,
            repeat_timer_ms: 0,
        }
    }

    /// Open the main settings menu.
    pub fn open(&mut self, buzzer: &mut Buzzer) {
        self.state = MenuState::MainMenu;
        self.selected_item = 0;
        self.scroll_offset = 0;
        self.page_idx = 0;
        self.sub_idx = 0;
        self.editing = false;
        self.request_calibration = false;
        self.request_bind = false;
        self.waiting_release = true;
        self.prev_keys = 0xFFFF;
        self.up_hold_ms = 0;
        self.down_hold_ms = 0;
        self.repeat_timer_ms = 0;
        buzzer.click();
    }

    /// Returns true if any menu or diagnostic screen is active.
    pub fn is_active(&self) -> bool {
        self.state != MenuState::Closed
    }

    /// Process navigation keys, update menu state, and render display.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        lcd: &mut St7567,
        keys: u16,
        storage: &mut RadioStorage,
        trims: &mut TrimController,
        raw_adc: &[u16; adc::NUM_CHANNELS],
        rf_chs: &[u16; 14],
        buzzer: &mut Buzzer,
    ) {
        // Key release tracking (bit 10: OK, bit 11: Cancel, bit 9: Up, bit 8: Down, bit 12: Bind)
        if self.waiting_release && (keys & ((1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12))) == 0 {
            self.waiting_release = false;
        }

        let newly_pressed = if self.waiting_release {
            0
        } else {
            keys & !self.prev_keys
        };
        self.prev_keys = keys;

        let raw_down = (keys & (1 << 8)) != 0 && !self.waiting_release;
        let raw_up = (keys & (1 << 9)) != 0 && !self.waiting_release;

        let mut down_pressed = (newly_pressed & (1 << 8)) != 0;
        let mut up_pressed = (newly_pressed & (1 << 9)) != 0;

        // Auto-repeat when UP or DOWN is held (quick traversal of lists, values, and characters)
        if raw_down {
            self.down_hold_ms = self.down_hold_ms.saturating_add(20);
            if self.down_hold_ms >= 300 {
                self.repeat_timer_ms = self.repeat_timer_ms.saturating_add(20);
                if self.repeat_timer_ms >= 70 {
                    down_pressed = true;
                    self.repeat_timer_ms = 0;
                }
            }
        } else {
            self.down_hold_ms = 0;
        }

        if raw_up {
            self.up_hold_ms = self.up_hold_ms.saturating_add(20);
            if self.up_hold_ms >= 300 {
                self.repeat_timer_ms = self.repeat_timer_ms.saturating_add(20);
                if self.repeat_timer_ms >= 70 {
                    up_pressed = true;
                    self.repeat_timer_ms = 0;
                }
            }
        } else {
            self.up_hold_ms = 0;
        }

        if !raw_up && !raw_down {
            self.repeat_timer_ms = 0;
        }

        let ok_pressed = (newly_pressed & (1 << 10)) != 0;
        let cancel_pressed = (newly_pressed & (1 << 11)) != 0;
        let bind_pressed = (newly_pressed & (1 << 12)) != 0;

        let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
        let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);

        lcd.clear(BinaryColor::Off).ok();

        match self.state {
            MenuState::Closed => {}

            MenuState::MainMenu => {
                const ITEM_COUNT: usize = 13;
                let items = [
                    "1. Model Select",
                    "2. Model Setup",
                    "3. Dual Rate/Expo",
                    "4. Thr Curve",
                    "5. Wing/Mixer",
                    "6. Aux Channels",
                    "7. Ch Reverse",
                    "8. Radio Setup",
                    "9. RX Setup & Bind",
                    "10. Channel Monitor",
                    "11. Calibration",
                    "12. Analog Diag",
                    "13. System Info",
                ];

                if cancel_pressed {
                    self.state = MenuState::Closed;
                    buzzer.click();
                    return;
                }

                if down_pressed {
                    if self.selected_item + 1 < ITEM_COUNT {
                        self.selected_item += 1;
                        if self.selected_item >= self.scroll_offset + 4 {
                            self.scroll_offset = self.selected_item - 3;
                        }
                    } else {
                        self.selected_item = 0;
                        self.scroll_offset = 0;
                    }
                    buzzer.play_tone(2200, 30);
                }

                if up_pressed {
                    if self.selected_item > 0 {
                        self.selected_item -= 1;
                        if self.selected_item < self.scroll_offset {
                            self.scroll_offset = self.selected_item;
                        }
                    } else {
                        self.selected_item = ITEM_COUNT - 1;
                        self.scroll_offset = ITEM_COUNT.saturating_sub(4);
                    }
                    buzzer.play_tone(2200, 30);
                }

                if ok_pressed {
                    buzzer.click();
                    self.waiting_release = true;
                    match self.selected_item {
                        0 => {
                            self.state = MenuState::ModelSelect;
                            self.selected_item = storage.radio.active_model as usize;
                            self.scroll_offset = self.selected_item.saturating_sub(2).min(NUM_MODELS.saturating_sub(4));
                        }
                        1 => {
                            self.state = MenuState::ModelSetup;
                            self.selected_item = 0;
                            self.sub_idx = 0;
                        }
                        2 => {
                            self.state = MenuState::DualRateExpo;
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                            self.sub_idx = 0;
                        }
                        3 => {
                            self.state = MenuState::ThrottleCurve;
                            self.selected_item = 0;
                        }
                        4 => {
                            self.state = MenuState::WingMixer;
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                        }
                        5 => {
                            self.state = MenuState::AuxChannels;
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                        }
                        6 => {
                            self.state = MenuState::ChannelReverse;
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                        }
                        7 => {
                            self.state = MenuState::RadioSetup;
                            self.selected_item = 0;
                        }
                        8 => {
                            self.state = MenuState::RxSetup;
                            self.selected_item = 0;
                        }
                        9 => {
                            self.state = MenuState::ChannelMonitor;
                            self.page_idx = 0;
                        }
                        10 => {
                            self.request_calibration = true;
                            self.state = MenuState::Closed;
                            return;
                        }
                        11 => {
                            self.state = MenuState::DiagAnas;
                        }
                        12 => {
                            self.state = MenuState::SystemInfo;
                        }
                        _ => {}
                    }
                    return;
                }

                // Render Header
                Text::new("SETTINGS MENU", Point::new(26, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                // Render 4 visible items
                for slot in 0..4 {
                    let idx = self.scroll_offset + slot;
                    if idx >= ITEM_COUNT {
                        break;
                    }
                    let y = 14 + (slot as i32 * 9);
                    let is_selected = idx == self.selected_item;

                    if is_selected {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9))
                            .into_styled(fill_style)
                            .draw(lcd)
                            .ok();
                        let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                        Text::new(items[idx], Point::new(4, y + 7), inv_style).draw(lcd).ok();
                    } else {
                        Text::new(items[idx], Point::new(4, y + 7), text_style).draw(lcd).ok();
                    }
                }

                // Render Footer
                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Select   [ESC] Exit", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::ModelSelect => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 0;
                    self.scroll_offset = 0;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if down_pressed {
                    if self.selected_item + 1 < NUM_MODELS {
                        self.selected_item += 1;
                        if self.selected_item >= self.scroll_offset + 4 {
                            self.scroll_offset = self.selected_item - 3;
                        }
                    } else {
                        self.selected_item = 0;
                        self.scroll_offset = 0;
                    }
                    buzzer.play_tone(2200, 30);
                }

                if up_pressed {
                    if self.selected_item > 0 {
                        self.selected_item -= 1;
                        if self.selected_item < self.scroll_offset {
                            self.scroll_offset = self.selected_item;
                        }
                    } else {
                        self.selected_item = NUM_MODELS - 1;
                        self.scroll_offset = NUM_MODELS.saturating_sub(4);
                    }
                    buzzer.play_tone(2200, 30);
                }

                if ok_pressed {
                    // Save active trims into current model before switching
                    let old_idx = storage.radio.active_model as usize;
                    storage.models[old_idx].trims = [
                        trims.values.roll,
                        trims.values.pitch,
                        trims.values.throttle,
                        trims.values.yaw,
                    ];

                    // Switch active model
                    let new_idx = self.selected_item;
                    storage.radio.active_model = new_idx as u8;

                    // Load new model's trims
                    trims.values.roll = storage.models[new_idx].trims[0];
                    trims.values.pitch = storage.models[new_idx].trims[1];
                    trims.values.throttle = storage.models[new_idx].trims[2];
                    trims.values.yaw = storage.models[new_idx].trims[3];

                    // Update RF driver receiver ID
                    rf::set_rx_id(storage.models[new_idx].rx_id);

                    // Persist to Flash
                    storage::save_storage(storage);
                    buzzer.play_tone_pattern(2400, 60, 40, 2);
                    self.waiting_release = true;
                }

                // Render Header
                Text::new("SELECT MODEL (1-20)", Point::new(10, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                for slot in 0..4 {
                    let idx = self.scroll_offset + slot;
                    if idx >= NUM_MODELS {
                        break;
                    }
                    let y = 14 + (slot as i32 * 9);
                    let is_cursor = idx == self.selected_item;
                    let is_active = idx == (storage.radio.active_model as usize);

                    let mut line_buf = [b' '; 18];
                    line_buf[0] = b'M';
                    line_buf[1] = b'0' + ((idx + 1) / 10) as u8;
                    line_buf[2] = b'0' + ((idx + 1) % 10) as u8;
                    line_buf[3] = b':';
                    line_buf[4..14].copy_from_slice(&storage.models[idx].name);
                    if is_active {
                        line_buf[15] = b'[';
                        line_buf[16] = b'*';
                        line_buf[17] = b']';
                    }
                    let line_str = core::str::from_utf8(&line_buf).unwrap_or("M??:---");

                    if is_cursor {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                        let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                        Text::new(line_str, Point::new(4, y + 7), inv_style).draw(lcd).ok();
                    } else {
                        Text::new(line_str, Point::new(4, y + 7), text_style).draw(lcd).ok();
                    }
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Load     [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::ModelSetup => {
                let active_idx = storage.radio.active_model as usize;
                const FIELD_COUNT: usize = 4;

                if !self.editing {
                    if cancel_pressed {
                        storage::save_storage(storage);
                        self.state = MenuState::MainMenu;
                        self.selected_item = 1;
                        self.waiting_release = true;
                        buzzer.click();
                        return;
                    }

                    if down_pressed {
                        self.selected_item = (self.selected_item + 1) % FIELD_COUNT;
                        buzzer.play_tone(2200, 20);
                    }

                    if up_pressed {
                        self.selected_item = if self.selected_item == 0 { FIELD_COUNT - 1 } else { self.selected_item - 1 };
                        buzzer.play_tone(2200, 20);
                    }

                    if bind_pressed {
                        self.selected_item = (self.selected_item + 1) % FIELD_COUNT;
                        buzzer.play_tone(2200, 20);
                    }

                    if ok_pressed {
                        match self.selected_item {
                            0 => {
                                // Enter Name editing mode
                                self.editing = true;
                                self.sub_idx = 0;
                                buzzer.click();
                            }
                            1 => {
                                // Toggle Model Type
                                storage.models[active_idx].model_type = (storage.models[active_idx].model_type + 1) % 4;
                                storage::save_storage(storage);
                                buzzer.click();
                            }
                            2 => {
                                // Bind RX for this model!
                                self.request_bind = true;
                                self.state = MenuState::Closed;
                                buzzer.click();
                                return;
                            }
                            3 => {
                                // Reset to default
                                storage.models[active_idx] = ModelConfig::default_for_index(active_idx);
                                storage::save_storage(storage);
                                buzzer.play_tone_pattern(2200, 80, 50, 2);
                                self.waiting_release = true;
                            }
                            _ => {}
                        }
                    }
                } else {
                    // In Name editing mode (editing character at self.sub_idx)
                    if cancel_pressed {
                        // Exit edit mode, save name
                        self.editing = false;
                        storage::save_storage(storage);
                        buzzer.click();
                    } else if bind_pressed {
                        // BIND advances to next character
                        self.sub_idx = (self.sub_idx + 1) % 10;
                        buzzer.click();
                    } else if ok_pressed {
                        // OK advances character; on char 9, confirms name and moves to Type field!
                        if self.sub_idx + 1 < 10 {
                            self.sub_idx += 1;
                            buzzer.click();
                        } else {
                            self.editing = false;
                            self.selected_item = 1; // Advance to Type
                            storage::save_storage(storage);
                            buzzer.play_tone(2600, 40);
                        }
                    } else if up_pressed {
                        let c = storage.models[active_idx].name[self.sub_idx];
                        storage.models[active_idx].name[self.sub_idx] = next_ascii(c);
                        buzzer.play_tone(2200, 20);
                    } else if down_pressed {
                        let c = storage.models[active_idx].name[self.sub_idx];
                        storage.models[active_idx].name[self.sub_idx] = prev_ascii(c);
                        buzzer.play_tone(2200, 20);
                    }
                }

                // Render Header
                Text::new("MODEL SETUP", Point::new(30, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                // Field 0: Name (y = 13)
                let y0 = 13;
                if !self.editing && self.selected_item == 0 {
                    Rectangle::new(Point::new(2, y0), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Name:", Point::new(4, y0 + 7), inv_style).draw(lcd).ok();
                    let name_str = core::str::from_utf8(&storage.models[active_idx].name).unwrap_or("----------");
                    Text::new(name_str, Point::new(40, y0 + 7), inv_style).draw(lcd).ok();
                } else {
                    Text::new("Name:", Point::new(4, y0 + 7), text_style).draw(lcd).ok();
                    let name_str = core::str::from_utf8(&storage.models[active_idx].name).unwrap_or("----------");
                    Text::new(name_str, Point::new(40, y0 + 7), text_style).draw(lcd).ok();
                }

                if self.editing {
                    // Draw inverted box over currently edited character
                    let char_x = 40 + (self.sub_idx as i32 * 6);
                    Rectangle::new(Point::new(char_x - 1, y0), Size::new(8, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    let single_char = [storage.models[active_idx].name[self.sub_idx]];
                    let c_str = core::str::from_utf8(&single_char).unwrap_or("?");
                    Text::new(c_str, Point::new(char_x, y0 + 7), inv_style).draw(lcd).ok();
                }

                // Field 1: Type (y = 23)
                let y1 = 23;
                let type_str = match storage.models[active_idx].model_type {
                    0 => "AIRPLANE",
                    1 => "GLIDER",
                    2 => "HELICOPTER",
                    _ => "MULTI / QUAD",
                };
                if !self.editing && self.selected_item == 1 {
                    Rectangle::new(Point::new(2, y1), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Type:", Point::new(4, y1 + 7), inv_style).draw(lcd).ok();
                    Text::new(type_str, Point::new(40, y1 + 7), inv_style).draw(lcd).ok();
                } else {
                    Text::new("Type:", Point::new(4, y1 + 7), text_style).draw(lcd).ok();
                    Text::new(type_str, Point::new(40, y1 + 7), text_style).draw(lcd).ok();
                }

                // Field 2: Bind RX (y = 33)
                let y2 = 33;
                let mut rx_buf = [b'0'; 8];
                u32_to_hex(storage.models[active_idx].rx_id, &mut rx_buf);
                let rx_hex_str = core::str::from_utf8(&rx_buf).unwrap_or("00000000");
                if !self.editing && self.selected_item == 2 {
                    Rectangle::new(Point::new(2, y2), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Rx:", Point::new(4, y2 + 7), inv_style).draw(lcd).ok();
                    Text::new(rx_hex_str, Point::new(24, y2 + 7), inv_style).draw(lcd).ok();
                    Text::new("[OK Bind]", Point::new(74, y2 + 7), inv_style).draw(lcd).ok();
                } else {
                    Text::new("Rx:", Point::new(4, y2 + 7), text_style).draw(lcd).ok();
                    Text::new(rx_hex_str, Point::new(24, y2 + 7), text_style).draw(lcd).ok();
                    Text::new("[OK Bind]", Point::new(74, y2 + 7), text_style).draw(lcd).ok();
                }

                // Field 3: Reset Defaults (y = 43)
                let y3 = 43;
                if !self.editing && self.selected_item == 3 {
                    Rectangle::new(Point::new(2, y3), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Reset: [OK Defaults]", Point::new(4, y3 + 7), inv_style).draw(lcd).ok();
                } else {
                    Text::new("Reset: [OK Defaults]", Point::new(4, y3 + 7), text_style).draw(lcd).ok();
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                if self.editing {
                    Text::new("[OK] Next Char [ESC] Done", Point::new(2, 62), text_style).draw(lcd).ok();
                } else {
                    Text::new("[OK] Select    [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
                }
            }

            MenuState::DualRateExpo => {
                let active_idx = storage.radio.active_model as usize;
                const FIELD_COUNT: usize = 6;

                if cancel_pressed {
                    if self.editing {
                        self.editing = false;
                    } else {
                        storage::save_storage(storage);
                        self.state = MenuState::MainMenu;
                        self.selected_item = 2;
                        self.scroll_offset = 0;
                        self.waiting_release = true;
                        buzzer.click();
                        return;
                    }
                }

                let axis = self.sub_idx.min(2); // 0: Roll, 1: Pitch, 2: Yaw

                if !self.editing {
                    if down_pressed {
                        if self.selected_item + 1 < FIELD_COUNT {
                            self.selected_item += 1;
                            if self.selected_item >= self.scroll_offset + 4 {
                                self.scroll_offset = self.selected_item - 3;
                            }
                        } else {
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                        }
                        buzzer.play_tone(2200, 30);
                    }

                    if up_pressed {
                        if self.selected_item > 0 {
                            self.selected_item -= 1;
                            if self.selected_item < self.scroll_offset {
                                self.scroll_offset = self.selected_item;
                            }
                        } else {
                            self.selected_item = FIELD_COUNT - 1;
                            self.scroll_offset = FIELD_COUNT.saturating_sub(4);
                        }
                        buzzer.play_tone(2200, 30);
                    }

                    if ok_pressed {
                        self.editing = true;
                        buzzer.click();
                    }
                } else {
                    // Editing value
                    match self.selected_item {
                        0 => {
                            // Switch (0..4)
                            if up_pressed {
                                storage.models[active_idx].dr_switch = (storage.models[active_idx].dr_switch + 1) % 5;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed {
                                storage.models[active_idx].dr_switch = if storage.models[active_idx].dr_switch == 0 { 4 } else { storage.models[active_idx].dr_switch - 1 };
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        1 => {
                            // Axis (0..2)
                            if up_pressed || down_pressed {
                                self.sub_idx = (self.sub_idx + 1) % 3;
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        2 => {
                            // Hi Rate (50..100%, step 5%)
                            let cur = storage.models[active_idx].dr_high[axis];
                            if up_pressed && cur <= 95 {
                                storage.models[active_idx].dr_high[axis] = cur + 5;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed && cur >= 55 {
                                storage.models[active_idx].dr_high[axis] = cur - 5;
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        3 => {
                            // Lo Rate (30..100%, step 5%)
                            let cur = storage.models[active_idx].dr_low[axis];
                            if up_pressed && cur <= 95 {
                                storage.models[active_idx].dr_low[axis] = cur + 5;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed && cur >= 35 {
                                storage.models[active_idx].dr_low[axis] = cur - 5;
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        4 => {
                            // Hi Expo (-100..+100%, step 5%)
                            let cur = storage.models[active_idx].expo_high[axis];
                            if up_pressed && cur <= 95 {
                                storage.models[active_idx].expo_high[axis] = cur + 5;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed && cur >= -95 {
                                storage.models[active_idx].expo_high[axis] = cur - 5;
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        5 => {
                            // Lo Expo (-100..+100%, step 5%)
                            let cur = storage.models[active_idx].expo_low[axis];
                            if up_pressed && cur <= 95 {
                                storage.models[active_idx].expo_low[axis] = cur + 5;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed && cur >= -95 {
                                storage.models[active_idx].expo_low[axis] = cur - 5;
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        _ => {}
                    }

                    if ok_pressed {
                        self.editing = false;
                        buzzer.click();
                    }
                }

                // Render Header
                Text::new("DUAL RATE / EXPO", Point::new(18, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                let mut b5 = [0u8; 5];
                let mut b6 = [0u8; 6];
                for slot in 0..4 {
                    let idx = self.scroll_offset + slot;
                    if idx >= FIELD_COUNT { break; }
                    let y = 14 + (slot as i32 * 9);
                    let is_sel = idx == self.selected_item;
                    let style = if is_sel {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off)
                    } else {
                        text_style
                    };

                    match idx {
                        0 => {
                            let sw_name = DR_SWITCH_NAMES[(storage.models[active_idx].dr_switch as usize).min(4)];
                            Text::new("Switch:", Point::new(4, y + 7), style).draw(lcd).ok();
                            Text::new(sw_name, Point::new(60, y + 7), style).draw(lcd).ok();
                        }
                        1 => {
                            Text::new("Channel:", Point::new(4, y + 7), style).draw(lcd).ok();
                            Text::new(AXIS_NAMES[axis], Point::new(60, y + 7), style).draw(lcd).ok();
                        }
                        2 => {
                            let str_val = u8_to_dec(storage.models[active_idx].dr_high[axis], &mut b5);
                            Text::new("Hi Rate:", Point::new(4, y + 7), style).draw(lcd).ok();
                            Text::new(str_val, Point::new(60, y + 7), style).draw(lcd).ok();
                        }
                        3 => {
                            let str_val = u8_to_dec(storage.models[active_idx].dr_low[axis], &mut b5);
                            Text::new("Lo Rate:", Point::new(4, y + 7), style).draw(lcd).ok();
                            Text::new(str_val, Point::new(60, y + 7), style).draw(lcd).ok();
                        }
                        4 => {
                            let str_val = i8_to_dec(storage.models[active_idx].expo_high[axis], &mut b6);
                            Text::new("Hi Expo:", Point::new(4, y + 7), style).draw(lcd).ok();
                            Text::new(str_val, Point::new(60, y + 7), style).draw(lcd).ok();
                        }
                        5 => {
                            let str_val = i8_to_dec(storage.models[active_idx].expo_low[axis], &mut b6);
                            Text::new("Lo Expo:", Point::new(4, y + 7), style).draw(lcd).ok();
                            Text::new(str_val, Point::new(60, y + 7), style).draw(lcd).ok();
                        }
                        _ => {}
                    }
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                let footer = if self.editing { "[OK] Done   [UP/DN] Value" } else { "[OK] Edit   [ESC] Exit" };
                Text::new(footer, Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::ChannelReverse => {
                const CH_COUNT: usize = 14;
                let active_idx = storage.radio.active_model as usize;

                if cancel_pressed {
                    storage::save_storage(storage);
                    self.state = MenuState::MainMenu;
                    self.selected_item = 6;
                    self.scroll_offset = 0;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if down_pressed {
                    if self.selected_item + 1 < CH_COUNT {
                        self.selected_item += 1;
                        if self.selected_item >= self.scroll_offset + 4 {
                            self.scroll_offset = self.selected_item - 3;
                        }
                    } else {
                        self.selected_item = 0;
                        self.scroll_offset = 0;
                    }
                    buzzer.play_tone(2200, 30);
                }

                if up_pressed {
                    if self.selected_item > 0 {
                        self.selected_item -= 1;
                        if self.selected_item < self.scroll_offset {
                            self.scroll_offset = self.selected_item;
                        }
                    } else {
                        self.selected_item = CH_COUNT - 1;
                        self.scroll_offset = CH_COUNT.saturating_sub(4);
                    }
                    buzzer.play_tone(2200, 30);
                }

                if ok_pressed {
                    // Toggle normal / reversed bit
                    storage.models[active_idx].channel_reverse ^= 1 << self.selected_item;
                    buzzer.click();
                }

                // Render Header
                Text::new("CHANNEL REVERSE", Point::new(18, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                let ch_names = [
                    "CH1 (AIL)", "CH2 (ELE)", "CH3 (THR)", "CH4 (RUD)",
                    "CH5 (SA) ", "CH6 (SB) ", "CH7 (VR1)", "CH8 (VR2)",
                    "CH9 (SC) ", "CH10(SD) ", "CH11     ", "CH12     ",
                    "CH13     ", "CH14     ",
                ];

                for slot in 0..4 {
                    let ch = self.scroll_offset + slot;
                    if ch >= CH_COUNT {
                        break;
                    }
                    let y = 14 + (slot as i32 * 9);
                    let is_sel = ch == self.selected_item;
                    let is_rev = (storage.models[active_idx].channel_reverse & (1 << ch)) != 0;
                    let status_str = if is_rev { "REVERSE" } else { "NORMAL " };

                    if is_sel {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                        let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                        Text::new(ch_names[ch], Point::new(4, y + 7), inv_style).draw(lcd).ok();
                        Text::new(status_str, Point::new(76, y + 7), inv_style).draw(lcd).ok();
                    } else {
                        Text::new(ch_names[ch], Point::new(4, y + 7), text_style).draw(lcd).ok();
                        Text::new(status_str, Point::new(76, y + 7), text_style).draw(lcd).ok();
                    }
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Toggle   [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::ThrottleCurve => {
                let active_idx = storage.radio.active_model as usize;
                let pts_count = if storage.models[active_idx].thr_curve_pts == 9 { 9 } else { 5 };
                let max_items = 2 + pts_count; // Item 0: Pts mode, Item 1: Smooth, Item 2..(2+pts_count-1): Points

                if self.editing {
                    // Editing active curve point (selected_item >= 2)
                    let pt_idx = self.selected_item.saturating_sub(2).min(pts_count - 1);

                    if cancel_pressed {
                        self.editing = false;
                        storage::save_storage(storage);
                        buzzer.click();
                        return;
                    }

                    if up_pressed {
                        if storage.models[active_idx].thr_curve[pt_idx] < 100 {
                            storage.models[active_idx].thr_curve[pt_idx] += 1;
                        }
                        buzzer.play_tone(2200, 20);
                    }

                    if down_pressed {
                        if storage.models[active_idx].thr_curve[pt_idx] > 0 {
                            storage.models[active_idx].thr_curve[pt_idx] -= 1;
                        }
                        buzzer.play_tone(2200, 20);
                    }

                    if ok_pressed {
                        // Confirm point and move to next point
                        if self.selected_item + 1 < max_items {
                            self.selected_item += 1;
                        } else {
                            self.editing = false;
                        }
                        storage::save_storage(storage);
                        buzzer.click();
                    } else if bind_pressed {
                        // BIND key tabs to next point
                        if self.selected_item + 1 < max_items {
                            self.selected_item += 1;
                        } else {
                            self.selected_item = 2; // Wrap back to P1
                        }
                        storage::save_storage(storage);
                        buzzer.click();
                    }
                } else {
                    // Navigating fields
                    if cancel_pressed {
                        storage::save_storage(storage);
                        self.state = MenuState::MainMenu;
                        self.selected_item = 3;
                        self.waiting_release = true;
                        buzzer.click();
                        return;
                    }

                    if down_pressed {
                        self.selected_item = (self.selected_item + 1) % max_items;
                        buzzer.play_tone(2200, 20);
                    }

                    if up_pressed {
                        self.selected_item = if self.selected_item == 0 {
                            max_items - 1
                        } else {
                            self.selected_item - 1
                        };
                        buzzer.play_tone(2200, 20);
                    }

                    if bind_pressed {
                        // Jump into points or cycle through points
                        if self.selected_item < 2 {
                            self.selected_item = 2;
                        } else {
                            self.selected_item = (self.selected_item + 1) % max_items;
                            if self.selected_item < 2 {
                                self.selected_item = 2;
                            }
                        }
                        buzzer.click();
                    }

                    if ok_pressed {
                        if self.selected_item == 0 {
                            // Toggle 5-PT <-> 9-PT with resampling so 9-PT is never flat zero
                            if storage.models[active_idx].thr_curve_pts == 5 {
                                let c = storage.models[active_idx].thr_curve;
                                storage.models[active_idx].thr_curve = [
                                    c[0],
                                    ((c[0] as u16 + c[1] as u16) / 2) as u8,
                                    c[1],
                                    ((c[1] as u16 + c[2] as u16) / 2) as u8,
                                    c[2],
                                    ((c[2] as u16 + c[3] as u16) / 2) as u8,
                                    c[3],
                                    ((c[3] as u16 + c[4] as u16) / 2) as u8,
                                    c[4],
                                ];
                                storage.models[active_idx].thr_curve_pts = 9;
                            } else {
                                let c = storage.models[active_idx].thr_curve;
                                storage.models[active_idx].thr_curve = [
                                    c[0], c[2], c[4], c[6], c[8], 0, 0, 0, 0,
                                ];
                                storage.models[active_idx].thr_curve_pts = 5;
                            }
                            storage::save_storage(storage);
                            buzzer.click();
                        } else if self.selected_item == 1 {
                            // Toggle Linear <-> Smooth (Catmull-Rom spline)
                            storage.models[active_idx].thr_curve_smooth = if storage.models[active_idx].thr_curve_smooth == 0 {
                                1
                            } else {
                                0
                            };
                            storage::save_storage(storage);
                            buzzer.click();
                        } else {
                            // Enter edit mode for selected point
                            self.editing = true;
                            buzzer.click();
                        }
                    }
                }

                // Render Header
                Text::new("THROTTLE CURVE", Point::new(20, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                // Left side: Mode & active point info
                let mode_str = if storage.models[active_idx].thr_curve_pts == 9 { "9-PT" } else { "5-PT" };
                let is_sel_pts = !self.editing && self.selected_item == 0;
                if is_sel_pts {
                    Rectangle::new(Point::new(2, 13), Size::new(70, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Pts:", Point::new(4, 20), inv_style).draw(lcd).ok();
                    Text::new(mode_str, Point::new(32, 20), inv_style).draw(lcd).ok();
                } else {
                    Text::new("Pts:", Point::new(4, 20), text_style).draw(lcd).ok();
                    Text::new(mode_str, Point::new(32, 20), text_style).draw(lcd).ok();
                }

                let smooth_str = if storage.models[active_idx].thr_curve_smooth != 0 { "SMOOTH" } else { "LINEAR" };
                let is_sel_crv = !self.editing && self.selected_item == 1;
                if is_sel_crv {
                    Rectangle::new(Point::new(2, 23), Size::new(70, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Crv:", Point::new(4, 30), inv_style).draw(lcd).ok();
                    Text::new(smooth_str, Point::new(32, 30), inv_style).draw(lcd).ok();
                } else {
                    Text::new("Crv:", Point::new(4, 30), text_style).draw(lcd).ok();
                    Text::new(smooth_str, Point::new(32, 30), text_style).draw(lcd).ok();
                }

                if self.selected_item >= 2 {
                    let pt_idx = self.selected_item - 2;
                    let val = storage.models[active_idx].thr_curve[pt_idx];
                    let mut p_buf = *b"P1:  0%";
                    p_buf[1] = b'1' + pt_idx as u8;
                    if val >= 100 {
                        p_buf[3] = b'1';
                        p_buf[4] = b'0';
                        p_buf[5] = b'0';
                    } else {
                        p_buf[3] = b' ';
                        p_buf[4] = b'0' + (val / 10);
                        p_buf[5] = b'0' + (val % 10);
                    }
                    let p_str = core::str::from_utf8(&p_buf).unwrap_or("P?:---%");

                    if self.editing {
                        Rectangle::new(Point::new(2, 35), Size::new(70, 11)).into_styled(fill_style).draw(lcd).ok();
                        let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                        Text::new(p_str, Point::new(4, 44), inv_style).draw(lcd).ok();
                    } else {
                        Rectangle::new(Point::new(2, 35), Size::new(70, 11)).into_styled(border_style).draw(lcd).ok();
                        Text::new(p_str, Point::new(4, 44), text_style).draw(lcd).ok();
                    }
                } else {
                    let pts_range_str = if pts_count == 9 { "Pts: 1..9" } else { "Pts: 1..5" };
                    Text::new(pts_range_str, Point::new(4, 44), text_style).draw(lcd).ok();
                }

                // Right side: Graph box (x = 76..124, y = 13..49)
                Rectangle::new(Point::new(76, 13), Size::new(49, 37)).into_styled(border_style).draw(lcd).ok();

                // Draw curve graph inside box (width 47, height 35)
                let pts_mode = storage.models[active_idx].thr_curve_pts;
                let is_smooth = storage.models[active_idx].thr_curve_smooth != 0;
                let curve_data = storage.models[active_idx].thr_curve;

                for px in 0..47 {
                    let input_pct = ((px as u32 * 1000) / 46) as u16;
                    let out_pct = curve::evaluate_curve(input_pct, pts_mode, is_smooth, &curve_data);
                    let py = 48 - ((out_pct as i32 * 34) / 1000);
                    Pixel(Point::new(77 + px, py), BinaryColor::On).draw(lcd).ok();
                }

                // Draw point indicator dot on graph for selected point
                if self.selected_item >= 2 {
                    let pt_idx = self.selected_item - 2;
                    let input_pct = (pt_idx as u32 * 1000) / (pts_count as u32 - 1);
                    let val = storage.models[active_idx].thr_curve[pt_idx];
                    let dot_x = 77 + ((input_pct * 46) / 1000) as i32;
                    let dot_y = 48 - (((val as i32 * 10) * 34) / 1000);

                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            Pixel(Point::new(dot_x + dx, dot_y + dy), BinaryColor::On).draw(lcd).ok();
                        }
                    }
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                if self.editing {
                    Text::new("[OK] Next Pt   [ESC] Done", Point::new(2, 62), text_style).draw(lcd).ok();
                } else if self.selected_item >= 2 {
                    Text::new("[OK] Edit Pt   [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
                } else {
                    Text::new("[OK] Toggle    [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
                }
            }

            MenuState::WingMixer => {
                let active_idx = storage.radio.active_model as usize;
                const MIX_ITEMS: usize = 9; // 0: Template, 1..8: Mix 1..8

                if cancel_pressed {
                    storage::save_storage(storage);
                    self.state = MenuState::MainMenu;
                    self.selected_item = 4;
                    self.scroll_offset = 0;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if down_pressed {
                    if self.selected_item + 1 < MIX_ITEMS {
                        self.selected_item += 1;
                        if self.selected_item >= self.scroll_offset + 4 {
                            self.scroll_offset = self.selected_item - 3;
                        }
                    } else {
                        self.selected_item = 0;
                        self.scroll_offset = 0;
                    }
                    buzzer.play_tone(2200, 30);
                }

                if up_pressed {
                    if self.selected_item > 0 {
                        self.selected_item -= 1;
                        if self.selected_item < self.scroll_offset {
                            self.scroll_offset = self.selected_item;
                        }
                    } else {
                        self.selected_item = MIX_ITEMS - 1;
                        self.scroll_offset = MIX_ITEMS.saturating_sub(4);
                    }
                    buzzer.play_tone(2200, 30);
                }

                if ok_pressed {
                    buzzer.click();
                    if self.selected_item == 0 {
                        // Cycle Wing Template (0..3)
                        storage.models[active_idx].wing_tail_mix = (storage.models[active_idx].wing_tail_mix + 1) % 4;
                    } else {
                        // Open Mix Line Editor
                        self.page_idx = self.selected_item - 1; // 0..7
                        self.selected_item = 0;
                        self.scroll_offset = 0;
                        self.editing = false;
                        self.state = MenuState::MixerLineEdit;
                        self.waiting_release = true;
                        return;
                    }
                }

                // Render Header
                Text::new("WING & MIXER", Point::new(26, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                for slot in 0..4 {
                    let idx = self.scroll_offset + slot;
                    if idx >= MIX_ITEMS { break; }
                    let y = 14 + (slot as i32 * 9);
                    let is_sel = idx == self.selected_item;
                    let style = if is_sel {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off)
                    } else {
                        text_style
                    };

                    if idx == 0 {
                        let t_idx = (storage.models[active_idx].wing_tail_mix as usize).min(3);
                        Text::new("Wing:", Point::new(4, y + 7), style).draw(lcd).ok();
                        Text::new(TEMPLATE_NAMES[t_idx], Point::new(36, y + 7), style).draw(lcd).ok();
                    } else {
                        let m_idx = idx - 1;
                        let mix = storage.models[active_idx].mixes[m_idx];
                        let mut m_buf = *b"M0: ";
                        m_buf[1] = b'1' + m_idx as u8;
                        let m_label = core::str::from_utf8(&m_buf).unwrap_or("M?: ");
                        Text::new(m_label, Point::new(4, y + 7), style).draw(lcd).ok();

                        if mix.target_ch == 0 {
                            Text::new("[DISABLED]", Point::new(28, y + 7), style).draw(lcd).ok();
                        } else {
                            let mut ch_buf = *b"CH00";
                            if mix.target_ch >= 10 {
                                ch_buf[2] = b'1';
                                ch_buf[3] = b'0' + (mix.target_ch - 10);
                            } else {
                                ch_buf[2] = b'0' + mix.target_ch;
                                ch_buf[3] = b' ';
                            }
                            let ch_str = core::str::from_utf8(&ch_buf).unwrap_or("CH??");
                            Text::new(ch_str, Point::new(24, y + 7), style).draw(lcd).ok();

                            Text::new("<-", Point::new(54, y + 7), style).draw(lcd).ok();
                            let s_idx = (mix.source as usize).min(25);
                            Text::new(SOURCE_NAMES[s_idx], Point::new(70, y + 7), style).draw(lcd).ok();
                        }
                    }
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Select/Edit   [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::MixerLineEdit => {
                let active_idx = storage.radio.active_model as usize;
                let mix_idx = self.page_idx.min(7);
                const FIELD_COUNT: usize = 6;

                if cancel_pressed {
                    if self.editing {
                        self.editing = false;
                    } else {
                        self.state = MenuState::WingMixer;
                        self.selected_item = mix_idx + 1;
                        self.scroll_offset = (mix_idx + 1).saturating_sub(3);
                        self.waiting_release = true;
                        buzzer.click();
                        return;
                    }
                }

                if !self.editing {
                    if down_pressed {
                        if self.selected_item + 1 < FIELD_COUNT {
                            self.selected_item += 1;
                            if self.selected_item >= self.scroll_offset + 4 {
                                self.scroll_offset = self.selected_item - 3;
                            }
                        } else {
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                        }
                        buzzer.play_tone(2200, 30);
                    }

                    if up_pressed {
                        if self.selected_item > 0 {
                            self.selected_item -= 1;
                            if self.selected_item < self.scroll_offset {
                                self.scroll_offset = self.selected_item;
                            }
                        } else {
                            self.selected_item = FIELD_COUNT - 1;
                            self.scroll_offset = FIELD_COUNT.saturating_sub(4);
                        }
                        buzzer.play_tone(2200, 30);
                    }

                    if ok_pressed {
                        self.editing = true;
                        buzzer.click();
                    }
                } else {
                    let mix = &mut storage.models[active_idx].mixes[mix_idx];
                    match self.selected_item {
                        0 => {
                            // Target CH: 0 (Off) .. 14
                            if up_pressed {
                                mix.target_ch = (mix.target_ch + 1) % 15;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed {
                                mix.target_ch = if mix.target_ch == 0 { 14 } else { mix.target_ch - 1 };
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        1 => {
                            // Source: 0..25
                            if up_pressed {
                                mix.source = (mix.source + 1) % 26;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed {
                                mix.source = if mix.source == 0 { 25 } else { mix.source - 1 };
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        2 => {
                            // Weight: -100..+100%, step 5%
                            if up_pressed && mix.weight <= 95 {
                                mix.weight += 5;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed && mix.weight >= -95 {
                                mix.weight -= 5;
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        3 => {
                            // Offset: -100..+100%, step 5%
                            if up_pressed && mix.offset <= 95 {
                                mix.offset += 5;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed && mix.offset >= -95 {
                                mix.offset -= 5;
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        4 => {
                            // Switch: 0..10
                            if up_pressed {
                                mix.switch = (mix.switch + 1) % 11;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed {
                                mix.switch = if mix.switch == 0 { 10 } else { mix.switch - 1 };
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        5 => {
                            // Mode: 0..2
                            if up_pressed {
                                mix.mode = (mix.mode + 1) % 3;
                                buzzer.play_tone(2200, 20);
                            } else if down_pressed {
                                mix.mode = if mix.mode == 0 { 2 } else { mix.mode - 1 };
                                buzzer.play_tone(2200, 20);
                            }
                        }
                        _ => {}
                    }

                    if ok_pressed {
                        self.editing = false;
                        buzzer.click();
                    }
                }

                // Render Header
                let mut title_buf = *b"EDIT MIX 0";
                title_buf[9] = b'1' + mix_idx as u8;
                let title_str = core::str::from_utf8(&title_buf).unwrap_or("EDIT MIX");
                Text::new(title_str, Point::new(34, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                let mix = storage.models[active_idx].mixes[mix_idx];
                let mut b6 = [0u8; 6];
                for slot in 0..4 {
                    let idx = self.scroll_offset + slot;
                    if idx >= FIELD_COUNT { break; }
                    let y = 14 + (slot as i32 * 9);
                    let is_sel = idx == self.selected_item;
                    let style = if is_sel {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off)
                    } else {
                        text_style
                    };

                    match idx {
                        0 => {
                            Text::new("Target:", Point::new(4, y + 7), style).draw(lcd).ok();
                            if mix.target_ch == 0 {
                                Text::new("Disabled", Point::new(56, y + 7), style).draw(lcd).ok();
                            } else {
                                let mut ch_buf = *b"CH00";
                                if mix.target_ch >= 10 {
                                    ch_buf[2] = b'1';
                                    ch_buf[3] = b'0' + (mix.target_ch - 10);
                                } else {
                                    ch_buf[2] = b'0' + mix.target_ch;
                                    ch_buf[3] = b' ';
                                }
                                let ch_str = core::str::from_utf8(&ch_buf).unwrap_or("CH??");
                                Text::new(ch_str, Point::new(56, y + 7), style).draw(lcd).ok();
                            }
                        }
                        1 => {
                            Text::new("Source:", Point::new(4, y + 7), style).draw(lcd).ok();
                            let s_idx = (mix.source as usize).min(25);
                            Text::new(SOURCE_NAMES[s_idx], Point::new(56, y + 7), style).draw(lcd).ok();
                        }
                        2 => {
                            Text::new("Weight:", Point::new(4, y + 7), style).draw(lcd).ok();
                            let w_str = i8_to_dec(mix.weight, &mut b6);
                            Text::new(w_str, Point::new(56, y + 7), style).draw(lcd).ok();
                        }
                        3 => {
                            Text::new("Offset:", Point::new(4, y + 7), style).draw(lcd).ok();
                            let o_str = i8_to_dec(mix.offset, &mut b6);
                            Text::new(o_str, Point::new(56, y + 7), style).draw(lcd).ok();
                        }
                        4 => {
                            Text::new("Switch:", Point::new(4, y + 7), style).draw(lcd).ok();
                            let sw_idx = (mix.switch as usize).min(10);
                            Text::new(SWITCH_COND_NAMES[sw_idx], Point::new(56, y + 7), style).draw(lcd).ok();
                        }
                        5 => {
                            Text::new("Mode:", Point::new(4, y + 7), style).draw(lcd).ok();
                            let m_idx = (mix.mode as usize).min(2);
                            Text::new(MODE_NAMES[m_idx], Point::new(56, y + 7), style).draw(lcd).ok();
                        }
                        _ => {}
                    }
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                let footer = if self.editing { "[OK] Done   [UP/DN] Value" } else { "[OK] Edit   [ESC] Back" };
                Text::new(footer, Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::AuxChannels => {
                let active_idx = storage.radio.active_model as usize;
                const AUX_COUNT: usize = 10;

                if cancel_pressed {
                    if self.editing {
                        self.editing = false;
                    } else {
                        storage::save_storage(storage);
                        self.state = MenuState::MainMenu;
                        self.selected_item = 5;
                        self.scroll_offset = 0;
                        self.waiting_release = true;
                        buzzer.click();
                        return;
                    }
                }

                if !self.editing {
                    if down_pressed {
                        if self.selected_item + 1 < AUX_COUNT {
                            self.selected_item += 1;
                            if self.selected_item >= self.scroll_offset + 4 {
                                self.scroll_offset = self.selected_item - 3;
                            }
                        } else {
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                        }
                        buzzer.play_tone(2200, 30);
                    }

                    if up_pressed {
                        if self.selected_item > 0 {
                            self.selected_item -= 1;
                            if self.selected_item < self.scroll_offset {
                                self.scroll_offset = self.selected_item;
                            }
                        } else {
                            self.selected_item = AUX_COUNT - 1;
                            self.scroll_offset = AUX_COUNT.saturating_sub(4);
                        }
                        buzzer.play_tone(2200, 30);
                    }

                    if ok_pressed {
                        self.editing = true;
                        buzzer.click();
                    }
                } else {
                    let cur = storage.models[active_idx].aux_channels[self.selected_item];
                    if up_pressed {
                        storage.models[active_idx].aux_channels[self.selected_item] = (cur + 1) % 11;
                        buzzer.play_tone(2200, 20);
                    } else if down_pressed {
                        storage.models[active_idx].aux_channels[self.selected_item] = if cur == 0 { 10 } else { cur - 1 };
                        buzzer.play_tone(2200, 20);
                    }
                    if ok_pressed {
                        self.editing = false;
                        buzzer.click();
                    }
                }

                // Render Header
                Text::new("AUX CHANNELS", Point::new(28, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                for slot in 0..4 {
                    let idx = self.scroll_offset + slot;
                    if idx >= AUX_COUNT { break; }
                    let y = 14 + (slot as i32 * 9);
                    let is_sel = idx == self.selected_item;
                    let style = if is_sel {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off)
                    } else {
                        text_style
                    };

                    let ch_num = 5 + idx;
                    let mut ch_buf = *b"CH00: ";
                    if ch_num >= 10 {
                        ch_buf[2] = b'1';
                        ch_buf[3] = b'0' + (ch_num - 10) as u8;
                    } else {
                        ch_buf[2] = b'0' + ch_num as u8;
                        ch_buf[3] = b' ';
                    }
                    let ch_label = core::str::from_utf8(&ch_buf).unwrap_or("CH??: ");
                    Text::new(ch_label, Point::new(4, y + 7), style).draw(lcd).ok();

                    let src_idx = (storage.models[active_idx].aux_channels[idx] as usize).min(10);
                    Text::new(SOURCE_NAMES[src_idx], Point::new(48, y + 7), style).draw(lcd).ok();
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                let footer = if self.editing { "[OK] Done   [UP/DN] Source" } else { "[OK] Edit   [ESC] Exit" };
                Text::new(footer, Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::RadioSetup => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 7;
                    self.scroll_offset = 0;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }
                const SETUP_ITEMS: usize = 6;

                if down_pressed {
                    self.selected_item = (self.selected_item + 1) % SETUP_ITEMS;
                    buzzer.play_tone(2200, 30);
                }
                if up_pressed {
                    self.selected_item = if self.selected_item == 0 {
                        SETUP_ITEMS - 1
                    } else {
                        self.selected_item - 1
                    };
                    buzzer.play_tone(2200, 30);
                }

                if ok_pressed {
                    buzzer.click();
                    match self.selected_item {
                        0 => {
                            // Cycle Throttle Trim Method: 0=OFF (Lock), 1=IDLE (T-Trim), 2=LINEAR
                            storage.radio.throttle_trim = (storage.radio.throttle_trim + 1) % 3;
                            trims.throttle_enabled = storage.radio.throttle_trim != 0;
                            storage::save_storage(storage);
                        }
                        1 => {
                            // Toggle Audio
                            storage.radio.audio_enabled = if storage.radio.audio_enabled == 0 { 1 } else { 0 };
                            buzzer.enabled = storage.radio.audio_enabled != 0;
                            storage::save_storage(storage);
                        }
                        2 => {
                            // Cycle Backlight Timeout: 0=Always On, 1=15s, 2=30s, 3=60s
                            storage.radio.backlight_timeout = (storage.radio.backlight_timeout + 1) % 4;
                            storage::save_storage(storage);
                        }
                        3 => {
                            // Cycle Backlight Brightness: 1..10 (10%..100%)
                            storage.radio.backlight_brightness = if storage.radio.backlight_brightness >= 10 {
                                1
                            } else {
                                storage.radio.backlight_brightness + 1
                            };
                            lcd.set_backlight_level(storage.radio.backlight_brightness * 10);
                            storage::save_storage(storage);
                        }
                        4 => {
                            // Cycle Contrast: 20 .. 50 in steps of 3 (wraps back to 20)
                            storage.radio.lcd_contrast = if storage.radio.lcd_contrast >= 50 {
                                20
                            } else {
                                (storage.radio.lcd_contrast + 3).min(50)
                            };
                            lcd.set_contrast(storage.radio.lcd_contrast);
                            storage::save_storage(storage);
                        }
                        5 => {
                            // Cycle Battery Alarm: 4.0V .. 5.0V (40..50 deci-volts)
                            storage.radio.vbat_warn_deci = if storage.radio.vbat_warn_deci >= 50 {
                                40
                            } else {
                                storage.radio.vbat_warn_deci + 1
                            };
                            storage::save_storage(storage);
                        }
                        _ => {}
                    }
                }

                // Render Header
                Text::new("RADIO SETUP", Point::new(32, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                // Item 0: Throttle Trim (y = 13)
                let y0 = 13;
                let is_sel0 = self.selected_item == 0;
                let val_str = match storage.radio.throttle_trim {
                    1 => "IDLE",
                    2 => "LINEAR",
                    _ => "OFF (Lock)",
                };
                if is_sel0 {
                    Rectangle::new(Point::new(2, y0), Size::new(124, 7)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Thr Trim:", Point::new(4, y0 + 6), inv).draw(lcd).ok();
                    Text::new(val_str, Point::new(62, y0 + 6), inv).draw(lcd).ok();
                } else {
                    Text::new("Thr Trim:", Point::new(4, y0 + 6), text_style).draw(lcd).ok();
                    Text::new(val_str, Point::new(62, y0 + 6), text_style).draw(lcd).ok();
                }

                // Item 1: Audio Beeper (y = 20)
                let y1 = 20;
                let is_sel1 = self.selected_item == 1;
                let beeper_str = if storage.radio.audio_enabled != 0 { "ENABLED" } else { "MUTED" };
                if is_sel1 {
                    Rectangle::new(Point::new(2, y1), Size::new(124, 7)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Beeper:", Point::new(4, y1 + 6), inv).draw(lcd).ok();
                    Text::new(beeper_str, Point::new(62, y1 + 6), inv).draw(lcd).ok();
                } else {
                    Text::new("Beeper:", Point::new(4, y1 + 6), text_style).draw(lcd).ok();
                    Text::new(beeper_str, Point::new(62, y1 + 6), text_style).draw(lcd).ok();
                }

                // Item 2: Backlight Timeout (y = 27)
                let y2 = 27;
                let is_sel2 = self.selected_item == 2;
                let timer_str = match storage.radio.backlight_timeout {
                    1 => "15 SEC",
                    2 => "30 SEC",
                    3 => "60 SEC",
                    _ => "ALWAYS ON",
                };
                if is_sel2 {
                    Rectangle::new(Point::new(2, y2), Size::new(124, 7)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("BL Timer:", Point::new(4, y2 + 6), inv).draw(lcd).ok();
                    Text::new(timer_str, Point::new(62, y2 + 6), inv).draw(lcd).ok();
                } else {
                    Text::new("BL Timer:", Point::new(4, y2 + 6), text_style).draw(lcd).ok();
                    Text::new(timer_str, Point::new(62, y2 + 6), text_style).draw(lcd).ok();
                }

                // Item 3: Backlight Brightness (y = 34)
                let y3 = 34;
                let is_sel3 = self.selected_item == 3;
                let mut b_buf = *b"  %";
                let pct = (storage.radio.backlight_brightness * 10).min(100);
                if pct == 100 {
                    b_buf = *b"100";
                } else {
                    b_buf[0] = b' ';
                    b_buf[1] = b'0' + (pct / 10);
                    b_buf[2] = b'0';
                }
                let b_str = core::str::from_utf8(&b_buf).unwrap_or("100");

                if is_sel3 {
                    Rectangle::new(Point::new(2, y3), Size::new(124, 7)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("BL Level:", Point::new(4, y3 + 6), inv).draw(lcd).ok();
                    Text::new(b_str, Point::new(62, y3 + 6), inv).draw(lcd).ok();
                    Text::new("%", Point::new(82, y3 + 6), inv).draw(lcd).ok();
                } else {
                    Text::new("BL Level:", Point::new(4, y3 + 6), text_style).draw(lcd).ok();
                    Text::new(b_str, Point::new(62, y3 + 6), text_style).draw(lcd).ok();
                    Text::new("%", Point::new(82, y3 + 6), text_style).draw(lcd).ok();
                }

                // Item 4: Contrast (y = 41)
                let y4 = 41;
                let is_sel4 = self.selected_item == 4;
                let mut c_buf = *b"00";
                c_buf[0] = b'0' + (storage.radio.lcd_contrast / 10);
                c_buf[1] = b'0' + (storage.radio.lcd_contrast % 10);
                let c_str = core::str::from_utf8(&c_buf).unwrap_or("37");

                if is_sel4 {
                    Rectangle::new(Point::new(2, y4), Size::new(124, 7)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Contrast:", Point::new(4, y4 + 6), inv).draw(lcd).ok();
                    Text::new(c_str, Point::new(62, y4 + 6), inv).draw(lcd).ok();
                } else {
                    Text::new("Contrast:", Point::new(4, y4 + 6), text_style).draw(lcd).ok();
                    Text::new(c_str, Point::new(62, y4 + 6), text_style).draw(lcd).ok();
                }

                // Item 5: Battery Alarm (y = 48)
                let y5 = 48;
                let is_sel5 = self.selected_item == 5;
                let mut v_buf = *b"0.0V";
                v_buf[0] = b'0' + (storage.radio.vbat_warn_deci / 10);
                v_buf[2] = b'0' + (storage.radio.vbat_warn_deci % 10);
                let v_str = core::str::from_utf8(&v_buf).unwrap_or("4.4V");

                if is_sel5 {
                    Rectangle::new(Point::new(2, y5), Size::new(124, 7)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Bat Warn:", Point::new(4, y5 + 6), inv).draw(lcd).ok();
                    Text::new(v_str, Point::new(62, y5 + 6), inv).draw(lcd).ok();
                } else {
                    Text::new("Bat Warn:", Point::new(4, y5 + 6), text_style).draw(lcd).ok();
                    Text::new(v_str, Point::new(62, y5 + 6), text_style).draw(lcd).ok();
                }

                let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
                Line::new(Point::new(0, 55), Point::new(127, 55)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Toggle/Cycle   [ESC] Back", Point::new(2, 62), text_style_small).draw(lcd).ok();
            }

            MenuState::RxSetup => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 8;
                    self.scroll_offset = 5;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if ok_pressed {
                    self.request_bind = true;
                    self.state = MenuState::Closed; // Exit to main view to observe binding banner & cancel
                    buzzer.click();
                    return;
                }

                let active_idx = storage.radio.active_model as usize;
                Text::new("RX SETUP & BIND", Point::new(20, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                Text::new("Protocol: AFHDS 2A", Point::new(4, 22), text_style).draw(lcd).ok();

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

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Bind RX  [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::ChannelMonitor => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 9;
                    self.scroll_offset = 6;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if up_pressed || down_pressed {
                    self.page_idx = if self.page_idx == 0 { 1 } else { 0 };
                    buzzer.play_tone(2200, 30);
                }

                // Render Header
                let title = if self.page_idx == 0 { "CHANNELS (1-7)" } else { "CHANNELS (8-14)" };
                Text::new(title, Point::new(20, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                let start_ch = if self.page_idx == 0 { 0 } else { 7 };
                let ch_names = if self.page_idx == 0 {
                    ["1:ROL", "2:PIT", "3:THR", "4:YAW", "5:SwA", "6:SwB", "7:VR1"]
                } else {
                    ["8:VR2", "9:SC ", "10:SD", "11:CH", "12:CH", "13:CH", "14:CH"]
                };

                let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);

                for (i, name) in ch_names.iter().enumerate().take(7) {
                    let ch = start_ch + i;
                    let y = 12 + (i as i32 * 6);
                    Text::new(name, Point::new(2, y + 5), text_style_small).draw(lcd).ok();

                    let us = rf_chs[ch].clamp(1000, 2000);
                    Rectangle::new(Point::new(44, y + 1), Size::new(40, 5))
                        .into_styled(border_style)
                        .draw(lcd)
                        .ok();
                    let fill_w = (((us - 1000) as u32 * 38) / 1000).min(38);
                    if fill_w > 0 {
                        Rectangle::new(Point::new(45, y + 2), Size::new(fill_w, 3))
                            .into_styled(fill_style)
                            .draw(lcd)
                            .ok();
                    }

                    let mut val_buf = [0u8; 4];
                    u16_to_dec_4(us, &mut val_buf);
                    let val_str = core::str::from_utf8(&val_buf).unwrap_or("1500");
                    Text::new(val_str, Point::new(90, y + 5), text_style_small).draw(lcd).ok();
                }

                Line::new(Point::new(0, 55), Point::new(127, 55)).into_styled(border_style).draw(lcd).ok();
                Text::new("[UP/DN] Page  [ESC] Back", Point::new(2, 62), text_style_small).draw(lcd).ok();
            }

            MenuState::DiagAnas => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 11;
                    self.scroll_offset = 8;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if up_pressed || down_pressed {
                    self.page_idx = if self.page_idx == 0 { 1 } else { 0 };
                    buzzer.play_tone(2200, 30);
                }

                // Render Header
                let title = if self.page_idx == 0 { "ANALOG (1-6)" } else { "ANALOG (7-11)" };
                Text::new(title, Point::new(24, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);

                let start_idx = if self.page_idx == 0 { 0 } else { 6 };
                let names: &[&str] = if self.page_idx == 0 {
                    &["RH:AIL", "RV:ELE", "LV:THR", "LH:RUD", "SW:SA ", "SW:SB "]
                } else {
                    &["POT:V1", "POT:V2", "SW:SC ", "SW:SD ", "VBAT  "]
                };

                for (i, &name) in names.iter().enumerate() {
                    let adc_idx = start_idx + i;
                    let y = 12 + (i as i32 * 7);
                    Text::new(name, Point::new(2, y + 5), text_style_small).draw(lcd).ok();

                    let raw = raw_adc[adc_idx].min(4095);
                    Rectangle::new(Point::new(44, y + 1), Size::new(40, 5))
                        .into_styled(border_style)
                        .draw(lcd)
                        .ok();
                    let fill_w = ((raw as u32 * 38) / 4095).min(38);
                    if fill_w > 0 {
                        Rectangle::new(Point::new(45, y + 2), Size::new(fill_w, 3))
                            .into_styled(fill_style)
                            .draw(lcd)
                            .ok();
                    }

                    let mut val_buf = [0u8; 4];
                    u16_to_dec_4(raw, &mut val_buf);
                    let val_str = core::str::from_utf8(&val_buf).unwrap_or("0000");
                    Text::new(val_str, Point::new(90, y + 5), text_style_small).draw(lcd).ok();
                }

                Line::new(Point::new(0, 55), Point::new(127, 55)).into_styled(border_style).draw(lcd).ok();
                Text::new("[UP/DN] Page  [ESC] Back", Point::new(2, 62), text_style_small).draw(lcd).ok();
            }

            MenuState::SystemInfo => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 12;
                    self.scroll_offset = 9;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                let profile = chip::get_mcu_profile();

                Text::new("SYSTEM INFORMATION", Point::new(10, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                Text::new("MCU:", Point::new(4, 21), text_style).draw(lcd).ok();
                Text::new(profile.name, Point::new(36, 21), text_style).draw(lcd).ok();

                Text::new("Firmware: v0.1.0 Rust", Point::new(4, 30), text_style).draw(lcd).ok();

                Text::new("Flash: 128KB (64P)", Point::new(4, 39), text_style).draw(lcd).ok();
                Text::new("Profiles: 20 Models", Point::new(4, 48), text_style).draw(lcd).ok();

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[ESC] Back", Point::new(38, 62), text_style).draw(lcd).ok();
            }
        }
    }
}
