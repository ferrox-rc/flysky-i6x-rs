# Changelog

All notable changes to the `flysky-i6x-rs` project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.16.0-rc.3] - 2026-09-23

### Summary
Comprehensive architectural upgrade migrating the peripheral driver layer to direct register access via the Peripheral Access Crate (`pac`), implementing a 2.0-second hardware watchdog with debug halt freezing, introducing an 8 KB 4-page append-only log storage engine powered by `sequential-storage` (yielding sub-3ms non-blocking saves with zero page erases on edits), refining USB composite mode with silent CLI connection, and synchronizing all project documentation.

### Added
- **Pure PAC Hardware Peripheral Architecture (`7c45a2c`)**:
  - **System Clock & LSI Stabilization ([`src/chip/mod.rs`](src/chip/mod.rs))**: Migrated clock configuration and flash latency setup to pure PAC (`pac::RCC` and `pac::FLASH`). Added explicit LSI enable (`RCC_CSR.lsion`) with bounded wait loop on `RCC_CSR.lsirdy` to guarantee the 40 kHz internal low-speed oscillator is stable prior to peripheral and watchdog activation.
  - **Key Matrix Scanning ([`src/boot.rs`](src/boot.rs))**: Migrated `init_keys()` and `scan_keys()` to `pac::GPIOC`, `pac::GPIOD`, `pac::GPIOF`, and `pac::RCC`.
  - **Piezo PWM Buzzer ([`src/buzzer.rs`](src/buzzer.rs))**: Migrated PA8 alternate function routing and TIM1 PWM setup to `pac::GPIOA`, `pac::TIM1`, and `pac::RCC`.
  - **ST7567 LCD Driver ([`src/display/st7567.rs`](src/display/st7567.rs))**: Migrated 8-bit parallel bus control (`GPIOE->ODR`) and control strobe sequencing to direct PAC registers. Framebuffer is explicitly cleared (`clear_buffer()`) and pushed to the display (`flush()`) *before* backlight PWM is turned on, preventing power-on visual noise.
- **Deterministic 2.0s Hardware Watchdog (`b4deb69`)**:
  - Independent hardware watchdog implemented via `pac::IWDG` clocked by the 40 kHz LSI oscillator with prescaler `/128` (PR=5, 312.5 Hz tick rate) and reload count `625` (exact 2.000s timeout).
  - **Debug Halt Freezing**: Sets `DBGMCU_APB1_FZ.DBG_IWDG_STOP` with `RCC_APB2ENR.DBGMCUEN` pre-enabled, allowing SWD debuggers (ST-Link, probe-rs, GDB) to pause CPU execution on breakpoints without triggering watchdog resets.
  - **Standalone Zero-Cost Feed**: Inlined `watchdog::feed()` writing key `0xAAAA` to `IWDG_KR`, called at ~500 Hz at the bottom of the main execution loop and during storage operations.
- **Log-Structured Append-Only Flash Storage Engine (`832a72b`)**:
  - **4-Page Memory Allocation**: Pages 60, 61, 62, and 63 (`0x0801_E000 .. 0x0802_0000`, 8,192 bytes total) reserved exclusively for non-volatile storage.
  - **Log-Structured Engine**: Powered by `sequential-storage` (`sequential_storage::map`), mapping Key 0 to `RadioConfig` (128 bytes) and Keys 1..20 to `ModelConfig` profiles (128 bytes each).
  - **Sub-3ms Non-Blocking Saves**: Incremental updates append only ~132 bytes to the open log page in **~2.8 ms with zero page erases**, eliminating the ~50 ms UI stall and loop jitter of whole-page erasing.
  - **Automatic Wear-Levelled Compaction**: When all 4 sectors become full of historical revisions, `sequential-storage` automatically compacts active records into a newly erased page, rotating evenly across Pages 60–63.
  - **Multi-Tier Legacy Migration**: Probes sequential storage first; if empty, automatically imports legacy v3 snapshot data from Page 62 (`0x0801_F000`) or legacy v1/v2 data from Page 63 (`0x0801_F800`) before committing factory defaults.
