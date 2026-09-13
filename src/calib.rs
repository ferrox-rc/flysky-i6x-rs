//! Interactive endpoint calibration subprogram.
//!
//! Guides the user through a 2-step wizard:
//! 1. Center all sticks and pots (Throttle centered to 50%) -> captures neutral points.
//! 2. Move sticks and pots to physical limits -> captures min/max extents.
//! Applies OpenTX-style margins (~2%) and saves to Flash.

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::adc;
use crate::buzzer::Buzzer;
use crate::display::St7567;
use crate::input;
use crate::storage::{self, ChannelCalib};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CalibStep {
    Inactive,
    Center,
    Limits,
    Complete,
}

pub struct CalibWizard {
    pub step: CalibStep,
    centers: [u16; 6], // 0..3: Roll, Pitch, Throttle, Yaw; 4..5: VRA, VRB
    mins: [u16; 6],
    maxs: [u16; 6],
    prev_keys: u16,
    waiting_release: bool,
    timer_ms: u16,
}

impl CalibWizard {
    pub const fn new() -> Self {
        Self {
            step: CalibStep::Inactive,
            centers: [2048; 6],
            mins: [4095; 6],
            maxs: [0; 6],
            prev_keys: 0xFFFF,
            waiting_release: false,
            timer_ms: 0,
        }
    }

    /// Start the calibration wizard.
    pub fn start(&mut self, buzzer: &mut Buzzer) {
        self.step = CalibStep::Center;
        self.centers = [2048; 6];
        self.mins = [4095; 6];
        self.maxs = [0; 6];
        self.prev_keys = 0xFFFF; // Block any immediate edge trigger
        self.waiting_release = true; // User must release OK before proceeding
        self.timer_ms = 0;
        buzzer.click();
    }

    /// Returns true if the wizard is currently active.
    pub fn is_active(&self) -> bool {
        self.step != CalibStep::Inactive
    }

