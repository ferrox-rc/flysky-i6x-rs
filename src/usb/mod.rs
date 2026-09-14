//! USB Subsystem for FlySky FS-i6X.
//!
//! Orchestrates the STM32F072 USB Full-Speed (12 Mbps) peripheral,
//! HID Gamepad class (for flight simulators), and CDC-ACM Virtual COM Port (telemetry/CLI).

#![allow(static_mut_refs)]

pub mod hid;
pub mod serial;

use crate::input::Switches;
use crate::rf::afhds2a::TelemetryData;
use serial::SerialHandler;
use stm32_usbd::{MemoryAccess, UsbBus, UsbPeripheral};
use usb_device::{
    bus::UsbBusAllocator,
    device::{StringDescriptors, UsbDevice, UsbDeviceBuilder, UsbDeviceState, UsbVidPid},
};
use usbd_hid::hid_class::HIDClass;
use usbd_serial::SerialPort;

/// Hardware USB peripheral mapping for STM32F072 on FlySky FS-i6X.
pub struct FlyskyUsb;

unsafe impl UsbPeripheral for FlyskyUsb {
    const REGISTERS: *const () = 0x4000_5C00 as *const ();
    const DP_PULL_UP_FEATURE: bool = true;
    const EP_MEMORY: *const () = 0x4000_6000 as *const ();
    const EP_MEMORY_SIZE: usize = 1024;
    const EP_MEMORY_ACCESS: MemoryAccess = MemoryAccess::Word16x2;

    fn enable() {
        unsafe {
            // Enable USB peripheral clock in RCC_APB1ENR bit 23
            let rcc_apb1enr = 0x4002_101C as *mut u32;
            core::ptr::write_volatile(rcc_apb1enr, core::ptr::read_volatile(rcc_apb1enr) | (1 << 23));
        }
    }

    fn startup_delay() {
        // 1 ms delay at 48 MHz
        cortex_m::asm::delay(48_000);
    }
}

pub type FlyskyUsbBus = UsbBus<FlyskyUsb>;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UsbMode {
    Off = 0,       // USB disconnected / charging only (Default)
    Joystick = 1,  // Native USB Gamepad (RF disabled for simulator play)
    Serial = 2,    // Virtual COM Port (RF active with live telemetry streaming)
    Composite = 3, // Both Joystick and Serial active simultaneously
}

impl UsbMode {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Joystick,
            2 => Self::Serial,
            3 => Self::Composite,
            _ => Self::Off,
        }
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Joystick => "JOYSTICK",
            Self::Serial => "SERIAL",
            Self::Composite => "COMPOSITE",
        }
    }
}

static mut USB_ALLOCATOR: Option<UsbBusAllocator<FlyskyUsbBus>> = None;
static mut USB_DEV: Option<UsbDevice<'static, FlyskyUsbBus>> = None;
static mut USB_HID: Option<HIDClass<'static, FlyskyUsbBus>> = None;
static mut USB_SERIAL: Option<SerialPort<'static, FlyskyUsbBus>> = None;
static mut SERIAL_HANDLER: SerialHandler = SerialHandler::new();
static mut CURRENT_MODE: UsbMode = UsbMode::Off;
static mut LAST_POLL_MS: u32 = 0;
static mut LAST_TELEM_STREAM_MS: u32 = 0;