- **Pure PAC Flash Driver (`FlashStorage`)**:
  - Direct PAC register implementation of `embedded_storage::nor_flash::NorFlash` and `MultiwriteNorFlash` using `pac::FLASH`.
  - Flash unlock sequence via `FLASH_KEYR` (`0x4567_0123`, `0xCDEF_89AB`) and status flag hygiene (`EOP`, `WRPRTERR`, `PGERR`).
- **Main Event Loop & Safety Lifecycle Integration (`1fd6a85`)**:
  - Deterministic boot order: early watchdog feed -> DFU check -> clock init & LSI stabilization -> SysTick -> clean LCD wipe -> watchdog start -> peripheral init -> pre-flight check -> main loop.
  - 100% stick throw preservation with adaptive pre-flight checks: uncalibrated checks raw ADC counts (`state.raw[2] > 1400`), calibrated checks normalized pulses (`state.sticks.throttle > -900`).
- **USB Composite Mode & Silent CLI Experience (`fc8c88f`)**:
  - EdgeTX composite device identity (`0x1209:0x4968`) with Interface Association Descriptors (IAD) enabling simultaneous 100 Hz HID Gamepad and CDC-ACM Virtual COM port.
  - Silent terminal connection: eliminated unsolicited banner transmission on USB enumeration, preventing buffer stalls, FIFO packet drops, and truncated greetings in terminal emulators (`picocom`, `minicom`, PuTTY).
  - Interactive prompt `i6x> ` rendered upon `[Enter]`, with built-in commands: `help`, `status`, `channels`, `telem`, `stream` (continuous 10 Hz JSON telemetry streaming, any key to pause), and `reboot`.

### Changed
- **Memory Map Partitioning (`memory.x`)**:
  - Clamped application flash partition `FLASH (rx)` to `120K` (`0x0800_0000 .. 0x0801_DFFF`, Pages 0–59), physically preventing linker code overflow from invading the storage sector at `0x0801_E000`.
- **Comprehensive Documentation Synchronization (`14874cf`)**:
  - Fully updated [`README.md`](README.md), [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`docs/CALIBRATION_AND_STORAGE.md`](docs/CALIBRATION_AND_STORAGE.md), [`docs/HARDWARE_REFERENCE.md`](docs/HARDWARE_REFERENCE.md), [`docs/USB_SUBSYSTEM.md`](docs/USB_SUBSYSTEM.md), [`docs/MIXER.md`](docs/MIXER.md), and [`docs/USER_GUIDE.md`](docs/USER_GUIDE.md) to reflect PAC drivers, watchdog timing, 4-page sequential storage, and USB composite CLI.

### Fixed
- **Flash Control Register LOCK Bit Re-Assertion (`d6bea38`)**:
  - **Issue**: Standard `modify()` writes on `FLASH_CR` in `stm32f0xx-hal` re-wrote the read state of bit 7 (`LOCK`), inadvertently re-locking the flash peripheral before page erase or halfword programming could execute.
  - **Resolution**: Implemented PAC register writes using `write_with_zero` on `FLASH_CR` with `LOCK = 0`, ensuring flash remains unlocked throughout erase and write sequences.
- **Flash Status Register Error Flag Clearing & Page Erase Sequence (`4d3e244`)**:
  - **Issue**: Lingering `PGERR` or `WRPRTERR` flags from prior power cycles blocked subsequent erase operations.
  - **Resolution**: Cleared all status flags prior to operation and followed ST programming manual sequence (`PER` set, `AR` address write, `STRT` assert, `BSY` poll, `PER` clear).
- **Watchdog APB1 Freeze HardFault (`b4deb69`)**:
  - **Issue**: Writing to `DBGMCU_APB1_FZ` during startup caused an immediate bus fault and continuous buzzer lockup because the DBGMCU clock was not enabled.
  - **Resolution**: Asserted `RCC_APB2ENR.DBGMCUEN` before setting `DBGMCU_APB1_FZ.DBG_IWDG_STOP`.
- **USB Terminal Corrupted Greeting Banner (`fc8c88f`)**:
  - **Issue**: Sending an unsolicited 160-byte greeting banner immediately upon USB bus configuration caused terminal emulators connected later (via `picocom /dev/ttyACM0`) to display partial leftover fragments (`================================`).
  - **Resolution**: Removed automatic enumeration banner; terminal connects silently and provides clean interactive CLI on demand.

