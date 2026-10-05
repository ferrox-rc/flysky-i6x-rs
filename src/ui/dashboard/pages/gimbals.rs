//! Page 0 (P1/4): Primary Gimbals & Trims, Switches, Pots, and Trim status.

use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};

use crate::display::St7567;
use crate::input::InputState;
use crate::storage::RadioStorage;
use crate::trim::{self, TrimController};
use crate::ui::format::{format_percent, format_throttle_percent, format_trim};
use crate::ui::glyphs::draw_switch_arrow;
use crate::ui::widgets;

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub fn render(
    lcd: &mut St7567,
    state: &InputState,
    storage: &RadioStorage,
    trims: &TrimController,
    is_binding: bool,
    timer_str: Option<&str>,
    timer_expired: bool,
    blink_on: bool,
) {
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
    let mut pct_buf = [0u8; 5];
    let is_general = storage.active_model().model_type == 4;

    // CH1: Roll / 1 (y = 13)
    let lbl1 = if is_general { "1" } else { "A" };
    Text::new(lbl1, Point::new(2, 19), text_style)
        .draw(lcd)
        .ok();
    widgets::draw_channel_gauge(lcd, 12, 13, 76, 7, state.sticks.roll, trims.values.roll);
    let p1 = format_percent(state.sticks.roll, &mut pct_buf);
    Text::new(p1, Point::new(92, 19), text_style).draw(lcd).ok();

    // CH2: Pitch / 2 (y = 21)
    let lbl2 = if is_general { "2" } else { "E" };
    Text::new(lbl2, Point::new(2, 27), text_style)
        .draw(lcd)
        .ok();
    widgets::draw_channel_gauge(lcd, 12, 21, 76, 7, state.sticks.pitch, trims.values.pitch);
    let p2 = format_percent(state.sticks.pitch, &mut pct_buf);
    Text::new(p2, Point::new(92, 27), text_style).draw(lcd).ok();

    // CH3: Throttle / 3 (y = 29)
    let lbl3 = if is_general { "3" } else { "T" };
    Text::new(lbl3, Point::new(2, 35), text_style)
        .draw(lcd)
        .ok();
    if is_general {
        widgets::draw_channel_gauge(
            lcd,
            12,
            29,
            76,
            7,
            state.sticks.throttle,
            trims.values.throttle,
        );
        let p3 = format_percent(state.sticks.throttle, &mut pct_buf);
        Text::new(p3, Point::new(92, 35), text_style).draw(lcd).ok();
    } else {
        let thr_trim = if storage.radio.throttle_trim != 0 {
            trims.values.throttle
        } else {
            0
        };
        widgets::draw_progress_bar(lcd, 12, 29, 76, 7, state.sticks.throttle, thr_trim);
        let p3 = format_throttle_percent(state.sticks.throttle, &mut pct_buf);
        Text::new(p3, Point::new(92, 35), text_style).draw(lcd).ok();
    }

    // CH4: Yaw / 4 (y = 37)
    let lbl4 = if is_general { "4" } else { "R" };
    Text::new(lbl4, Point::new(2, 43), text_style)
        .draw(lcd)
        .ok();
    widgets::draw_channel_gauge(lcd, 12, 37, 76, 7, state.sticks.yaw, trims.values.yaw);
    let p4 = format_percent(state.sticks.yaw, &mut pct_buf);
    Text::new(p4, Point::new(92, 43), text_style).draw(lcd).ok();

    // Switches and Pots Row (y = 46..53)
    // Check if configuration has any non-stock customizations or 3+ pots active
    let ext_sw = storage.radio.ext_switches != 0;
    let ext_adc = storage.radio.ext_adc != 0;
    let has_custom_modes = storage.radio.adc_modes.iter().any(|&m| m != 0);

    // If completely stock defaults, use standard optimized layout with 6-pos flight mode awareness
    if !has_custom_modes && !ext_adc {
        if ext_sw {
            draw_switch_slot(
                lcd,
                2,
                7,
                "A",
                state.switches.sa,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                13,
                18,
                "B",
                state.switches.sb,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                24,
                29,
                "C",
                state.switches.sc,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                35,
                40,
                "D",
                state.switches.sd,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                46,
                51,
                "E",
                state.switches.se,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                57,
                62,
                "F",
                state.switches.sf,
                false,
                0,
                text_style,
                text_style_small,
            );
        } else {
            draw_switch_slot(
                lcd,
                2,
                9,
                "A",
                state.switches.sa,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                20,
                27,
                "B",
                state.switches.sb,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                38,
                45,
                "C",
                state.switches.sc,
                false,
                0,
                text_style,
                text_style_small,
            );
            draw_switch_slot(
                lcd,
                56,
                63,
                "D",
                state.switches.sd,
                false,
                0,
                text_style,
                text_style_small,
            );
        }

        Text::new("VR", Point::new(74, 52), text_style_small)
            .draw(lcd)
            .ok();
        widgets::draw_split_pot_bar(lcd, 84, 46, 42, state.pots.vr1, state.pots.vr2);
    } else {
        // Adaptive Mode: dynamically collect all active switches and all active continuous pots
        // Channels: 0..3: SA..SD, 4..5: VRA..VRB, 6..9: VRC..VRF (P7)
        let ch_names = ["A", "B", "C", "D", "1", "2", "3", "4", "5", "6"];
        let mut sw_list: [(&str, crate::input::SwitchPos, bool, u8); 12] =
            [("", crate::input::SwitchPos::Up, false, 0); 12];
        let mut sw_count = 0usize;
        let mut pot_vals = [0i16; 10];
        let mut pot_count = 0usize;

        let max_channels = if ext_adc { 10 } else { 6 };
        for ch in 0..max_channels {
            let mode = crate::storage::AdcInputMode::resolve(ch, storage.radio.adc_modes[ch]);
            match mode {
                crate::storage::AdcInputMode::TwoPos | crate::storage::AdcInputMode::ThreePos => {
                    let pos = match ch {
                        0 => state.switches.sa,
                        1 => state.switches.sb,
                        2 => state.switches.sc,
                        3 => state.switches.sd,
                        _ => {
                            let p = state.aux_pots[ch];
                            if p < -333 {
                                crate::input::SwitchPos::Up
                            } else if p < 333 {
                                crate::input::SwitchPos::Mid
                            } else {
                                crate::input::SwitchPos::Down
                            }
                        }
                    };
                    sw_list[sw_count] = (ch_names[ch], pos, false, 0);
                    sw_count += 1;
                }
                crate::storage::AdcInputMode::SixPos => {
                    let raw_ch = match ch {
                        0 => state.raw[4],
                        1 => state.raw[5],
                        2 => state.raw[8],
                        3 => state.raw[9],
                        4 => state.raw[6],
                        5 => state.raw[7],
                        6 => state.raw[11],
                        7 => state.raw[12],
                        8 => state.raw[13],
                        _ => state.raw[14],
                    };
                    let mode_num = crate::input::decode_switch_6pos_num(raw_ch);
                    sw_list[sw_count] =
                        (ch_names[ch], crate::input::SwitchPos::Mid, true, mode_num);
                    sw_count += 1;
                }
                crate::storage::AdcInputMode::Pot => {
                    pot_vals[pot_count] = state.aux_pots[ch];
                    pot_count += 1;
                }
                crate::storage::AdcInputMode::Default => unreachable!(),
            }
        }

        // Add external switches SE / SF if enabled
        if ext_sw {
            if sw_count < 12 {
                sw_list[sw_count] = ("E", state.switches.se, false, 0);
                sw_count += 1;
            }
            if sw_count < 12 {
                sw_list[sw_count] = ("F", state.switches.sf, false, 0);
                sw_count += 1;
            }
        }

        // Dynamic width allocation across the 124px row (x = 2..126):
        // If pot_count == 0: Switches span the entire available width!
        // If pot_count > 0 && sw_count == 0: Pots span the entire available width!
        // If both are present: Allocate space proportionally with min pot width based on pot count.
        let (sw_region_w, pot_start_x, pot_w) = if pot_count == 0 {
            (124i32, 126i32, 0u32)
        } else if sw_count == 0 {
            (0i32, 2i32, 124u32)
        } else {
            // Needed width for pots: 1 pot => 28px, 2 pots => 36px, 3+ pots => 44px
            let min_pot_w = match pot_count {
                1 => 28i32,
                2 => 36i32,
                _ => 44i32,
            };
            // Max space switches can take while preserving pot room
            let max_sw_w = 124i32 - min_pot_w - 4; // 4px gap
            let desired_sw_w = (sw_count as i32 * 11).min(max_sw_w);
            let p_start = 2 + desired_sw_w + 4;
            let p_width = (126 - p_start).max(min_pot_w) as u32;
            (desired_sw_w, p_start, p_width)
        };

        // Render switches across sw_region_w
        if sw_count > 0 {
            let sw_spacing = if sw_count == 1 {
                sw_region_w
            } else {
                (sw_region_w / sw_count as i32).max(9)
            };

            for i in 0..sw_count {
                let x = 2 + (i as i32 * sw_spacing);
                let (name, pos, is_6pos, mode_num) = sw_list[i];
                draw_switch_slot(
                    lcd,
                    x,
                    x + 5,
                    name,
                    pos,
                    is_6pos,
                    mode_num,
                    text_style,
                    text_style_small,
                );
            }
        }

        // Render continuous pots on the right
        if pot_count > 0 && pot_w > 0 {
            if pot_count == 1 {
                widgets::draw_single_pot_bar(lcd, pot_start_x, 46, pot_w, pot_vals[0]);
            } else if pot_count == 2 {
                widgets::draw_split_pot_bar(lcd, pot_start_x, 46, pot_w, pot_vals[0], pot_vals[1]);
            } else {
                widgets::draw_multi_pot_bar(lcd, pot_start_x, 46, pot_w, &pot_vals[..pot_count]);
            }
        }
    }

    // Standardized Footer (y = 55..63)
    if is_binding {
        widgets::draw_footer(lcd, "[ESC] Finish Bind");
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
        widgets::draw_footer_split(lcd, trm_str, "Hold OK:Menu");
    } else if let Some(t_str) = timer_str {
        let invert_timer = timer_expired && blink_on;
        widgets::draw_footer_three(lcd, "P1/5", t_str, "Hold OK:Menu", invert_timer);
    } else {
        widgets::draw_footer_split(lcd, "P1/5", "Hold OK:Menu");
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn draw_switch_slot(
    lcd: &mut St7567,
    text_x: i32,
    glyph_x: i32,
    name: &str,
    pos: crate::input::SwitchPos,
    is_6pos: bool,
    mode_num: u8,
    _text_style: MonoTextStyle<'_, BinaryColor>,
    text_style_small: MonoTextStyle<'_, BinaryColor>,
) {
    Text::new(name, Point::new(text_x + 1, 53), text_style_small)
        .draw(lcd)
        .ok();
    if is_6pos {
        let num_char = [b'0' + mode_num.clamp(1, 6)];
        let num_str = crate::ui::format::ascii_as_str(&num_char);
        Text::new(num_str, Point::new(glyph_x + 1, 52), text_style_small)
            .draw(lcd)
            .ok();
    } else {
        draw_switch_arrow(lcd, glyph_x, 46, pos);
    }
}
