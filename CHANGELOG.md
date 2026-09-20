# Changelog

All notable changes to the `flysky-i6x-rs` project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased] - 2026-09-20

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

### Added
- Native ExpressLRS / CRSF configuration engine with 8 parameter slots.
- Native CRSF telemetry sensor dashboard on Flight Page 3 (LQ, RSSI dBm, SNR, Antenna, Output Power, RF Rate, Battery Voltage, Capacity consumed).
- Live JSON telemetry streaming over USB CDC Serial including CRSF telemetry fields.
- Configurable external module power switch polarity (PC13 Active HIGH / Active LOW).

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