    /// Update wizard state and render screen.
    pub fn update(
        &mut self,
        lcd: &mut St7567,
        raw_adc: &[u16; adc::NUM_CHANNELS],
        keys: u16,
        dt_ms: u16,
        buzzer: &mut Buzzer,
    ) {
        // Debounce / release tracking for OK button (bit 10)
        if self.waiting_release {
            if (keys & (1 << 10)) == 0 {
                self.waiting_release = false;
            }
        }

        let newly_pressed = if self.waiting_release {
            0
        } else {
            keys & !self.prev_keys
        };
        self.prev_keys = keys;

        let ok_pressed = (newly_pressed & (1 << 10)) != 0;
        let cancel_pressed = (newly_pressed & (1 << 11)) != 0;

        let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

        // Map raw ADC to wizard channels:
        // [0] PA0: Roll
        // [1] PA1: Pitch
        // [2] PA2: Throttle
        // [3] PA3: Yaw
        // [4] PA6: VRA
        // [5] PA7: VRB
        let current_raw = [
            raw_adc[0],
            raw_adc[1],
            raw_adc[2],
            raw_adc[3],
            raw_adc[6],
            raw_adc[7],
        ];

        match self.step {
            CalibStep::Inactive => {}

            CalibStep::Center => {
                if cancel_pressed {
                    self.step = CalibStep::Inactive;
                    buzzer.click();
                    return;
                }

                if ok_pressed {
                    for i in 0..6 {
                        self.centers[i] = current_raw[i];
                        self.mins[i] = current_raw[i];
                        self.maxs[i] = current_raw[i];
                    }
                    self.step = CalibStep::Limits;
                    self.waiting_release = true; // Must release OK before accepting save in step 2!
                    buzzer.play_tone(2400, 50);
                    return;
                }

                // Render Step 1
                lcd.clear(BinaryColor::Off).ok();
                Text::new("CALIBRATION (1/2)", Point::new(12, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                Text::new("1. Center all sticks", Point::new(2, 22), text_style).draw(lcd).ok();
                Text::new("   & rotary pots.", Point::new(2, 32), text_style).draw(lcd).ok();
                Text::new("2. Move THR to middle!", Point::new(2, 43), text_style).draw(lcd).ok();

                Line::new(Point::new(0, 48), Point::new(127, 48)).into_styled(border_style).draw(lcd).ok();
                if self.waiting_release {
                    Text::new("Release [OK] key...", Point::new(4, 59), text_style).draw(lcd).ok();
                } else {
                    Text::new("[OK] Next  [ESC] Exit", Point::new(2, 59), text_style).draw(lcd).ok();
                }
            }

            CalibStep::Limits => {
                if cancel_pressed {
                    self.step = CalibStep::Inactive;
                    buzzer.click();
                    return;
                }

                // Continuously track minimum and maximum reached
                for i in 0..6 {
                    if current_raw[i] < self.mins[i] {
                        self.mins[i] = current_raw[i];
                    }
                    if current_raw[i] > self.maxs[i] {
                        self.maxs[i] = current_raw[i];
                    }
                }

                // Check readiness of sticks (each stick must move at least 250 counts in each direction)
                let mut ready_count = 0u8;
                let mut stick_ready = [false; 4];
                for i in 0..4 {
                    let span_neg = self.centers[i].saturating_sub(self.mins[i]);
                    let span_pos = self.maxs[i].saturating_sub(self.centers[i]);
                    if span_neg >= 250 && span_pos >= 250 {
                        stick_ready[i] = true;
                        ready_count += 1;
                    }
                }

                let all_ready = ready_count == 4;

                if ok_pressed {
                    if all_ready {
                        // Apply OpenTX STICK_TOLERANCE 64 margin (62/64 = ~96.8%)
                        let mut cfg = storage::load_config();
                        cfg.magic = storage::FLASH_MAGIC;
                        cfg.version = storage::CONFIG_VERSION;

                        for i in 0..4 {
                            let center = self.centers[i];
                            let span_neg = ((center.saturating_sub(self.mins[i]) as u32 * 62) / 64) as u16;
                            let span_pos = ((self.maxs[i].saturating_sub(center) as u32 * 62) / 64) as u16;
                            cfg.sticks[i] = ChannelCalib::new(
                                center.saturating_sub(span_neg),
                                center,
                                center.saturating_add(span_pos),
                            );
                        }

                        // Pots: if moved by >= 500 counts total span, update; otherwise preserve
                        for i in 0..2 {
                            let p_idx = 4 + i;
                            let span = self.maxs[p_idx].saturating_sub(self.mins[p_idx]);
                            if span >= 500 {
                                let center = self.centers[p_idx];
                                let span_neg = ((center.saturating_sub(self.mins[p_idx]) as u32 * 62) / 64) as u16;
                                let span_pos = ((self.maxs[p_idx].saturating_sub(center) as u32 * 62) / 64) as u16;
                                cfg.pots[i] = ChannelCalib::new(
                                    center.saturating_sub(span_neg),
                                    center,
                                    center.saturating_add(span_pos),
                                );
                            }
                        }

                        input::apply_calibration(&cfg);
                        storage::save_config(&cfg);

                        buzzer.play_tone(2800, 150);
                        self.step = CalibStep::Complete;
                        self.timer_ms = 1200; // Display success banner for 1.2s
                    } else {
                        // Warning buzz if not all sticks have been moved to limits
                        buzzer.play_tone(1100, 100);
                    }
                    return;
                }

                // Render Step 2
                lcd.clear(BinaryColor::Off).ok();
                Text::new("CALIBRATION (2/2)", Point::new(12, 9), text_style).draw(lcd).ok();
                Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();

                let labels = ["A", "E", "T", "R"];
                for i in 0..4 {
                    let y = 13 + (i as i32 * 8);
                    Text::new(labels[i], Point::new(2, y + 6), text_style).draw(lcd).ok();

                    // Outline gauge box (width = 80)
                    Rectangle::new(Point::new(12, y), Size::new(80, 7))
                        .into_styled(border_style)
                        .draw(lcd)
                        .ok();

                    // Center tick mark at 52 (center of 80 box: 12 + 40 = 52)
                    Line::new(Point::new(52, y), Point::new(52, y + 6))
                        .into_styled(border_style)
                        .draw(lcd)
                        .ok();

                    // Display extent covered from center
                    let center = self.centers[i];
                    let span_neg = center.saturating_sub(self.mins[i]);
                    let span_pos = self.maxs[i].saturating_sub(center);

                    // Left fill: up to 38 pixels left
                    let left_w = ((span_neg as u32 * 38) / 1600).min(38) as i32;
                    if left_w > 0 {
                        Line::new(Point::new(52 - left_w, y + 3), Point::new(52, y + 3))
                            .into_styled(border_style)
                            .draw(lcd)
                            .ok();
                    }

                    // Right fill: up to 38 pixels right
                    let right_w = ((span_pos as u32 * 38) / 1600).min(38) as i32;
                    if right_w > 0 {
                        Line::new(Point::new(52, y + 3), Point::new(52 + right_w, y + 3))
                            .into_styled(border_style)
                            .draw(lcd)
                            .ok();
                    }

                    // Current stick position tick mark
                    let cur = current_raw[i];
                    let delta = cur as i32 - center as i32;
                    let cur_x = (52 + ((delta * 38) / 1600)).clamp(13, 90);
                    Line::new(Point::new(cur_x, y + 1), Point::new(cur_x, y + 5))
                        .into_styled(border_style)
                        .draw(lcd)
                        .ok();

                    // Status text on right
                    if stick_ready[i] {
                        Text::new("OK", Point::new(96, y + 6), text_style).draw(lcd).ok();
                    } else {
                        Text::new("--", Point::new(96, y + 6), text_style).draw(lcd).ok();
                    }
                }

                Line::new(Point::new(0, 48), Point::new(127, 48)).into_styled(border_style).draw(lcd).ok();
                if self.waiting_release {
                    Text::new("Release [OK] key...", Point::new(4, 59), text_style).draw(lcd).ok();
                } else if all_ready {
                    Text::new("[OK] Save  [ESC] Exit", Point::new(2, 59), text_style).draw(lcd).ok();
                } else {
                    Text::new("Stir sticks to stops", Point::new(2, 59), text_style).draw(lcd).ok();
                }
            }

            CalibStep::Complete => {
                lcd.clear(BinaryColor::Off).ok();
                Text::new("CALIBRATION SAVED!", Point::new(10, 24), text_style).draw(lcd).ok();
                Text::new("Flash updated OK", Point::new(14, 38), text_style).draw(lcd).ok();

                if self.timer_ms > dt_ms {
                    self.timer_ms -= dt_ms;
                } else {
                    self.step = CalibStep::Inactive;
                }
            }
        }
    }
}
