//! Settings and Diagnostics Menu Subsystem for FlySky FS-i6X.
//!
//! Provides navigation, 20-model memory management, channel reversing,
//! 5/9-point throttle curve editing with real-time spline visualization,
//! configuration editing with Flash persistence, live channel monitoring,
//! and raw ADC diagnostics.

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MenuState {
    Closed,
    MainMenu,
    ModelSelect,
    ModelSetup,
    ChannelReverse,
    ThrottleCurve,
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
    pub request_calibration: bool,
    prev_keys: u16,
    waiting_release: bool,
}

impl MenuController {
    pub const fn new() -> Self {
        Self {
            state: MenuState::Closed,
            selected_item: 0,
            scroll_offset: 0,
            page_idx: 0,
            sub_idx: 0,
            request_calibration: false,
            prev_keys: 0xFFFF,
            waiting_release: false,
        }
    }

    /// Open the main settings menu.
    pub fn open(&mut self, buzzer: &mut Buzzer) {
        self.state = MenuState::MainMenu;
        self.selected_item = 0;
        self.scroll_offset = 0;
        self.page_idx = 0;
        self.sub_idx = 0;
        self.request_calibration = false;
        self.waiting_release = true;
        self.prev_keys = 0xFFFF;
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
        // Key release tracking (bit 10: OK, bit 11: Cancel, bit 9: Up, bit 8: Down)
        if self.waiting_release && (keys & ((1 << 8) | (1 << 9) | (1 << 10) | (1 << 11))) == 0 {
            self.waiting_release = false;
        }

        let newly_pressed = if self.waiting_release {
            0
        } else {
            keys & !self.prev_keys
        };
        self.prev_keys = keys;

        let down_pressed = (newly_pressed & (1 << 8)) != 0;
        let up_pressed = (newly_pressed & (1 << 9)) != 0;
        let ok_pressed = (newly_pressed & (1 << 10)) != 0;
        let cancel_pressed = (newly_pressed & (1 << 11)) != 0;

        let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
        let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);

        lcd.clear(BinaryColor::Off).ok();