/// Initialize the hardware USB peripheral according to the configured USB mode.
/// Supports on-the-fly switching between modes by forcing physical disconnect and core reset.
pub fn init(mode: u8) {
    let usb_mode = UsbMode::from_u8(mode);
    unsafe {
        CURRENT_MODE = usb_mode;

        // 1. Force physical disconnect on host PC by disabling DPPU and driving PA12 (D+) LOW
        let bcdr = 0x4000_5C58 as *mut u32;
        core::ptr::write_volatile(bcdr, 0); // Disable DPPU

        let gpioa_moder = 0x4800_0000 as *mut u32;
        let gpioa_bsrr = 0x4800_0018 as *mut u32;
        let moder = core::ptr::read_volatile(gpioa_moder);
        // Set PA12 to output (0b01) and drive LOW to assert Single-Ended Zero (SE0) disconnect
        core::ptr::write_volatile(gpioa_moder, (moder & !(0b11 << 24)) | (0b01 << 24));
        core::ptr::write_volatile(gpioa_bsrr, 1 << (12 + 16));
        cortex_m::asm::delay(1_000_000); // ~20 ms delay to guarantee host root hub detects disconnect

        // 2. Hardware peripheral reset via RCC APB1RSTR (bit 23 = USBRST)
        let rcc_apb1rstr = 0x4002_1010 as *mut u32;
        core::ptr::write_volatile(rcc_apb1rstr, core::ptr::read_volatile(rcc_apb1rstr) | (1 << 23));
        cortex_m::asm::delay(48_000);
        core::ptr::write_volatile(rcc_apb1rstr, core::ptr::read_volatile(rcc_apb1rstr) & !(1 << 23));

        // 3. Drop existing USB stack instances
        USB_DEV = None;
        USB_HID = None;
        USB_SERIAL = None;
        USB_ALLOCATOR = None;

        let rcc_apb1enr = 0x4002_101C as *mut u32;
        if usb_mode == UsbMode::Off {
            // Disable USB peripheral clock
            core::ptr::write_volatile(rcc_apb1enr, core::ptr::read_volatile(rcc_apb1enr) & !(1 << 23));
            // Set PA11 and PA12 as analog inputs to float pins and conserve battery
            let moder = core::ptr::read_volatile(gpioa_moder);
            core::ptr::write_volatile(gpioa_moder, moder | (0b11 << 22) | (0b11 << 24));
            return;
        }

        // Enable USB peripheral clock
        core::ptr::write_volatile(rcc_apb1enr, core::ptr::read_volatile(rcc_apb1enr) | (1 << 23));

        // Configure PA11 & PA12 as AF0 (USB_DM & USB_DP)
        let gpioa_afrh = 0x4800_0024 as *mut u32;
        let moder = core::ptr::read_volatile(gpioa_moder);
        core::ptr::write_volatile(
            gpioa_moder,
            (moder & !((0b11 << 22) | (0b11 << 24))) | ((0b10 << 22) | (0b10 << 24)),
        );
        let afrh = core::ptr::read_volatile(gpioa_afrh);
        core::ptr::write_volatile(gpioa_afrh, afrh & !((0xF << 12) | (0xF << 16)));

        // Stabilization delay
        cortex_m::asm::delay(48_000);

        // 4. Setup UsbBus allocator
        let bus = FlyskyUsbBus::new(FlyskyUsb);
        USB_ALLOCATOR = Some(bus);
        let alloc = USB_ALLOCATOR.as_ref().unwrap();

        // 5. Initialize classes based on mode
        match usb_mode {
            UsbMode::Joystick => {
                let hid = HIDClass::new_ep_in(alloc, hid::GAMEPAD_REPORT_DESC, 10);
                USB_HID = Some(hid);

                let dev = UsbDeviceBuilder::new(alloc, UsbVidPid(0x1209, 0x4F54)) // OpenTX / EdgeTX Radio Joystick
                    .device_class(0x00)
                    .strings(&[StringDescriptors::default()
                        .manufacturer("FlySky")
                        .product("FS-i6X Joystick")
                        .serial_number("FS-I6X-SIM")])
                    .unwrap_or_else(|_| panic_fallback())
                    .build();
                USB_DEV = Some(dev);
            }
            UsbMode::Serial => {
                let serial = SerialPort::new(alloc);
                USB_SERIAL = Some(serial);

                let dev = UsbDeviceBuilder::new(alloc, UsbVidPid(0x0483, 0x5740)) // Standard STM32 VCP
                    .device_class(usbd_serial::USB_CLASS_CDC)
                    .strings(&[StringDescriptors::default()
                        .manufacturer("FlySky")
                        .product("FS-i6X Serial")
                        .serial_number("FS-I6X-VCP")])
                    .unwrap_or_else(|_| panic_fallback())
                    .build();
                USB_DEV = Some(dev);
            }
            UsbMode::Composite => {
                let hid = HIDClass::new_ep_in(alloc, hid::GAMEPAD_REPORT_DESC, 10);
                let serial = SerialPort::new(alloc);
                USB_HID = Some(hid);
                USB_SERIAL = Some(serial);

                let dev = UsbDeviceBuilder::new(alloc, UsbVidPid(0x1209, 0x4968)) // EdgeTX Radio Composite
                    .device_class(0xEF) // Miscellaneous device (IAD)
                    .device_sub_class(0x02)
                    .device_protocol(0x01)
                    .strings(&[StringDescriptors::default()
                        .manufacturer("FlySky")
                        .product("FS-i6X Radio")
                        .serial_number("FS-I6X-COMP")])
                    .unwrap_or_else(|_| panic_fallback())
                    .build();
                USB_DEV = Some(dev);
            }
            UsbMode::Off => {}
        }
    }
}

