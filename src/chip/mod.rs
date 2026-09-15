//! Chip identification, Silicon Unique ID (UID) extraction, and DFU bootloader jump.

use core::ptr;

/// Supported MCU types.
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum McuType {
    Stm32f072,
    Apm32f072,
}

/// MCU-specific memory map profile.
#[allow(dead_code)]
pub struct McuProfile {
    pub mcu_type: McuType,
    pub name: &'static str,
    pub uid_addr: usize,
    pub dfu_bootloader_addr: usize,
}

/// STM32F072 memory layout definitions.
pub const STM32_PROFILE: McuProfile = McuProfile {
    mcu_type: McuType::Stm32f072,
    name: "STM32F072VB",
    uid_addr: 0x1FFF_F7AC,
    dfu_bootloader_addr: 0x1FFF_C800,
};

/// APM32F072 (Geehy) memory layout definitions.
#[allow(dead_code)]
pub const APM32_PROFILE: McuProfile = McuProfile {
    mcu_type: McuType::Apm32f072,
    name: "APM32F072VB",
    uid_addr: 0x1FFF_F7E8,
    dfu_bootloader_addr: 0x1FFF_F000,
};

/// Detect the active MCU profile.
pub fn get_mcu_profile() -> McuProfile {
    #[cfg(feature = "apm32")]
    {
        APM32_PROFILE
    }
    #[cfg(not(feature = "apm32"))]
    {
        STM32_PROFILE
    }
}

/// Read the 96-bit (12-byte) Unique Device ID from MCU silicon.
#[allow(dead_code)]
pub fn read_uid(profile: &McuProfile) -> [u8; 12] {
    let mut uid = [0u8; 12];
    unsafe {
        let src = profile.uid_addr as *const u8;
        for (i, byte) in uid.iter_mut().enumerate() {
            *byte = ptr::read_volatile(src.add(i));
        }
    }
    uid
}

/// Initialize system clock to 48 MHz using external 8 MHz crystal (HSE) + PLL.
/// Matches OpenI6X SetSysClock configuration.
pub fn init_system_clock() {
    const RCC_CR: *mut u32 = 0x4002_1000 as *mut u32;
    const RCC_CFGR: *mut u32 = 0x4002_1004 as *mut u32;
    const FLASH_ACR: *mut u32 = 0x4002_2000 as *mut u32;

    unsafe {
        // 1. Enable HSE (8 MHz external crystal on FlySky FS-i6X PCB)
        let cr = ptr::read_volatile(RCC_CR);
        ptr::write_volatile(RCC_CR, cr | (1 << 16)); // HSEON

        // 2. Wait for HSERDY with timeout
        let mut timeout = 100_000u32;
        while (ptr::read_volatile(RCC_CR) & (1 << 17)) == 0 && timeout > 0 {
            timeout -= 1;
        }

        if timeout > 0 {
            // 3. Set Flash latency to 1 wait state (required for 48 MHz) & enable prefetch buffer
            ptr::write_volatile(FLASH_ACR, (1 << 4) | (1 << 0)); // PRFTBE | LATENCY

            // 4. Configure PLL: HSE / 1 * 6 = 48 MHz
            // PLLSRC = HSE PREDIV (bit 16)
            // PLLMUL = 6 (0b0100 << 18 = 0x0010_0000)
            let cfgr = ptr::read_volatile(RCC_CFGR);
            let cfgr_clean = cfgr & !(0x003F_0000 | 0x0000_0003);
            ptr::write_volatile(RCC_CFGR, cfgr_clean | (1 << 16) | 0x0010_0000);

            // 5. Enable PLL (PLLON = bit 24)
            let cr = ptr::read_volatile(RCC_CR);
            ptr::write_volatile(RCC_CR, cr | (1 << 24));

            // 6. Wait for PLLRDY (bit 25)
            timeout = 100_000;
            while (ptr::read_volatile(RCC_CR) & (1 << 25)) == 0 && timeout > 0 {
                timeout -= 1;
            }

            // 7. Select PLL as system clock source (SW = 0b10)
            let cfgr = ptr::read_volatile(RCC_CFGR);
            ptr::write_volatile(RCC_CFGR, (cfgr & !0x03) | 0x02);

            // 8. Wait for PLL to be used as system clock source (SWS = 0b1000)
            timeout = 100_000;
            while (ptr::read_volatile(RCC_CFGR) & 0x0C) != 0x08 && timeout > 0 {
                timeout -= 1;
            }

            // 9. Select PLL (48 MHz) as USB clock source (USBSW = 1 in RCC_CFGR3 bit 7)
            const RCC_CFGR3: *mut u32 = 0x4002_1030 as *mut u32;
            let cfgr3 = ptr::read_volatile(RCC_CFGR3);
            ptr::write_volatile(RCC_CFGR3, cfgr3 | (1 << 7));
        }
    }
}

