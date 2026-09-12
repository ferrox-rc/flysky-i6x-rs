# Hardware Reference & Bring-Up Notes

Comprehensive technical documentation for the FlySky FS-i6X hardware reverse-engineered from board analysis and OpenI6X sources.

---

## 1. Microcontroller & Memory

- **Primary MCU:** STMicroelectronics **STM32F072VB** (Cortex-M0 @ 48 MHz).
- **Secondary Clone Variant:** Geehy **APM32F072VB** (pin- and register-compatible clone).
- **Memory Map:**
  - **Flash:** 128 KB (`0x08000000 .. 0x0801FFFF`)
  - **SRAM:** 16 KB (`0x20000000 .. 0x20003FFF`)

### Dual-MCU Silicon Details

| Parameter | STM32F072VB | APM32F072VB |
| :--- | :--- | :--- |
| **96-bit Silicon UID** | `0x1FFFF7AC` | `0x1FFFF7E8` |
| **System Memory ROM (DFU)** | `0x1FFFC800` | `0x1FFFF000` / `0x1FFFC400` |
| **USB DFU VID:PID** | `0483:df11` | `314b:0106` |
| **Flash Page Size** | 2048 bytes (64 pages) | 2048 bytes |

---

## 2. Keypad & Trim Matrix (3 Columns × 4 Rows)

The radio faceplate buttons and trims are arranged in a 3×4 matrix scanned by the MCU, plus one dedicated direct button for Bind.

### Matrix Wiring

- **Columns (Outputs):** `PC6` (Col 0 / R1), `PC7` (Col 1 / R2), `PC8` (Col 2 / R3)
  - Driven LOW sequentially during scanning; kept HIGH at idle.
- **Rows (Inputs):** `PD12` (Row 0 / L1), `PD13` (Row 1 / L2), `PD14` (Row 2 / L3), `PD15` (Row 3 / L4)
  - Configured as inputs with internal pull-ups (`GPIO_PuPd_UP`). Active LOW when pressed.
- **Dedicated Bind Key:** `PF2`
  - Input with internal pull-up. Connected directly to GND when pressed (Active LOW).

### Matrix Key Map

| Row / Line | Column 0 (`PC6`) | Column 1 (`PC7`) | Column 2 (`PC8`) |
| :--- | :--- | :--- | :--- |
| **Row 0 (`PD12`)** | Roll Right (`TRM_RH_UP`) | Throttle Up (`TRM_LV_UP`) | **Down** (`KEY_DOWN`) |
| **Row 1 (`PD13`)** | **Roll Left** (`TRM_RH_DWN`) | Throttle Down (`TRM_LV_DWN`) | **Up** (`KEY_UP`) |
| **Row 2 (`PD14`)** | Pitch Up (`TRM_RV_UP`) | **Yaw Right** (`TRM_LH_UP`) | **OK / Enter** (`KEY_ENTER`) |
| **Row 3 (`PD15`)** | Pitch Down (`TRM_RV_DWN`) | Yaw Left (`TRM_LH_DWN`) | **Cancel / Exit** (`KEY_EXIT`) |

> [!NOTE]
> **Bootloader Combo:** Pushing both horizontal trims inward towards the power switch activates **Roll Left** (`PC6` + `PD13`) and **Yaw Right** (`PC7` + `PD14`). In OpenI6X this corresponds to mask `0x1080`.

---

## 3. ST7567 128×64 Parallel LCD Display

The transmitter uses a Sitronix **ST7567** (or compatible) monochrome LCD controller connected via an 8-bit parallel bus operating in 6800-series mode.

### Pinout & Signals

| Signal | MCU Pin | Function & Idle State |
| :--- | :--- | :--- |
| **D0 .. D7** | `PE0 .. PE7` | Full-byte parallel data bus written via `GPIOE->ODR[7:0]` |
| **RS** | `PB3` | Command / Data select: Low = Command, High = Graphic data |
| **RST** | `PB4` | Active Low hardware reset (pulse Low for $\ge 20\,\mu\text{s}$) |
| **RW** | `PB5` | Read / Write select: Kept **LOW** for write mode |
| **CS** | `PD2` | Chip Select: Kept **LOW** to permanently enable the display |
| **RD / E** | `PD7` | 6800-series latch strobe: Data is latched on **High $\to$ Low** transition |

