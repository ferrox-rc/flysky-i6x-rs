//! Page 1 (P2/4): 14-Channel Dual-Column Live Monitor.

use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::display::St7567;
use crate::ui::format::u16_to_dec_4;
use crate::ui::widgets;

pub fn render(
    lcd: &mut St7567,
    rf_chs: &[u16; 14],
    is_binding: bool,
) {
    let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
    let sep_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    // Col 0 (CH 1..7) at x = 2..62, Col 1 (CH 8..14) at x = 66..126
    for col in 0..2 {
        let col_x = if col == 0 { 2 } else { 66 };
        let start_ch = col * 7;

        for row in 0..7 {
            let ch = start_ch + row;
            let y = 12 + (row as i32 * 6);

            // Label: " 1:" .. "14:"
            let mut lbl_buf = *b"  :";
            if ch + 1 >= 10 {
                lbl_buf[0] = b'1';
                lbl_buf[1] = b'0' + ((ch + 1) % 10) as u8;
            } else {
                lbl_buf[0] = b' ';
                lbl_buf[1] = b'1' + ch as u8;
            }
            let lbl_str = core::str::from_utf8(&lbl_buf).unwrap_or("??:");
            Text::new(lbl_str, Point::new(col_x, y + 5), text_style_small).draw(lcd).ok();

            // Bar gauge (width 22, height 5) using shared widget
            let us = rf_chs[ch].clamp(1000, 2000);
            let fill_w = (((us - 1000) as u32 * 20) / 1000).min(20);
            widgets::draw_bar_gauge(
                lcd,
                Rectangle::new(Point::new(col_x + 13, y + 1), Size::new(22, 5)),
                fill_w,
            );

            // Value: "1500"
            let mut val_buf = [0u8; 4];
            u16_to_dec_4(us, &mut val_buf);
            let val_str = core::str::from_utf8(&val_buf).unwrap_or("1500");
            Text::new(val_str, Point::new(col_x + 37, y + 5), text_style_small).draw(lcd).ok();
        }
    }

    // Vertical divider line between columns
    Line::new(Point::new(64, 12), Point::new(64, 53))
        .into_styled(sep_style)
        .draw(lcd)
        .ok();

    // Standardized Footer (y = 55..63)
    if is_binding {
        widgets::draw_footer(lcd, "[ESC] Finish Bind");
    } else {
        widgets::draw_footer_split(lcd, "P2/4", "14-CH MONITOR");
    }
}