/// Reset RCC to default power-on state (HSI 8MHz, no PLL, peripheral clocks disabled).
unsafe fn rcc_deinit() {
    const RCC_CR: *mut u32 = 0x4002_1000 as *mut u32;
    const RCC_CFGR: *mut u32 = 0x4002_1004 as *mut u32;
    const RCC_CIR: *mut u32 = 0x4002_1008 as *mut u32;
    const RCC_APB2RSTR: *mut u32 = 0x4002_100C as *mut u32;
    const RCC_APB1RSTR: *mut u32 = 0x4002_1010 as *mut u32;
    const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
    const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;
    const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;

    // Turn on HSI
    let cr = ptr::read_volatile(RCC_CR);
    ptr::write_volatile(RCC_CR, cr | 0x0000_0001); // HSION
    while (ptr::read_volatile(RCC_CR) & 0x0000_0002) == 0 {
        // Wait for HSIRDY
    }

    // Reset CFGR
    ptr::write_volatile(RCC_CFGR, 0x0000_0000);

    // Reset HSEON, CSSON, PLLON
    let cr = ptr::read_volatile(RCC_CR);
    ptr::write_volatile(RCC_CR, cr & !(0x0001_0000 | 0x0008_0000 | 0x0100_0000));

    // Reset CIR interrupts
    ptr::write_volatile(RCC_CIR, 0x0000_0000);

    // Reset APB peripherals
    ptr::write_volatile(RCC_APB2RSTR, 0xFFFF_FFFF);
    ptr::write_volatile(RCC_APB2RSTR, 0x0000_0000);
    ptr::write_volatile(RCC_APB1RSTR, 0xFFFF_FFFF);
    ptr::write_volatile(RCC_APB1RSTR, 0x0000_0000);

    // Reset peripheral clocks except SYSCFG & SRAM
    ptr::write_volatile(RCC_AHBENR, 0x0000_0014); // SRAMEN + FLITFEN
    ptr::write_volatile(RCC_APB2ENR, 0x0000_0001); // SYSCFGEN
    ptr::write_volatile(RCC_APB1ENR, 0x0000_0000);
}

/// Perform a clean software jump to the MCU's built-in factory DFU bootloader.
///
/// Matches the exact OpenI6X SystemBootloaderJump() sequence:
/// 1. Reset RCC to default state
/// 2. Clear SysTick timer
/// 3. Remap System Memory to 0x0000_0000 via SYSCFG
/// 4. Re-enable interrupts so the factory bootloader's USB stack functions!
/// 5. Bootstrap into the bootloader reset vector.
pub fn enter_dfu_bootloader(profile: &McuProfile) -> ! {
    unsafe {
        // 1. Reset RCC to default HSI 8MHz state
        rcc_deinit();

        // 2. Clear SysTick
        const SYST_CSR: *mut u32 = 0xE000_E010 as *mut u32;
        const SYST_RVR: *mut u32 = 0xE000_E014 as *mut u32;
        const SYST_CVR: *mut u32 = 0xE000_E018 as *mut u32;
        ptr::write_volatile(SYST_CSR, 0);
        ptr::write_volatile(SYST_RVR, 0);
        ptr::write_volatile(SYST_CVR, 0);

        // 3. Clear NVIC interrupt enables and pendings
        const NVIC_ICER: *mut u32 = 0xE000_E180 as *mut u32;
        const NVIC_ICPR: *mut u32 = 0xE000_E280 as *mut u32;
        ptr::write_volatile(NVIC_ICER, 0xFFFF_FFFF);
        ptr::write_volatile(NVIC_ICPR, 0xFFFF_FFFF);

        cortex_m::interrupt::disable();

        cortex_m::asm::dsb();

        // 4. Remap System Memory to 0x0000_0000
        const SYSCFG_CFGR1: *mut u32 = 0x4001_0000 as *mut u32;
        let cfgr1 = ptr::read_volatile(SYSCFG_CFGR1);
        ptr::write_volatile(SYSCFG_CFGR1, (cfgr1 & !0x03) | 0x01);

        cortex_m::asm::dsb();
        cortex_m::asm::isb();

        // 5. Read initial MSP and reset handler from the bootloader base address
        let bootloader_addr = profile.dfu_bootloader_addr;
        let initial_msp = ptr::read_volatile(bootloader_addr as *const u32);
        let reset_handler_addr = ptr::read_volatile((bootloader_addr + 4) as *const u32);

        // 6. CRITICAL: Re-enable interrupts! The ST USB DFU ROM bootloader requires IRQs!
        cortex_m::interrupt::enable();

        // 7. Bootstrap into the bootloader
        cortex_m::asm::bootstrap(initial_msp as *const u32, reset_handler_addr as *const u32);
    }
}
