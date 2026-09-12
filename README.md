# flysky-i6x-rs

A minimalist, clean-slate RC transmitter firmware for the **FlySky FS-i6X**, written in `no_std` Rust.

Focused on the built-in **A7105** 2.4 GHz RF transceiver (**AFHDS2A** protocol), **i-BUS** telemetry, and the **128×64 LCD** interface.

---

## 1. Overview & Philosophy

The standard OpenTX/EdgeTX port for the FS-i6X ([OpenI6X](https://github.com/OpenI6X/opentx)) is an incredible engineering feat that fits a ~30,000 LOC C++ codebase into the microcontroller's 128 KB flash memory. However, it operates at ~94% flash capacity with less than 7 KB of headroom, making customization and maintenance difficult.

`flysky-i6x-rs` is a **clean-slate rewrite** in Rust designed with:
- **Zero-cost abstractions:** Microcontroller-native, static allocation, no heap allocations (`no_std`).
- **Hard Real-Time Concurrency:** Interrupt-driven scheduling using RTIC (or async Embassy) for deterministic sub-millisecond RF hopping and packet timing.
- **Strict Scope:** Dedicated support for the built-in hardware (A7105 AFHDS2A + i-BUS), 4-axis gimbals, switches, trims, and an OpenTX-inspired 128×64 monochrome UI.
- **Estimated Footprint:** ~35–50 KB Flash (leaving ~70 KB free) and ~3–4 KB RAM (leaving ~12 KB free).

---

## 2. Target Hardware Map (STM32F072VB)

| Peripheral | Controller / Spec | MCU Pins & Ports | Notes |
| :--- | :--- | :--- | :--- |
| **MCU** | STM32F072VB (Cortex-M0 @ 48 MHz) | ARMv6-M (`thumbv6m-none-eabi`) | 128 KB Flash, 16 KB SRAM |
| **RF Transceiver** | Amiccom **A7105** 2.4 GHz | **SPI1** + GPIOs | SPI1 (SCK, MOSI, MISO) |
| | Chip Select (CSN) | `PE12` (Active Low) | Fast GPIO output |
| | Antenna Switch | `PE10` (RF0), `PE11` (RF1) | Diversity / TR switch |
| | Packet Ready IRQ | `PB2` (EXTI2) | GIO2 line from A7105 (Tx/Rx done) |
| | RF Timer | `TIM16` | Periodic packet scheduling (~3.8ms–7.5ms) |
| **Display** | **ST7567** (128×64 Monochrome LCD) | 8-bit 6800 Parallel Bus | 1024-byte framebuffer in RAM |
| | Data Bus (D0..D7) | `PE0 .. PE7` | Full-byte ODR write (`GPIOE->ODR[7:0]`) |
| | Command/Data (RS) | `PB3` | Low = Command, High = Data |
| | Reset (RST) | `PB4` | Active Low hardware reset |
| | Read/Write (RW) | `PB5` | Kept Low for write mode |
| | Chip Select (CS) | `PD2` | Active Low (held Low for bus access) |
| | Strobe (RD / E) | `PD7` | 6800-series latch strobe (High -> Low pulse) |
| | Backlight (Stock) | `PF3` | **Active HIGH** (drives NPN transistor base) |
| | Backlight (Modded)| `PC9` / `PB1` | `TIM3_CH4` PWM dimming mod pad |
| **Analog Inputs** | 12-bit ADC1 via DMA | 10 Channels scanned | Circular DMA buffer |
| | Sticks (RV, RH, LV, LH) | `PA0`, `PA1`, `PA2`, `PA3` | Channels 0, 1, 2, 3 |
| | Potentiometers (VR1, VR2)| `PA6`, `PB0` | Channels 6, 8 |
| | Battery Sense | `PC0` | Channel 10 (voltage divider) |
| | Switches (SA, SB, SC, SD)| Resistor dividers on ADC | Channels 4, 5, 7, 9 |
| **Digital Keys** | 3 Columns × 4 Rows Matrix | Keypad & Trims | Polled at ~50–100 Hz |
| | Matrix Columns (R1..R3)| `PC6`, `PC7`, `PC8` | Driven Low sequentially |
| | Matrix Rows (L1..L4) | `PD12`, `PD13`, `PD14`, `PD15` | Inputs with internal pull-ups |
| | Inward Trim Keys | `PC6`+`PD13` & `PC7`+`PD14` | Roll Left (RHL) + Yaw Right (LHR) |
| | Dedicated Bind Key | `PF2` | Active Low (pull-up enabled) |
| **Storage** | 24C64 I2C EEPROM (64 Kbit) | `I2C2` (`PB10` SCL, `PB11` SDA) | Model configs & calibrations |
| **Telemetry / Serial**| UART Interfaces | `USART2` (PD5 Tx / PA15 Rx) | External telemetry / i-BUS mirror |
| **Audio** | Piezo Buzzer | `TIM14` | Frequency & tone generator |

---

## 3. Software Architecture

```
                 +---------------------------------------------+
                 |            RTIC v2 Application              |
                 +---------------------------------------------+
                   |                 |                       |
       [Priority 3 (High)]    [Priority 2 (Mid)]     [Priority 1 (Low / Idle)]
       ------------------     ------------------     -------------------------
       TIM16 & EXTI2 (RF)       ADC1 DMA IRQ            SysTick / Idle Loop
       - A7105 State Machine   - Stick Calibration    - Key Matrix Debounce
       - AFHDS2A Packet Tx     - Mixer & Rates/Expo   - 20 Hz Display Engine
       - i-BUS Telemetry Rx    - Channel Mapping      - Menu Navigation
                               - Failsafe Monitor     - EEPROM Persistence
```

### Key Libraries / Crates
- `cortex-m`, `cortex-m-rt`: Core ARM runtime and interrupt vector tables.
- `stm32f0xx-hal`: Embedded HAL implementation for STM32F0 peripherals.
- `rtic` (v2.x): Real-Time Interrupt-driven Concurrency framework for hardware-timed tasks without an RTOS.
- `embedded-graphics`: Monochrome UI primitive rendering, fonts, lines, and bitmaps.
- `embedded-hal`: Trait abstractions for SPI, I2C, and GPIO.

---

## 4. AFHDS2A & i-BUS Subsystem Design

### Frequency Hopping (FHSS)
AFHDS2A distributes transmission across 16 pseudo-random frequencies generated from the transmitter's unique silicon ID:
```
UID (at 0x1FFFF7AC) -> LCG Random Seed -> 16 Unique Channels (1..164 with min spacing >= 5)
```

### Packet Protocol
1. **Bind Packets (`0xBB` / `0xBC`):** Broadcasts TX ID and hopping table to allow receivers (e.g. FS-iA6B) to sync.
2. **Stick Data Packet (`0x58`):** 38-byte frame containing up to 14 channels encoded as microsecond pulses (1000–2000 µs).
3. **Settings Packet (`0xAA`):** Directs receiver output mode (PWM, PPM, i-BUS, or S.BUS).
4. **Telemetry Reception Window:** The transmitter switches the A7105 to RX immediately following packet transmission to receive i-BUS sensor telemetry frames (TX/RX voltage, RSSI, temperature, RPM, etc.).

---

## 5. Display & User Interface (128×64)

The ST7567 parallel LCD driver maintains a **1024-byte framebuffer** in SRAM (`128 * 64 / 8`). Updating the entire screen takes $< 1\text{ ms}$ via direct 8-bit GPIO port writes (`GPIOE->ODR`).

### Planned Screens (OpenTX Inspired)
1. **Flight Screen (Main View):**
   - Top status bar: Model name, TX battery voltage, RX battery voltage, RSSI signal bar.
   - Graphic channel indicators: Live CH1–CH4 horizontal bar graphs.
   - Flight timer & digital trim indicators.
2. **Channel Monitor:**
   - Numerical (µs) and bar display for all active channels (CH1–CH14).
3. **Model Configuration:**
   - Channel reversing, dual rates, expo curves, endpoints (min/max), and sub-trims.
4. **Telemetry Monitor:**
   - Live sensor list displaying received i-BUS sensor IDs, names, and current readings.
5. **RX Setup & Bind:**
   - AFHDS2A bind trigger, receiver output mode selection (PWM / i-BUS), and failsafe configuration.

---

## 6. Implementation Roadmap

### Phase 1: Board Bring-Up & Display (COMPLETED)
- [x] Configure `cortex-m-rt`, linker script (`memory.x`), and target `thumbv6m-none-eabi`.
- [x] Dual MCU support for both `STM32F072VB` and `APM32F072VB` (UID & DFU mapping).
- [x] Implement parallel 8-bit ST7567 driver for `GPIOE` (ODR write) + control lines.
- [x] Connect `embedded-graphics` `DrawTarget` to 1024-byte SRAM buffer.
- [x] Fix screen orientation (`0xA1`/`0xC0`), column 4 offset, and factory backlight on `PF3`.
- [x] Safe DFU bootloader software jump on startup/runtime via inward trims or Bind key.
- [x] Fast power-on boot (< 30 ms).
- **Footprint:** **8.9 KB Flash** (leaves > 119 KB free / ~93% headroom), **0 B static data**, **> 14 KB free SRAM**.

### Phase 2: Analog & Digital Inputs
- [ ] Setup ADC1 with DMA continuous circular buffer for 4 stick axes + battery voltage.
- [ ] Calibrate ADC readings to normalized stick positions (`-1000 .. +1000`).
- [ ] Implement key matrix scanner on `GPIOC`/`GPIOD` for trims, buttons, and bind switch.
- [ ] Create basic stick calibration wizard screen.

### Phase 3: A7105 SPI Driver & Hopping Table
- [ ] Implement A7105 SPI1 driver and verify register read/write (confirm chip ID `0x00` / `0x01`).
- [ ] Configure RF power levels and antenna switch control (`PE10`/`PE11`).
- [ ] Generate 16-channel pseudo-random hopping table from STM32 Unique Device ID.
- [ ] Configure `TIM16` for microsecond-accurate packet interval timing.

### Phase 4: AFHDS2A Over-the-Air Link
- [ ] Implement AFHDS2A 4-phase bind sequence (`0xBB`/`0xBC`).
- [ ] Verify successful binding with a physical FlySky receiver (e.g., FS-iA6B).
- [ ] Implement normal data packet transmission (`0x58`) with live gimbal channel data.
- [ ] Verify servo / flight controller response over PWM / i-BUS receiver pins.

### Phase 5: i-BUS Telemetry, UI & Storage
- [ ] Implement A7105 RX window and parse incoming i-BUS telemetry packets.
- [ ] Display RSSI and receiver battery voltage on the main flight screen.
- [ ] Implement I2C EEPROM driver (`24C64`) to persist model configs and trims.
- [ ] Finalize clean menu navigation and telemetry screens.

---

## 7. Toolchain, Flashing & Reversion

- **Rust Target:** `thumbv6m-none-eabi`
- **Compiler:** `stable` or `nightly` (edition 2021)
- **DFU Flashing (USB):**
  1. Push both horizontal trims inward toward the power switch (or hold the Bind key) and turn ON.
  2. The MCU jumps directly into the factory ST ROM bootloader (`0483:df11`).
  3. Flash firmware via `dfu-util`:
     ```bash
     dfu-util -a0 -s 0x08000000:leave -d 0483:df11 -D target/flysky-i6x-rs.bin
     ```
- **Reverting to OpenTX / OpenI6X:**
  Because the factory bootloader resides in permanent ROM, you can restore your original firmware anytime:
  ```bash
  dfu-util -a0 -s 0x08000000:leave -d 0483:df11 -D opentx_backup.bin
  ```
- **Hardware Recovery Override:**
  If custom code ever hangs before key polling, short the `R53` pads (BOOT0 to 3.3V) with tweezers while plugging in USB to force hardware DFU mode.