fn panic_fallback() -> UsbDeviceBuilder<'static, FlyskyUsbBus> {
    unsafe {
        let alloc = USB_ALLOCATOR.as_ref().unwrap();
        UsbDeviceBuilder::new(alloc, UsbVidPid(0x1209, 0x4F54))
    }
}

/// Periodic USB task (called from the main loop).
/// Dispatches HID reports at ~100 Hz (10 ms) and services serial CLI & telemetry.
pub fn poll(
    now_ms: u32,
    rf_chs: &[u16; 14],
    switches: &Switches,
    telem: &TelemetryData,
    battery_mv: u16,
) {
    unsafe {
        let mode = CURRENT_MODE;
        if mode == UsbMode::Off {
            return;
        }

        let dev = match USB_DEV.as_mut() {
            Some(d) => d,
            None => return,
        };

        // Poll USB bus events
        match mode {
            UsbMode::Joystick => {
                if let Some(hid) = USB_HID.as_mut() {
                    dev.poll(&mut [hid]);
                }
            }
            UsbMode::Serial => {
                if let Some(serial) = USB_SERIAL.as_mut() {
                    dev.poll(&mut [serial]);
                    SERIAL_HANDLER.update(serial, rf_chs, telem, battery_mv);
                }
            }
            UsbMode::Composite => {
                if let (Some(hid), Some(serial)) = (USB_HID.as_mut(), USB_SERIAL.as_mut()) {
                    dev.poll(&mut [hid, serial]);
                    SERIAL_HANDLER.update(serial, rf_chs, telem, battery_mv);
                }
            }
            UsbMode::Off => return,
        }

        // Only send reports when USB is configured and active
        if dev.state() != UsbDeviceState::Configured {
            return;
        }

        // Throttle Gamepad HID reports to ~100 Hz (10 ms)
        if (mode == UsbMode::Joystick || mode == UsbMode::Composite)
            && now_ms.wrapping_sub(LAST_POLL_MS) >= 10
        {
            LAST_POLL_MS = now_ms;
            if let Some(ref mut hid) = USB_HID.as_mut() {
                let mut report = [0u8; hid::REPORT_SIZE];
                hid::build_gamepad_report(rf_chs, switches, &mut report);
                let _ = hid.push_raw_input(&report);
            }
        }

        // Periodic telemetry streaming over Serial in Serial/Composite modes (at 20 Hz / 50 ms)
        if (mode == UsbMode::Serial || mode == UsbMode::Composite)
            && now_ms.wrapping_sub(LAST_TELEM_STREAM_MS) >= 50
        {
            LAST_TELEM_STREAM_MS = now_ms;
            if let Some(ref mut serial) = USB_SERIAL.as_mut() {
                SERIAL_HANDLER.send_telemetry_line(serial, rf_chs, telem, battery_mv);
            }
        }
    }
}

/// Check if the USB device is currently enumerated and configured by the host PC.
pub fn is_connected() -> bool {
    unsafe {
        USB_DEV
            .as_ref()
            .map(|d| d.state() == UsbDeviceState::Configured)
            .unwrap_or(false)
    }
}

/// Check if the radio is actively in Joystick / Simulator mode and connected.
/// When true, the RF transceiver is placed in Standby (zero RF emission / silent running).
pub fn is_sim_mode() -> bool {
    unsafe {
        CURRENT_MODE == UsbMode::Joystick && is_connected()
    }
}

/// Get currently active USB mode.
#[allow(dead_code)]
pub fn get_mode() -> UsbMode {
    unsafe { CURRENT_MODE }
}