### Firmware Footprint Verification
```text
   text    data     bss     dec     hex filename
  89432    1876    1128   92436   16914 flysky-i6x-rs
```
- **Application Flash (.text + .data)**: **91,308 bytes (~89.2 KB)** used out of **120 KB (122,880 bytes)** code partition (**>30.8 KB / 25.7% free headroom**).
- **Static RAM (.data + .bss)**: **3,004 bytes (~2.9 KB)** out of **16 KB (16,384 bytes)** total SRAM (**>81% free**, >6.3 KB stack margin).
- **Non-Volatile Storage**: **8,192 bytes** (Pages 60–63, `0x0801_E000 .. 0x0802_0000`).

---

## [0.16.0-rc.1] - 2026-09-21

### Added
- **Unified UI Subsystem (`src/ui/`)**:
  - `src/ui/widgets.rs`: Shared zero-allocation UI components (`draw_header`, `draw_footer`, `draw_footer_split`, `draw_bar_gauge`, `draw_channel_gauge`, `draw_progress_bar`, `draw_list_row`, `navigate_4slot_list`).
  - `src/ui/format.rs`: Shared zero-allocation stack formatters (`format_vbat`, `format_percent`, `format_throttle_percent`, `format_trim`, `u32_to_hex`, `u32_to_dec_5`, `u16_to_dec_4`, `i8_to_dec`, `u8_to_dec`).
  - `src/ui/dashboard/`: Modular 4-page live flight dashboard (`pages/gimbals.rs`, `pages/channels.rs`, `pages/model.rs`, `pages/telemetry.rs`, `status_bar.rs`).
  - `src/ui/menu/`: Modular settings menu decomposed into discrete screen controllers.
  - Standardized 8-pixel footer geometry ($y = 55$ divider, $y = 62$ `FONT_4X6` baseline) across all flight dashboards and settings screens, eliminating text clipping on 128px displays.
