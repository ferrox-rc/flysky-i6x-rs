//! Reusable UI widgets and layout helpers for FlySky FS-i6X menu displays.

use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::buzzer::Buzzer;
use crate::display::St7567;

/// Draw a standardized top header banner with title and underline divider.
pub fn draw_header(lcd: &mut St7567, title: &str) {
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let x = ((128i32 - title.len() as i32 * 6) / 2).max(2);
    Text::new(title, Point::new(x, 9), text_style).draw(lcd).ok();
    Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(border_style).draw(lcd).ok();
}

/// Draw a standardized bottom footer with normal font (FONT_6X10) and divider line at y = 52.
pub fn draw_footer(lcd: &mut St7567, text: &str) {
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
    Text::new(text, Point::new(2, 62), text_style).draw(lcd).ok();
}

/// Draw a standardized bottom footer with small font (FONT_4X6) and divider line at y = 52.
pub fn draw_footer_small(lcd: &mut St7567, text: &str) {
    let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
    let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    Line::new(Point::new(0, 52), Point::new(127, 52)).into_styled(border_style).draw(lcd).ok();
    Text::new(text, Point::new(2, 62), text_style_small).draw(lcd).ok();
}

/// Handle 4-slot circular scrolling list navigation with tone feedback.
pub fn navigate_4slot_list(
    selected: &mut usize,
    scroll_offset: &mut usize,
    count: usize,
    up_pressed: bool,
    down_pressed: bool,
    buzzer: &mut Buzzer,
) {
    if count == 0 {
        return;
    }
    if down_pressed {
        if *selected + 1 < count {
            *selected += 1;
            if *selected >= *scroll_offset + 4 {
                *scroll_offset = *selected - 3;
            }
        } else {
            *selected = 0;
            *scroll_offset = 0;
        }
        buzzer.play_tone(2200, 30);
    }
    if up_pressed {
        if *selected > 0 {
            *selected -= 1;
            if *selected < *scroll_offset {
                *scroll_offset = *selected;
            }
        } else {
            *selected = count - 1;
            *scroll_offset = count.saturating_sub(4);
        }
        buzzer.play_tone(2200, 30);
    }
}

/// Render a single list item row within a 4-slot view with highlight.
pub fn draw_list_row(
    lcd: &mut St7567,
    slot: usize,
    is_selected: bool,
    label: &str,
    value: Option<&str>,
    value_x: i32,
) {
    let y = 14 + (slot as i32 * 9);
    let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);
    let style = if is_selected {
        Rectangle::new(Point::new(2, y), Size::new(124, 9))
            .into_styled(fill_style)
            .draw(lcd)
            .ok();
        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off)
    } else {
        MonoTextStyle::new(&FONT_6X10, BinaryColor::On)
    };

    Text::new(label, Point::new(4, y + 7), style).draw(lcd).ok();
    if let Some(val) = value {
        Text::new(val, Point::new(value_x, y + 7), style).draw(lcd).ok();
    }
}

/// Draw a framed bar gauge meter with an inner filled level.
pub fn draw_bar_gauge(
    lcd: &mut St7567,
    box_rect: Rectangle,
    fill_width: u32,
) {
    let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);
    box_rect.into_styled(border_style).draw(lcd).ok();
    if fill_width > 0 {
        let inner_p = Point::new(box_rect.top_left.x + 1, box_rect.top_left.y + 1);
        let inner_h = box_rect.size.height.saturating_sub(2);
        Rectangle::new(inner_p, Size::new(fill_width, inner_h))
            .into_styled(fill_style)
            .draw(lcd)
            .ok();
    }
}
