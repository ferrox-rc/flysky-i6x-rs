# Changelog

All notable changes to the `flysky-i6x-rs` project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.16.0] - 2026-09-21

### Added
- **Unified UI Subsystem (`src/ui/`)**:
  - `src/ui/widgets.rs`: Shared zero-allocation UI components (`draw_header`, `draw_footer`, `draw_footer_split`, `draw_bar_gauge`, `draw_channel_gauge`, `draw_progress_bar`, `draw_list_row`, `navigate_4slot_list`).
  - `src/ui/format.rs`: Shared zero-allocation stack formatters (`format_vbat`, `format_percent`, `format_throttle_percent`, `format_trim`, `u32_to_hex`, `u32_to_dec_5`, `u16_to_dec_4`, `i8_to_dec`, `u8_to_dec`).
  - `src/ui/dashboard/`: Modular 4-page live flight dashboard (`pages/gimbals.rs`, `pages/channels.rs`, `pages/model.rs`, `pages/telemetry.rs`, `status_bar.rs`).
  - `src/ui/menu/`: Modular settings menu decomposed into discrete screen controllers.
  - Standardized 8-pixel footer geometry ($y = 55$ divider, $y = 62$ `FONT_4X6` baseline) across all flight dashboards and settings screens, eliminating text clipping on 128px displays.
- Native ExpressLRS / CRSF configuration engine with 8 parameter slots.
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