- **Rich Audio & Switch Chimes (PR #4)**:
  - Melodic welcome and goodbye chimes with selectable tone styles (`RICH` vs `SIMPLE`).
  - Per-model configurable arm switch assignments and armed/disarmed audio notifications.
- **ExpressLRS / CRSF Integration & Hardened Command Lifecycle**:
  - Native ELRS configuration engine with 8 parameter slots (`CRSF_FRAMETYPE_PARAMETER_READ` 0x2C / `PARAMETER_WRITE` 0x2D / `PARAMETER_ENTRY` 0x2B), labeled `ELRS Setup (Beta)`.
  - Robust non-blocking `ActiveCommandState` machine for actions (Wi-Fi, Bind, etc.) with automatic 250ms periodic `STATUS_POLL` transmission, watchdog timeouts, and interactive modal confirmation dialogs.
  - Unified extended parameter wire frame serialization via `build_param_ext_frame`.
  - Native CRSF telemetry sensor dashboard on Flight Page 3 (LQ, RSSI dBm, SNR, Antenna, Output Power, RF Rate, Battery Voltage, Capacity consumed).
  - Live JSON telemetry streaming over USB CDC Serial including CRSF telemetry fields.
  - Configurable external module power switch polarity (PC13 Active HIGH / Active LOW).

### Changed
- Slimmed `src/main.rs` by over 570 lines, delegating inline display rendering to `DashboardController`.
- Reduced Flash `.text` footprint by ~984 bytes via formatter and widget deduplication.

### Fixed
- **Critical Boot Stack Overflow & Reverse Throttle Fix (`dev-crsf`)**:
  - **Incident Summary**: When building the `dev-crsf` branch, the throttle stick was consistently inverted at boot (causing the "THROTTLE NOT AT IDLE!" startup alarm to require full stick-up to pass, channel 3 showing 2000 µs at idle, and Flight Page 1 showing 100% with stick down). Rerunning gimbal calibration completed successfully but did not resolve the inversion.
  - **Root Cause Analysis**:
    - During system boot, nested calls from `main()` -> `input::init()` -> `storage::load_config()` -> `storage::load_storage()` -> `RadioStorage::default_factory()` repeatedly allocated 2,688-byte `RadioStorage` instances by value on the stack.
    - Combined with `main()`'s 7.3 KB stack allocation, this pushed the call-chain stack depth to **15,428 bytes**—on an STM32F072 microcontroller with only **16,384 bytes (16 KB) of total SRAM**.
    - In commit `66a0b94`, the addition of `CONFIG_ENGINE` (896 bytes in `.data`) shifted static variables higher in memory, placing `THROTTLE_CALIB` at `0x2000_0a10`.
    - When the stack reached down to `0x2000_03dc` during `load_config()`, stack frames directly collided with and corrupted static RAM, specifically writing non-zero data into `0x2000_0a1a` (`THROTTLE_CALIB.invert = true`).
    - Because `apply_calibration()` only updated `min`, `center`, and `max` endpoints, `THROTTLE_CALIB.invert` remained permanently set to `true`, causing `normalize()` to negate -1000 to +1000.
  - **Resolution**:
    - **In-Place Storage Loading (`load_storage_into`)**: Eliminated pass-by-value stack instantiation of 2,688-byte `RadioStorage`. Flash data is read directly into caller-provided memory.
    - **Lightweight `load_config()`**: Refactored `storage::load_config()` to read only the 128-byte `RadioConfig` header directly from Flash without instantiating full 20-model `RadioStorage`.
    - **Lightweight `load_saved_rx_id()`**: Refactored AFHDS 2A receiver ID loading to read the 4-byte `rx_id` directly from Flash without stack allocations.
    - **BSS Relocation & Downsizing of `CONFIG_ENGINE`**: Relocated `CONFIG_ENGINE` from `.data` to `.bss` (setting `device_id` to 0 in const constructor and assigning `0xEE` on handshake start). Reduced `MAX_PARAMS` to 8 and optimized option buffers, reducing size from 952 bytes to 516 bytes and shrinking `.data` back to 1,776 bytes.
    - **Immutable Axis Inversion Flags**: Updated `input::apply_calibration()` to explicitly enforce `invert = false` on `THROTTLE_CALIB` and `YAW_CALIB`, and `invert = true` on `ROLL_CALIB` and `PITCH_CALIB`.
    - **Stack Safety Margin Restored**: Maximum boot stack usage dropped from **15.4 KB down to ~7.2 KB**, leaving **> 6.3 KB of guaranteed uncorrupted safety margin** between the stack and static RAM.
- **CRSF Protocol Framing & Sync Byte Compliance (`tbs-fpv/tbs-crsf-spec`)**:
  - **Serial Sync Byte Compliance (`0xC8`)**: Conformed all outgoing extended frames (`DEVICE_PING` 0x28, `PARAMETER_READ` 0x2C, `PARAMETER_WRITE` 0x2D) to start with `0xC8` (`CRSF_SYNC_BYTE`). Previously `build_ping_frame` emitted broadcast `0x00` on the wire and parameter frames emitted destination `0xEE`, causing external module serial parsers to discard packets.
  - **Frame Length Field Off-By-One Fix**: Corrected length field (`out_frame[1]`) from 5 to 6 in `build_param_read_frame()` and `build_param_write_frame()` to accurately encompass `Type (1) + Dest (1) + Origin (1) + Param (1) + Chunk/Val (1) + CRC (1) = 6` per TBS specification.
  - **Unbounded Null-Terminator Search**: Fixed premature loop exit when scanning device and parameter names with lengths $\ge$ 20 or $\ge$ 16 bytes, preventing memory offset miscalculations when parsing parameter counts, options lists, and current values.
  - **Multi-Chunk Parameter Retrieval & Rapid Pipeline**: Automatically request subsequent parameter chunks when `chunks_remain > 0`, and immediately dispatch parameter 1 request upon receiving device info `0x29`.
  - **ELRS Status Keep-Alive**: Extended telemetry parser to accept and keep the telemetry connection alive on incoming `0x2E` (`CRSF_FRAMETYPE_ELRS_STATUS`) packets.
---

## [0.15.1] - 2026-09-19

### Changed
- Bumped version and refreshed release artifacts.
- Clippy `-D warnings` cleanup across telemetry match guards.
- Cargo bare-metal harness configuration updates.

---

## [0.15.0] - 2026-09-19

### Added
- USB composite device mode (HID Joystick + CDC Serial simultaneous operation).
- Matrix mixer engine with 16 configurable mix rules.
- AFHDS 2A autonomous continuous telemetry stream and sensor parsing.