### Controller Dimensions & Column Offset

The ST7567 controller contains 132 column segment drivers, while the FS-i6X physical LCD panel is 128 pixels wide.
- Active display starts at **Column 4** (`col_start = 0x04`).
- Pages: 8 vertical pages ($8 \times 8 = 64$ rows), each byte containing 8 vertical pixels (LSB at top).
- Total SRAM framebuffer size: $128 \times 8 = 1024$ bytes.

### Initialization Sequence

```
0xE2 -> Software Reset
0xAE -> Display OFF
0xA4 -> Normal RAM display mode
0xA3 -> Bias Select 1/7
0xC0 -> COM Scan Normal (COM0 -> COM63)
0xA1 -> SEG Scan Inverse (SEG131 -> SEG0) [Corrects 180° inversion]
0x2F -> Power Control: Booster, Regulator & Follower all ON
0x23 -> V0 Internal Resistor Ratio (011)
0x81 -> Electronic Volume Mode Set (Contrast)
0x25 -> Contrast Level (0x00 .. 0x3F)
0x40 -> Display Start Line 0
0xB0 -> Page Address 0
0x04 -> Column Address Low Nibble = 4 (Centers display)
0x10 -> Column Address High Nibble = 0
0xAF -> Display ON
```

---

## 4. Backlight Circuitry

### Stock Configuration (Unmodded)
- **Control Pin:** **`GPIOF` Pin 3 (`PF3`)**
- **Polarity:** **Active HIGH** (`PF3 = 3.3V` turns the backlight ON).
- **Circuit:** `PF3` drives the base of an NPN switching transistor through a series resistor. The transistor's collector pulls the LED cathode string to ground.
- **Dimming:** Digital ON/OFF only. `PF3` does not support hardware timer PWM.

### Optional Hardware PWM Mod (Dimming)
- **Control Pin:** **`GPIOC` Pin 9 (`PC9`)** (or `PB1`)
- **Circuit:** Solder jumper added from the unpopulated `PC9` pad to the backlight transistor base pad (`BL`).
- **Dimming:** Driven via `TIM3_CH4` (AF0) with 500 Hz PWM for variable brightness levels ($0\dots 100\%$).
- **Software Strategy:** The firmware simultaneously drives `PF3` and `PC9` HIGH, supporting both stock and modded hardware transparently.

---

## 5. Safe DFU Bootloader Jump

The STM32F072 contains a factory-programmed DFU bootloader in System ROM (`0x1FFFC800`). The firmware can jump into this bootloader in software without physical access to the `BOOT0` pin.

### Jump Requirements

1. **Reset RCC:** Return all peripheral clocks to power-on defaults (HSI 8 MHz, PLL disabled).
2. **Clear SysTick & NVIC:** Disable SysTick timer and clear all pending interrupt requests in NVIC.
3. **SYSCFG Remap:** Remap System Memory to `0x00000000` via `SYSCFG->CFGR1` (`MEM_MODE = 0b01`).
4. **Re-Enable Global Interrupts:** **CRITICAL.** The ST factory DFU bootloader requires USB interrupts to enumerate on the host PC. Global interrupts must be enabled (`cortex_m::interrupt::enable()`) before executing the jump.
5. **Bootstrap:** Load Main Stack Pointer (`MSP`) from `0x1FFFC800` and branch to reset handler at `0x1FFFC804` via `cortex_m::asm::bootstrap`.

### Hardware Recovery (`R53`)

If custom firmware ever hangs before polling the keys, the hardware override is the **`R53`** solder pads located on the back of the motherboard:
- Shorting `R53` pulls `BOOT0` to 3.3V.
- Powering on while shorted forces the chip directly into the ROM bootloader (`0483:df11`).
