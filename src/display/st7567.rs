//! ST7567 128×64 Monochrome LCD 8-bit Parallel Driver with Embedded-Graphics Support.

use core::convert::Infallible;
use core::ptr;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::BinaryColor,
    Pixel,
};

// Base register addresses for STM32F072
const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;

const GPIOB_MODER: *mut u32 = 0x4800_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4800_0418 as *mut u32;

const GPIOC_MODER: *mut u32 = 0x4800_0800 as *mut u32;
const GPIOC_BSRR: *mut u32 = 0x4800_0818 as *mut u32;

const GPIOD_MODER: *mut u32 = 0x4800_0C00 as *mut u32;
const GPIOD_BSRR: *mut u32 = 0x4800_0C18 as *mut u32;

const GPIOE_MODER: *mut u32 = 0x4800_1000 as *mut u32;
const GPIOE_ODR: *mut u32 = 0x4800_1014 as *mut u32;

const GPIOF_MODER: *mut u32 = 0x4800_1400 as *mut u32;
const GPIOF_BSRR: *mut u32 = 0x4800_1418 as *mut u32;

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 64;
pub const BUFFER_SIZE: usize = WIDTH * (HEIGHT / 8); // 1024 bytes

/// ST7567 LCD Driver with 1024-byte internal framebuffer.
pub struct St7567 {
    framebuffer: [u8; BUFFER_SIZE],
}

impl St7567 {
    /// Initialize GPIO pins and the ST7567 display controller.
    pub fn new() -> Self {
        let display = Self {
            framebuffer: [0u8; BUFFER_SIZE],
        };

        display.init_hardware();
        display.init_controller();
        display.set_backlight(true);

        display
    }

    /// Configure MCU GPIO ports for parallel LCD interface and backlight.
    fn init_hardware(&self) {
        unsafe {
            // Enable GPIOB, GPIOC, GPIOD, GPIOE, GPIOF in RCC_AHBENR
            // Bit 18 = GPIOB, 19 = GPIOC, 20 = GPIOD, 21 = GPIOE, 22 = GPIOF
            let ahbenr = ptr::read_volatile(RCC_AHBENR);
            ptr::write_volatile(
                RCC_AHBENR,
                ahbenr | (1 << 18) | (1 << 19) | (1 << 20) | (1 << 21) | (1 << 22),
            );

            // Configure PE0..PE7 as outputs (MODER = 01)
            let e_moder = ptr::read_volatile(GPIOE_MODER);
            ptr::write_volatile(GPIOE_MODER, (e_moder & !0x0000_FFFF) | 0x0000_5555);

            // Configure PB3 (RS), PB4 (RST), PB5 (RW) as outputs (MODER = 01)
            let b_moder = ptr::read_volatile(GPIOB_MODER);
            let b_mask = (3 << 6) | (3 << 8) | (3 << 10);
            let b_val = (1 << 6) | (1 << 8) | (1 << 10);
            ptr::write_volatile(GPIOB_MODER, (b_moder & !b_mask) | b_val);

            // Configure PD2 (CS), PD7 (RD / E-strobe) as outputs (MODER = 01)
            let d_moder = ptr::read_volatile(GPIOD_MODER);
            let d_mask = (3 << 4) | (3 << 14);
            let d_val = (1 << 4) | (1 << 14);
            ptr::write_volatile(GPIOD_MODER, (d_moder & !d_mask) | d_val);

            // Standard Backlight is on PF3 (Active HIGH). Configure PF3 as output.
            let f_moder = ptr::read_volatile(GPIOF_MODER);
            ptr::write_volatile(GPIOF_MODER, (f_moder & !(3 << 6)) | (1 << 6));

            // Modded Backlight pads on PC9 and PB1. Configure as outputs too.
            let c_moder = ptr::read_volatile(GPIOC_MODER);
            ptr::write_volatile(GPIOC_MODER, (c_moder & !(3 << 18)) | (1 << 18));
            let b_moder = ptr::read_volatile(GPIOB_MODER);
            ptr::write_volatile(GPIOB_MODER, (b_moder & !(3 << 2)) | (1 << 2));

            // Default LCD states matching OpenI6X:
            // CS (PD2) = Low (chip enabled)
            // RW (PB5) = Low (write mode enabled)
            // RD (PD7) = High (strobe idle high)
            // RST (PB4) = High
            // RS (PB3) = High
            ptr::write_volatile(GPIOD_BSRR, (1 << (2 + 16)) | (1 << 7)); // PD2=0 (CS Low), PD7=1 (RD High)
            ptr::write_volatile(GPIOB_BSRR, (1 << (5 + 16)) | (1 << 4) | (1 << 3)); // PB5=0 (RW Low), PB4=1 (RST High), PB3=1 (RS High)
        }
    }

    /// Set Backlight state (Standard PF3 active HIGH, and PC9/PB1 active HIGH).
    pub fn set_backlight(&self, on: bool) {
        unsafe {
            if on {
                // Stock FS-i6X factory backlight: PF3 HIGH turns it ON (OpenTX GPIO_SetBits)
                ptr::write_volatile(GPIOF_BSRR, 1 << 3);
                // Also drive PC9 and PB1 HIGH for any modded backlights
                ptr::write_volatile(GPIOC_BSRR, 1 << 9);
                ptr::write_volatile(GPIOB_BSRR, 1 << 1);
            } else {
                ptr::write_volatile(GPIOF_BSRR, 1 << (3 + 16));
                ptr::write_volatile(GPIOC_BSRR, 1 << (9 + 16));
                ptr::write_volatile(GPIOB_BSRR, 1 << (1 + 16));
            }
        }
    }