        match self.state {
            MenuState::Closed => {}

            MenuState::MainMenu => {
                const ITEM_COUNT: usize = 10;
                let items = [
                    "1. Model Select",
                    "2. Model Setup",
                    "3. Ch Reverse",
                    "4. Thr Curve",
                    "5. Radio Setup",
                    "6. RX Setup & Bind",
                    "7. Channel Monitor",
                    "8. Calibration",
                    "9. Analog Diag",
                    "10. System Info",
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
                            self.state = MenuState::ChannelReverse;
                            self.selected_item = 0;
                            self.scroll_offset = 0;
                        }
                        3 => {
                            self.state = MenuState::ThrottleCurve;
                            self.selected_item = 0;
                        }
                        4 => {
                            self.state = MenuState::RadioSetup;
                            self.selected_item = 0;
                        }
                        5 => {
                            self.state = MenuState::RxSetup;
                            self.selected_item = 0;
                        }
                        6 => {
                            self.state = MenuState::ChannelMonitor;
                            self.page_idx = 0;
                        }
                        7 => {
                            self.request_calibration = true;
                            self.state = MenuState::Closed;
                            return;
                        }
                        8 => {
                            self.state = MenuState::DiagAnas;
                        }
                        9 => {
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

                if cancel_pressed {
                    storage::save_storage(storage);
                    self.state = MenuState::MainMenu;
                    self.selected_item = 1;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                // 3 fields: 0 = Name, 1 = Type, 2 = Reset
                if self.selected_item == 0 {
                    // Editing Name
                    if ok_pressed {
                        self.sub_idx = (self.sub_idx + 1) % 10;
                        buzzer.click();
                    }
                    if down_pressed {
                        let c = storage.models[active_idx].name[self.sub_idx];
                        storage.models[active_idx].name[self.sub_idx] = next_ascii(c);
                        buzzer.play_tone(2200, 20);
                    }
                    if up_pressed {
                        let c = storage.models[active_idx].name[self.sub_idx];
                        storage.models[active_idx].name[self.sub_idx] = prev_ascii(c);
                        buzzer.play_tone(2200, 20);
                    }
                } else if self.selected_item == 1 {
                    // Model Type
                    if ok_pressed || down_pressed {
                        storage.models[active_idx].model_type = (storage.models[active_idx].model_type + 1) % 4;
                        buzzer.click();
                    }
                    if up_pressed {
                        storage.models[active_idx].model_type = if storage.models[active_idx].model_type == 0 { 3 } else { storage.models[active_idx].model_type - 1 };
                        buzzer.click();
                    }
                } else if self.selected_item == 2 {
                    // Reset to Default
                    if ok_pressed {
                        storage.models[active_idx] = ModelConfig::default_for_index(active_idx);
                        storage::save_storage(storage);
                        buzzer.play_tone_pattern(2200, 80, 50, 2);
                        self.waiting_release = true;
                    }
                }

                // Hold down to jump between fields
                // Right now, user can cycle fields or we add a cursor
                // For simplicity: Header, Name, Type, Reset
                Text::new("MODEL SETUP", Point::new(30, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                // Field 0: Name
                Text::new("Name:", Point::new(4, 23), text_style).draw(lcd).ok();
                let name_str = core::str::from_utf8(&storage.models[active_idx].name).unwrap_or("----------");
                Text::new(name_str, Point::new(46, 23), text_style).draw(lcd).ok();

                // Draw underline under currently edited character
                let char_x = 46 + (self.sub_idx as i32 * 6);
                Line::new(Point::new(char_x, 25), Point::new(char_x + 5, 25))
                    .into_styled(border_style)
                    .draw(lcd)
                    .ok();

                // Field 1: Type
                Text::new("Type:", Point::new(4, 35), text_style).draw(lcd).ok();
                let type_str = match storage.models[active_idx].model_type {
                    1 => "AIRPLANE",
                    2 => "HELICOPTER",
                    3 => "GLIDER",
                    _ => "MULTI / QUAD",
                };
                Text::new(type_str, Point::new(46, 35), text_style).draw(lcd).ok();

                // Field 2: Reset
                Text::new("Reset: [OK Defaults]", Point::new(4, 47), text_style).draw(lcd).ok();

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[UP/DN] Char  [OK] Next", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::ChannelReverse => {
                let active_idx = storage.radio.active_model as usize;
                const CH_COUNT: usize = 14;

                if cancel_pressed {
                    storage::save_storage(storage);
                    self.state = MenuState::MainMenu;
                    self.selected_item = 2;
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
                let max_items = 2 + pts_count; // Item 0: Pts mode, Item 1: Smooth, Item 2..: Points

                if cancel_pressed {
                    storage::save_storage(storage);
                    self.state = MenuState::MainMenu;
                    self.selected_item = 3;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if ok_pressed {
                    if self.selected_item == 0 {
                        // Toggle 5-PT <-> 9-PT
                        storage.models[active_idx].thr_curve_pts = if storage.models[active_idx].thr_curve_pts == 9 { 5 } else { 9 };
                        buzzer.click();
                    } else if self.selected_item == 1 {
                        // Toggle Linear <-> Smooth (Catmull-Rom spline)
                        storage.models[active_idx].thr_curve_smooth = if storage.models[active_idx].thr_curve_smooth == 0 { 1 } else { 0 };
                        buzzer.click();
                    }
                }

                if self.selected_item >= 2 {
                    let pt_idx = self.selected_item - 2;
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
                } else {
                    if down_pressed {
                        self.selected_item = (self.selected_item + 1) % max_items;
                        buzzer.play_tone(2200, 20);
                    }
                    if up_pressed {
                        self.selected_item = if self.selected_item == 0 { max_items - 1 } else { self.selected_item - 1 };
                        buzzer.play_tone(2200, 20);
                    }
                }

                // Render Header
                Text::new("THROTTLE CURVE", Point::new(20, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                // Left side: Mode & active point info
                let mode_str = if storage.models[active_idx].thr_curve_pts == 9 { "9-PT" } else { "5-PT" };
                Text::new("Pts:", Point::new(2, 21), text_style).draw(lcd).ok();
                Text::new(mode_str, Point::new(32, 21), text_style).draw(lcd).ok();

                let smooth_str = if storage.models[active_idx].thr_curve_smooth != 0 { "SMOOTH" } else { "LINEAR" };
                Text::new("Crv:", Point::new(2, 31), text_style).draw(lcd).ok();
                Text::new(smooth_str, Point::new(32, 31), text_style).draw(lcd).ok();

                if self.selected_item >= 2 {
                    let pt_idx = self.selected_item - 2;
                    let val = storage.models[active_idx].thr_curve[pt_idx];
                    let mut p_buf = *b"P1:  %";
                    p_buf[1] = b'1' + pt_idx as u8;
                    p_buf[3] = b'0' + (val / 10);
                    p_buf[4] = b'0' + (val % 10);
                    let p_str = core::str::from_utf8(&p_buf).unwrap_or("P?:--%");
                    Text::new(p_str, Point::new(2, 45), text_style).draw(lcd).ok();
                } else {
                    Text::new("Select Pt", Point::new(2, 45), text_style).draw(lcd).ok();
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

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Mode     [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::RadioSetup => {
                const SETUP_ITEMS: usize = 4;
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 4;
                    self.scroll_offset = 0;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

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
                        _ => {}
                    }
                }

                // Render Header
                Text::new("RADIO SETUP", Point::new(32, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                // Item 0: Throttle Trim
                let y0 = 14;
                let is_sel0 = self.selected_item == 0;
                let val_str = match storage.radio.throttle_trim {
                    1 => "IDLE",
                    2 => "LINEAR",
                    _ => "OFF (Lock)",
                };
                if is_sel0 {
                    Rectangle::new(Point::new(2, y0), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Thr Trim:", Point::new(4, y0 + 7), inv).draw(lcd).ok();
                    Text::new(val_str, Point::new(62, y0 + 7), inv).draw(lcd).ok();
                } else {
                    Text::new("Thr Trim:", Point::new(4, y0 + 7), text_style).draw(lcd).ok();
                    Text::new(val_str, Point::new(62, y0 + 7), text_style).draw(lcd).ok();
                }

                // Item 1: Audio Beeper
                let y1 = 23;
                let is_sel1 = self.selected_item == 1;
                if is_sel1 {
                    Rectangle::new(Point::new(2, y1), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Beeper:", Point::new(4, y1 + 7), inv).draw(lcd).ok();
                    let val_str = if storage.radio.audio_enabled != 0 { "ENABLED" } else { "MUTED" };
                    Text::new(val_str, Point::new(62, y1 + 7), inv).draw(lcd).ok();
                } else {
                    Text::new("Beeper:", Point::new(4, y1 + 7), text_style).draw(lcd).ok();
                    let val_str = if storage.radio.audio_enabled != 0 { "ENABLED" } else { "MUTED" };
                    Text::new(val_str, Point::new(62, y1 + 7), text_style).draw(lcd).ok();
                }

                // Item 2: Backlight Timeout
                let y2 = 32;
                let is_sel2 = self.selected_item == 2;
                let timer_str = match storage.radio.backlight_timeout {
                    1 => "15 SEC",
                    2 => "30 SEC",
                    3 => "60 SEC",
                    _ => "ALWAYS ON",
                };
                if is_sel2 {
                    Rectangle::new(Point::new(2, y2), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("BL Timer:", Point::new(4, y2 + 7), inv).draw(lcd).ok();
                    Text::new(timer_str, Point::new(62, y2 + 7), inv).draw(lcd).ok();
                } else {
                    Text::new("BL Timer:", Point::new(4, y2 + 7), text_style).draw(lcd).ok();
                    Text::new(timer_str, Point::new(62, y2 + 7), text_style).draw(lcd).ok();
                }

                // Item 3: Backlight Brightness
                let y3 = 41;
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
                    Rectangle::new(Point::new(2, y3), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("BL Level:", Point::new(4, y3 + 7), inv).draw(lcd).ok();
                    Text::new(b_str, Point::new(62, y3 + 7), inv).draw(lcd).ok();
                    Text::new("%", Point::new(82, y3 + 7), inv).draw(lcd).ok();
                } else {
                    Text::new("BL Level:", Point::new(4, y3 + 7), text_style).draw(lcd).ok();
                    Text::new(b_str, Point::new(62, y3 + 7), text_style).draw(lcd).ok();
                    Text::new("%", Point::new(82, y3 + 7), text_style).draw(lcd).ok();
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Toggle   [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
            }

            MenuState::RxSetup => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                if ok_pressed {
                    rf::set_bind_mode(true);
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
                Text::new("[ESC] Back", Point::new(38, 62), text_style).draw(lcd).ok();
            }

            MenuState::ChannelMonitor => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
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

                for (i, name) in ch_names.iter().enumerate().take(7) {
                    let ch = start_ch + i;
                    let y = 13 + (i as i32 * 6);
                    Text::new(name, Point::new(2, y + 5), text_style).draw(lcd).ok();

                    let us = rf_chs[ch].clamp(1000, 2000);
                    Rectangle::new(Point::new(44, y), Size::new(40, 5))
                        .into_styled(border_style)
                        .draw(lcd)
                        .ok();
                    let fill_w = (((us - 1000) as u32 * 38) / 1000).min(38);
                    if fill_w > 0 {
                        Rectangle::new(Point::new(45, y + 1), Size::new(fill_w, 3))
                            .into_styled(fill_style)
                            .draw(lcd)
                            .ok();
                    }

                    let mut val_buf = [0u8; 4];
                    u16_to_dec_4(us, &mut val_buf);
                    let val_str = core::str::from_utf8(&val_buf).unwrap_or("1500");
                    Text::new(val_str, Point::new(90, y + 5), text_style).draw(lcd).ok();
                }

                Line::new(Point::new(0, 55), Point::new(127, 55)).into_styled(border_style).draw(lcd).ok();
                Text::new("[UP/DN] Page  [ESC] Back", Point::new(2, 63), text_style).draw(lcd).ok();
            }

            MenuState::DiagAnas => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                Text::new("ANALOG DIAGNOSTICS", Point::new(10, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                let names = [
                    "RH:AIL", "RV:ELE", "LV:THR", "LH:RUD",
                    "SW:SA ", "SW:SB ", "POT:V1", "POT:V2",
                    "SW:SC ", "SW:SD ", "VBAT  ",
                ];

                for i in 0..6 {
                    let y = 14 + (i as i32 * 6);
                    Text::new(names[i], Point::new(2, y + 5), text_style).draw(lcd).ok();
                    let mut b = [0u8; 4];
                    u16_to_dec_4(raw_adc[i], &mut b);
                    let s = core::str::from_utf8(&b).unwrap_or("0000");
                    Text::new(s, Point::new(44, y + 5), text_style).draw(lcd).ok();
                }

                for i in 6..11 {
                    let y = 14 + ((i - 6) as i32 * 6);
                    Text::new(names[i], Point::new(70, y + 5), text_style).draw(lcd).ok();
                    let mut b = [0u8; 4];
                    u16_to_dec_4(raw_adc[i], &mut b);
                    let s = core::str::from_utf8(&b).unwrap_or("0000");
                    Text::new(s, Point::new(102, y + 5), text_style).draw(lcd).ok();
                }

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[ESC] Back", Point::new(38, 62), text_style).draw(lcd).ok();
            }

            MenuState::SystemInfo => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
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
