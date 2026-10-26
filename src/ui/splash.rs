//! Startup splash screen renderer with Ferrox-RC logo and firmware information.

use crate::display::St7567;
use crate::ui::glyphs::draw_ferrox_logo;

/// Render the Ferrox-RC splash screen onto the display buffer.
/// - Top: 28x27 Ferrox-RC kinetic delta emblem centered at x = 50, y = 3
/// - Brand Header: "FERROX-RC" in 6x10 centered at x = 37, y = 34
/// - Firmware ID: "flysky-i6x-rs" in 4x6 centered at x = 38, y = 45
/// - Version / Build: version and commit hash centered at y = 54
pub fn draw_splash(lcd: &mut St7567) {
    lcd.clear_buffer();

    draw_ferrox_logo(lcd, 50, 3);
    lcd.draw_str_6x10(37, 34, "FERROX-RC", false);
    lcd.draw_str_4x6(38, 45, crate::hw::FIRMWARE_ID, false);

    // Center version tag horizontally
    const VERSION_TAG: &str = concat!(env!("FIRMWARE_VERSION"), " (", env!("GIT_HASH"), ")");
    const VERSION_X: i32 = ((128 - (VERSION_TAG.len() * 4)) / 2) as i32;
    lcd.draw_str_4x6(VERSION_X, 54, VERSION_TAG, false);
}
