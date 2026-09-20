# FS-i6X Open-Source Ecosystem & Architectural Context

The FlySky FS-i6X is an accessible, well-engineered RC transmitter powered by an ARM Cortex-M0 microcontroller (STM32F072VB / APM32F072VB). This document provides context on the open-source firmware options available for the platform and the technical design philosophies behind them.

---

## 1. The OpenI6X Project: Paving the Way

The open-source development of the FlySky FS-i6X was made possible by the pioneering work of **Kotak** and the contributors to the [OpenI6X](https://github.com/OpenI6X/opentx) project. 

OpenI6X accomplished a remarkable engineering achievement: adapting the powerful, full-featured OpenTX/EdgeTX operating system to run on an MCU with 128 KB Flash and 16 KB SRAM. In doing so, the OpenI6X project:
- Reverse-engineered the motherboard schematics, MCU pinouts, and peripheral connections.
- Documented the ST7567 LCD 8-bit parallel bus timings and initialization sequences.
- Decoded the Amiccom A7105 SPI transceiver control registers and RF hopping tables.
- Devised and documented essential hardware enhancements, notably the universal **`PC9` backlight PWM dimming mod** and CRSF/ELRS expansion wiring.
- Provided a rich OpenTX user interface, flight timers, model setups, and multi-protocol capabilities to the community.

Anyone developing software for the FS-i6X platform owes deep gratitude to OpenI6X for making this hardware understandable and accessible to everyone.

---

## 2. Design Philosophy of `flysky-i6x-rs`

`flysky-i6x-rs` was created as an experimental, complementary exploration of what a clean-slate firmware written specifically in bare-metal `no_std` Rust looks like on this microcontroller:

- **Clean-Slate Architecture:** Rather than porting an existing multi-target OS, the codebase was designed directly around the STM32F072 hardware peripherals.
- **Bare-Metal `no_std` Rust:** Leverages Rust's memory safety guarantees, zero-cost abstractions, and static allocations with zero dynamic heap usage (`no_std`).
- **Deterministic RF Timing:** Uses dedicated hardware timers (`TIM16`) for frame synchronization, paired with autonomous DMA ADC scanning and a decoupled execution loop.
- **Embedded CRSF Support:** Implements native bidirectional CRSF parameter parsing directly in Rust to configure external modules on the monochrome screen without requiring an external script engine.

Both projects share the common goal of empowering pilots and makers to get the absolute most out of their hardware.

---

## 3. Backing Up, Testing, & DFU Access

Because the STM32F072/APM32F072 features a permanent factory DFU bootloader in System ROM, pilots can explore different firmwares safely:

1. **Enter Bootloader Mode:**
   - **From Stock Factory Firmware:** Bridge the **`R53`** boot pads on the back of the motherboard while switching power ON (see the [OpenI6X Flashing & Upgrading Guide](https://github.com/OpenI6X/opentx/wiki/Flashing-&-Upgrading)).
   - **From OpenI6X or `flysky-i6x-rs`:** Simply hold both horizontal trims inward (**Roll Left + Yaw Right**) toward the power switch while switching power ON.
2. **Back Up Current Firmware (CRITICAL):**
   ```bash
   dfu-util -a 0 -s 0x08000000:131072 -U backup_full.bin
   ```
3. **Restore Anytime:**
   ```bash
   dfu-util -a 0 -s 0x08000000:leave -D backup_full.bin
   ```

For detailed technical documentation on the internal architecture of `flysky-i6x-rs`, please see [ARCHITECTURE.md](ARCHITECTURE.md).
