#![no_std]
#![no_main]

use panic_halt as _;
use stm32f0xx_hal as _;

use cortex_m_rt::entry;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};

mod boot;
mod chip;
mod display;

use display::St7567;

const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

fn u16_to_hex(val: u16, buf: &mut [u8; 4]) {
    buf[0] = HEX_CHARS[((val >> 12) & 0x0F) as usize];
    buf[1] = HEX_CHARS[((val >> 8) & 0x0F) as usize];
    buf[2] = HEX_CHARS[((val >> 4) & 0x0F) as usize];
    buf[3] = HEX_CHARS[(val & 0x0F) as usize];
}

#[entry]
fn main() -> ! {
    // 1. MCU Profile & Early DFU Bootloader Check
    let mcu_profile = chip::get_mcu_profile();
    boot::check_dfu_entry(&mcu_profile);

    // 2. Read Silicon Unique ID (UID)
    let uid = chip::read_uid(&mcu_profile);

    // 3. Initialize ST7567 128×64 LCD (Parallel 8-bit bus on GPIOE + control pins)
    let mut lcd = St7567::new();

    // 4. Render Initial Bring-Up Screen
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let border_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    // Outer framing box
    Rectangle::new(Point::new(0, 0), Size::new(128, 64))
        .into_styled(border_style)
        .draw(&mut lcd)
        .ok();

    // Header title
    Text::new("flysky-i6x-rs", Point::new(24, 11), text_style)
        .draw(&mut lcd)
        .ok();

    // Separator line
    Rectangle::new(Point::new(2, 14), Size::new(124, 1))
        .into_styled(border_style)
        .draw(&mut lcd)
        .ok();

    // MCU target info
    Text::new(mcu_profile.name, Point::new(6, 25), text_style)
        .draw(&mut lcd)
        .ok();

    // Convert first 4 bytes of UID to hex string
    let mut uid_buf = [b'0'; 8];
    for i in 0..4 {
        let b = uid[i];
        uid_buf[i * 2] = HEX_CHARS[(b >> 4) as usize];
        uid_buf[i * 2 + 1] = HEX_CHARS[(b & 0x0F) as usize];
    }
    let uid_str = core::str::from_utf8(&uid_buf).unwrap_or("UNKNOWN");

    Text::new("UID:", Point::new(6, 37), text_style)
        .draw(&mut lcd)
        .ok();
    Text::new(uid_str, Point::new(36, 37), text_style)
        .draw(&mut lcd)
        .ok();

    Text::new("Trims/Bind -> DFU", Point::new(6, 49), text_style)
        .draw(&mut lcd)
        .ok();

    lcd.flush();

    // 5. Main Loop: Poll keys, live diagnostic, and runtime DFU trigger
    let mut last_keys = 0xFFFFu16;
    let mut dfu_confirm_count = 0u8;

    loop {
        let keys = boot::scan_keys();

        // Check for DFU trigger at runtime (inward trims or bind key)
        if boot::is_dfu_requested(keys) {
            dfu_confirm_count += 1;
            if dfu_confirm_count >= 5 {
                // Clear and display DFU entry message
                lcd.clear(BinaryColor::Off).ok();
                Text::new("ENTERING DFU...", Point::new(18, 32), text_style)
                    .draw(&mut lcd)
                    .ok();
                lcd.flush();

                // Small delay so the message is visible before reset
                for _ in 0..500_000 {
                    cortex_m::asm::nop();
                }

                chip::enter_dfu_bootloader(&mcu_profile);
            }
        } else {
            dfu_confirm_count = 0;
        }

        // If key state changed, update live key status line on screen
        if keys != last_keys {
            last_keys = keys;

            // Clear bottom diagnostic line (y = 51..62, x = 6..122)
            Rectangle::new(Point::new(6, 51), Size::new(116, 11))
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
                .draw(&mut lcd)
                .ok();

            let mut hex_buf = [b'0'; 4];
            u16_to_hex(keys, &mut hex_buf);
            let hex_str = core::str::from_utf8(&hex_buf).unwrap_or("0000");

            Text::new("Raw:", Point::new(6, 60), text_style)
                .draw(&mut lcd)
                .ok();
            Text::new(hex_str, Point::new(36, 60), text_style)
                .draw(&mut lcd)
                .ok();

            if (keys & (1 << 12)) != 0 {
                Text::new("BIND", Point::new(80, 60), text_style)
                    .draw(&mut lcd)
                    .ok();
            }

            lcd.flush();
        }

        // Loop pacing (~20ms)
        for _ in 0..100_000 {
            cortex_m::asm::nop();
        }
    }
}
