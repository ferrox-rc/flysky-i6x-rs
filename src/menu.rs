//! Settings and Diagnostics Menu Subsystem for FlySky FS-i6X.
//!
//! Provides navigation, configuration editing with Flash persistence,
//! live channel monitoring, raw ADC diagnostics, and hardware information.

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
use crate::display::St7567;
use crate::rf;
use crate::storage::{self, RadioConfig};
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MenuState {
    Closed,
    MainMenu,
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
        self.request_calibration = false;
        self.waiting_release = true; // wait for key release before accepting clicks
        self.prev_keys = 0xFFFF;
        buzzer.click();
    }

    /// Returns true if any menu or diagnostic screen is active.
    pub fn is_active(&self) -> bool {
        self.state != MenuState::Closed
    }

    /// Process navigation keys, update menu state, and render display.
    pub fn update(
        &mut self,
        lcd: &mut St7567,
        keys: u16,
        config: &mut RadioConfig,
        trims: &mut TrimController,
        raw_adc: &[u16; adc::NUM_CHANNELS],
        rf_chs: &[u16; 14],
        buzzer: &mut Buzzer,
    ) {
        // Key release tracking (bit 10 is OK, bit 11 is Cancel, bit 9 is Up, bit 8 is Down)
        if self.waiting_release {
            if (keys & ((1 << 8) | (1 << 9) | (1 << 10) | (1 << 11))) == 0 {
                self.waiting_release = false;
            }
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
                const ITEM_COUNT: usize = 6;
                let items = [
                    "1. Radio Setup",
                    "2. Calibration",
                    "3. RX Setup & Bind",
                    "4. Channel Monitor",
                    "5. Analog Diag",
                    "6. System Info",
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
                            self.state = MenuState::RadioSetup;
                            self.selected_item = 0;
                        }
                        1 => {
                            self.request_calibration = true;
                            self.state = MenuState::Closed;
                            return;
                        }
                        2 => {
                            self.state = MenuState::RxSetup;
                            self.selected_item = 0;
                        }
                        3 => {
                            self.state = MenuState::ChannelMonitor;
                            self.page_idx = 0;
                        }
                        4 => {
                            self.state = MenuState::DiagAnas;
                        }
                        5 => {
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

            MenuState::RadioSetup => {
                const SETUP_ITEMS: usize = 4;
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.selected_item = 0;
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
                            // Toggle Throttle Trim
                            config.throttle_trim = if config.throttle_trim == 0 { 1 } else { 0 };
                            trims.throttle_enabled = config.throttle_trim != 0;
                            storage::save_config(config);
                        }
                        1 => {
                            // Toggle Audio
                            config.audio_enabled = if config.audio_enabled == 0 { 1 } else { 0 };
                            buzzer.enabled = config.audio_enabled != 0;
                            storage::save_config(config);
                        }
                        2 => {
                            // Cycle Backlight Timeout: 0=Always On, 1=15s, 2=30s, 3=60s
                            config.backlight_timeout = (config.backlight_timeout + 1) % 4;
                            storage::save_config(config);
                        }
                        3 => {
                            // Cycle Backlight Brightness: 1..10 (10%..100%)
                            config.backlight_brightness = if config.backlight_brightness >= 10 {
                                1
                            } else {
                                config.backlight_brightness + 1
                            };
                            lcd.set_backlight_level(config.backlight_brightness * 10);
                            storage::save_config(config);
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
                if is_sel0 {
                    Rectangle::new(Point::new(2, y0), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Thr Trim:", Point::new(4, y0 + 7), inv).draw(lcd).ok();
                    let val_str = if config.throttle_trim != 0 { "ENABLED" } else { "OFF (Lock)" };
                    Text::new(val_str, Point::new(62, y0 + 7), inv).draw(lcd).ok();
                } else {
                    Text::new("Thr Trim:", Point::new(4, y0 + 7), text_style).draw(lcd).ok();
                    let val_str = if config.throttle_trim != 0 { "ENABLED" } else { "OFF (Lock)" };
                    Text::new(val_str, Point::new(62, y0 + 7), text_style).draw(lcd).ok();
                }

                // Item 1: Audio Beeper
                let y1 = 23;
                let is_sel1 = self.selected_item == 1;
                if is_sel1 {
                    Rectangle::new(Point::new(2, y1), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                    let inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
                    Text::new("Beeper:", Point::new(4, y1 + 7), inv).draw(lcd).ok();
                    let val_str = if config.audio_enabled != 0 { "ENABLED" } else { "MUTED" };
                    Text::new(val_str, Point::new(62, y1 + 7), inv).draw(lcd).ok();
                } else {
                    Text::new("Beeper:", Point::new(4, y1 + 7), text_style).draw(lcd).ok();
                    let val_str = if config.audio_enabled != 0 { "ENABLED" } else { "MUTED" };
                    Text::new(val_str, Point::new(62, y1 + 7), text_style).draw(lcd).ok();
                }

                // Item 2: Backlight Timeout
                let y2 = 32;
                let is_sel2 = self.selected_item == 2;
                let timer_str = match config.backlight_timeout {
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
                let mut b_buf = [b' ', b' ', b'%'];
                let pct = (config.backlight_brightness * 10).min(100);
                if pct == 100 {
                    b_buf = [b'1', b'0', b'0'];
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

                // Render Footer
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

                Text::new("RX SETUP & BIND", Point::new(20, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                Text::new("Protocol: AFHDS 2A", Point::new(4, 22), text_style).draw(lcd).ok();

                let mut rx_buf = [b'0'; 8];
                u32_to_hex(config.rx_id, &mut rx_buf);
                let rx_str = core::str::from_utf8(&rx_buf).unwrap_or("00000000");
                Text::new("Bound RX: 0x", Point::new(4, 32), text_style).draw(lcd).ok();
                Text::new(rx_str, Point::new(76, 32), text_style).draw(lcd).ok();

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[OK] Bind     [ESC] Back", Point::new(2, 62), text_style).draw(lcd).ok();
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
                    ["1:AIL", "2:ELE", "3:THR", "4:RUD", "5:SA ", "6:SB ", "7:VR1"]
                } else {
                    ["8:VR2", "9:SC ", "10:SD", "11:CH", "12:CH", "13:CH", "14:CH"]
                };

                for i in 0..7 {
                    let ch = start_ch + i;
                    let y = 13 + (i as i32 * 6);
                    Text::new(ch_names[i], Point::new(2, y + 5), text_style).draw(lcd).ok();

                    // Bar graph width 40 pixels (x = 44..84)
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

                    // Text microseconds
                    let mut us_buf = [b'0'; 4];
                    u16_to_dec_4(us, &mut us_buf);
                    let us_str = core::str::from_utf8(&us_buf).unwrap_or("1500");
                    Text::new(us_str, Point::new(88, y + 5), text_style).draw(lcd).ok();
                    Text::new("us", Point::new(114, y + 5), text_style).draw(lcd).ok();
                }

                Line::new(Point::new(0, 56), Point::new(127, 56)).into_styled(border_style).draw(lcd).ok();
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

                // Left Column: Sticks & Battery
                // PA0 (RH), PA1 (RV), PA2 (LV), PA3 (LH), PC0 (VBAT)
                let labels_left = ["RH:", "RV:", "LV:", "LH:", "BT:"];
                let channels_left = [0, 1, 2, 3, 10];
                for i in 0..5 {
                    let y = 14 + (i as i32 * 8);
                    Text::new(labels_left[i], Point::new(2, y + 6), text_style).draw(lcd).ok();
                    let val = raw_adc[channels_left[i]];
                    let mut buf = [b'0'; 4];
                    u16_to_dec_4(val, &mut buf);
                    let s = core::str::from_utf8(&buf).unwrap_or("0000");
                    Text::new(s, Point::new(22, y + 6), text_style).draw(lcd).ok();
                }

                // Right Column: Pots & Switches
                // PA6 (VRA), PA7 (VRB), PA4 (SA), PA5 (SB), PB0 (SC), PB1 (SD)
                let labels_right = ["V1:", "V2:", "SA:", "SB:", "SC:", "SD:"];
                let channels_right = [6, 7, 4, 5, 8, 9];
                for i in 0..6 {
                    let y = 14 + (i as i32 * 7);
                    Text::new(labels_right[i], Point::new(56, y + 5), text_style).draw(lcd).ok();
                    let val = raw_adc[channels_right[i]];
                    let mut buf = [b'0'; 4];
                    u16_to_dec_4(val, &mut buf);
                    let s = core::str::from_utf8(&buf).unwrap_or("0000");
                    Text::new(s, Point::new(76, y + 5), text_style).draw(lcd).ok();

                    // For switches, show state character (U / M / D)
                    if i >= 2 {
                        let sw_char = match val {
                            v if v < 1365 => "U",
                            v if v < 2730 => "M",
                            _ => "D",
                        };
                        Text::new(sw_char, Point::new(104, y + 5), text_style).draw(lcd).ok();
                    }
                }

                Line::new(Point::new(0, 56), Point::new(127, 56)).into_styled(border_style).draw(lcd).ok();
                Text::new("[ESC] Back to Menu", Point::new(12, 63), text_style).draw(lcd).ok();
            }

            MenuState::SystemInfo => {
                if cancel_pressed {
                    self.state = MenuState::MainMenu;
                    self.waiting_release = true;
                    buzzer.click();
                    return;
                }

                let mcu = chip::get_mcu_profile();
                let uid = chip::read_uid(&mcu);

                Text::new("SYSTEM INFORMATION", Point::new(10, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                Text::new("FW: flysky-i6x-rs v0.1", Point::new(2, 20), text_style).draw(lcd).ok();

                Text::new("MCU:", Point::new(2, 29), text_style).draw(lcd).ok();
                Text::new(mcu.name, Point::new(28, 29), text_style).draw(lcd).ok();

                // Format 96-bit UID
                let mut uid_hex = [b'0'; 16];
                for i in 0..8 {
                    uid_hex[i * 2] = HEX_CHARS[((uid[i] >> 4) & 0x0F) as usize];
                    uid_hex[i * 2 + 1] = HEX_CHARS[(uid[i] & 0x0F) as usize];
                }
                let uid_str = core::str::from_utf8(&uid_hex).unwrap_or("UID");
                Text::new("UID:", Point::new(2, 38), text_style).draw(lcd).ok();
                Text::new(uid_str, Point::new(28, 38), text_style).draw(lcd).ok();

                Text::new("Flash: 24KB / 128KB", Point::new(2, 47), text_style).draw(lcd).ok();

                Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
                Text::new("[ESC] Back to Menu", Point::new(12, 62), text_style).draw(lcd).ok();
            }
        }
    }
}