    #[inline(always)]
    fn delay_cycles(&self, count: u32) {
        for _ in 0..count {
            cortex_m::asm::nop();
        }
    }

    /// Send byte to ST7567 using 6800-series parallel strobe:
    /// Put byte on PE0..PE7, then pulse RD (PD7) High -> Low.
    #[inline(always)]
    fn write_byte(&self, byte: u8, is_data: bool) {
        unsafe {
            // Set RS: PB3 (1 for Data, 0 for Command)
            if is_data {
                ptr::write_volatile(GPIOB_BSRR, 1 << 3);
            } else {
                ptr::write_volatile(GPIOB_BSRR, 1 << (3 + 16));
            }

            // Put byte on PE0..PE7 (low 8 bits of GPIOE_ODR)
            let curr_e = ptr::read_volatile(GPIOE_ODR);
            ptr::write_volatile(GPIOE_ODR, (curr_e & !0xFF) | (byte as u32));

            // Strobe RD (PD7): High then Low latches data (OpenI6X timing)
            ptr::write_volatile(GPIOD_BSRR, 1 << 7); // RD High
            cortex_m::asm::nop();
            ptr::write_volatile(GPIOD_BSRR, 1 << (7 + 16)); // RD Low
        }
    }

    #[inline(always)]
    pub fn write_cmd(&self, cmd: u8) {
        self.write_byte(cmd, false);
    }

    #[inline(always)]
    pub fn write_data(&self, data: u8) {
        self.write_byte(data, true);
    }

    /// Hardware reset and send OpenI6X-matched ST7567 initialization sequence.
    fn init_controller(&self) {
        unsafe {
            // Hardware reset: PB4 Low for ~50us, then High (OpenI6X delay_us(20))
            ptr::write_volatile(GPIOB_BSRR, 1 << (4 + 16)); // RST Low
            self.delay_cycles(500);
            ptr::write_volatile(GPIOB_BSRR, 1 << 4);         // RST High
            self.delay_cycles(500);
        }

        // Exact OpenI6X initialization sequence:
        self.write_cmd(0xE2); // LCD_CMD_RESET
        self.delay_cycles(10_000);

        self.write_cmd(0xAE); // LCD_CMD_DISPLAY_OFF
        self.write_cmd(0xA4); // LCD_CMD_MODE_RAM
        self.write_cmd(0xA3); // LCD_CMD_BIAS_1_7 (1/7 bias)
        self.write_cmd(0xC0); // LCD_CMD_COM_NORMAL (COM0 -> COM63)
        self.write_cmd(0xA1); // LCD_CMD_SEG_INVERSE (SEG131 -> SEG0) - Fixes upside-down!
        self.write_cmd(0x2F); // LCD_CMD_POWERCTRL_ALL_ON
        self.write_cmd(0x23); // LCD_CMD_REG_RATIO_011
        self.write_cmd(0x81); // LCD_CMD_EV (Electronic Volume)
        self.write_cmd(0x25); // Default contrast
        self.write_cmd(0x40); // LCD_CMD_SET_STARTLINE (0)
        self.write_cmd(0xB0); // LCD_CMD_SET_PAGESTART (0)
        self.write_cmd(0x04); // LCD_CMD_SET_COL_LO (start at line 4: 132 lines controller vs 128 LCD)
        self.write_cmd(0x10); // LCD_CMD_SET_COL_HI (0)
        self.write_cmd(0xAF); // LCD_CMD_DISPLAY_ON
        self.delay_cycles(10_000);
    }

    /// Flush the entire 1024-byte framebuffer to the ST7567 LCD.
    /// Notice: Controller is 132 columns wide while the panel is 128, starting at column 4!
    pub fn flush(&self) {
        for page in 0..8 {
            self.write_cmd(0xB0 | page as u8); // Set page (0..7)
            self.write_cmd(0x04);             // Col low nibble = 4
            self.write_cmd(0x10);             // Col high nibble = 0

            let start = page * WIDTH;
            let end = start + WIDTH;
            for &byte in &self.framebuffer[start..end] {
                self.write_data(byte);
            }
        }
    }

    /// Clear the framebuffer (all pixels off).
    #[allow(dead_code)]
    pub fn clear_buffer(&mut self) {
        self.framebuffer.fill(0);
    }
}

impl OriginDimensions for St7567 {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for St7567 {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0 && coord.x < WIDTH as i32 && coord.y >= 0 && coord.y < HEIGHT as i32 {
                let x = coord.x as usize;
                let y = coord.y as usize;

                let page = y / 8;
                let bit = y % 8;
                let index = page * WIDTH + x;

                match color {
                    BinaryColor::On => {
                        self.framebuffer[index] |= 1 << bit;
                    }
                    BinaryColor::Off => {
                        self.framebuffer[index] &= !(1 << bit);
                    }
                }
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        let fill = match color {
            BinaryColor::On => 0xFF,
            BinaryColor::Off => 0x00,
        };
        self.framebuffer.fill(fill);
        Ok(())
    }
}
