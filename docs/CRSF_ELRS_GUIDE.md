# CRSF / ExpressLRS Subsystem & Native Module Configurator

Technical guide and user documentation for the native Crossfire (CRSF) and ExpressLRS (ELRS) subsystem in the FlySky FS-i6X Rust firmware.

---

## 1. Hardware Interface

The FlySky FS-i6X motherboard provides an internal rear module connector and back port routed to STM32F072VB peripherals:

| Pin | Function | Peripheral | Description |
| :--- | :--- | :--- | :--- |
| `PD5` | **USART2_TX** | AF0 | Bidirectional CRSF serial TX to external ELRS/Crossfire TX module |
| `PA15` | **USART2_RX** | AF1 | Telemetry serial RX from external TX module |
| `PC13` | **MOD_PWR** | GPIO Out | External module power rail switch (Active HIGH: powers module via VCC) |

Implemented in [`src/crsf/uart.rs`](../src/crsf/uart.rs).

### Supported Baud Rates
Baud rates can be selected per-model in `9. Protocol Setup`:
- **420,000 baud** (Default for ExpressLRS high-speed communication)
- **416,666 baud** (Standard Team BlackSheep Crossfire)
- **115,200 baud** (Low-speed debug / legacy transmitters)
- **921,600 baud** (Ultra-low latency for compatible external microcontrollers)

---

## 2. Flight Dashboard: Native CRSF Link Diagnostics

When `rf_protocol` is set to `1` (`CRSF / ELRS`), Page 4/4 of the flight dashboard transitions from the AFHDS 2A packet counter into a real-time link diagnostics screen:

```
+---------------------------------------------------------------+
| [M01:DRONE   ]   CRSF: 250Hz               [ 4.8V]            |
+---------------------------------------------------------------+
| LQ:   100%            | PWR:  100mW                           |
| RSSI: -72dBm          | RATE: 250Hz (Mode 7)                  |
| SNR:  +12dB           | BAT:  16.4V                           |
| ANT:  1 (Active)      | CAP:  450mAh                          |
+---------------------------------------------------------------+
| P4/4           CRSF LINK DIAGNOSTICS                          |
+---------------------------------------------------------------+
```

### Metrics Decoded:
- **Link Quality (`LQ`)**: Decoded from CRSF Frame `0x14` (`CRSF_FRAMETYPE_LINK_STATISTICS`), byte 8 (`uplink_link_quality`). Displayed as `0..100%`.
- **RSSI (`RSSI 1`)**: Decoded from byte 3 (`uplink_rssi_1`). Displayed directly in `dBm` (e.g. `-72dBm`).
- **SNR (`SNR`)**: Decoded from byte 9 (`uplink_snr`). Displayed in signed `dB` (e.g. `+12dB` / `-4dB`).
- **Active Antenna (`ANT`)**: Decoded from byte 7 (`active_antenna`). Shows `1` or `2`.
- **TX Power (`PWR`)**: ExpressLRS Status frame / link statistics transmit power in milliwatts (`mW`).
- **RF Packet Rate (`RATE`)**: Automatically resolved from `rf_mode` index to human-readable rates (`50Hz`, `100Hz`, `150Hz`, `250Hz`, `333Hz`, `500Hz`, `D250`, `D500`, `F500`, `F1000`).
- **Flight Pack Battery (`BAT` / `CAP`)**: Decoded from CRSF Frame `0x08` (`CRSF_FRAMETYPE_BATTERY_SENSOR`). Displays battery voltage in `0.1V` precision and consumed capacity in `mAh`.

---

## 3. Native Module Configurator

On standard EdgeTX and OpenTX radios, ExpressLRS module configuration is typically handled via a Lua script. Because the FS-i6X's STM32F072 microcontroller has 16 KB of SRAM, running a full Lua virtual machine is impractical on this platform.

To enable full on-radio module configuration, `flysky-i6x-rs` implements the **bidirectional CRSF parameter protocol** natively in bare-metal Rust with **zero dynamic heap allocation**:

### Parameter Exchange Protocol
```text
Radio (FS-i6X)                               External ELRS TX Module
      |                                                 |
      | -------- 0x28 (DEVICE_PING) ------------------> |
      | <------- 0x29 (DEVICE_INFO: Name, Count) ------ |
      |                                                 |
      | loop for each parameter:                        |
      | -------- 0x2C (PARAM_READ: ID, Chunk) --------> |
      | <------- 0x2B (PARAM_ENTRY: Name, Options) ---- |
      |                                                 |
      | [User changes setting or clicks action]         |
      | -------- 0x2D (PARAM_WRITE: ID, Value) -------> |
```

### Navigating the Configurator:
1. Open Menu with long-press `[OK]`.
2. Scroll to `9. Protocol Setup` and press `[OK]`.
3. Highlight `[Configure Module]` and press `[OK]`.
4. The radio sends `0x28 Ping` and dynamically populates parameters (Packet Rate, Power, TLM Ratio, Wi-Fi Mode, Bind).
5. Press `[UP]` / `[DOWN]` to navigate between parameters.
6. Press `[OK]` on a selection option (e.g., `Rate`) to cycle through available frequencies immediately.
7. Press `[OK]` on a command action (e.g., `[Wi-Fi Mode]` or `[Bind]`) to trigger module functions.
8. Press `[ESC]` at any time to return to the Protocol Setup menu.

---

## 4. USB CDC Telemetry Streaming

In `Serial` or `Composite` USB mode, the transmitter streams JSON telemetry over the virtual COM port (`i6x> stream` or `i6x> telem`):

```json
{
  "vbat": 5.18,
  "rssi": 98,
  "rx_v": 5.02,
  "tx": 15820,
  "rx": 15798,
  "err": 22,
  "crsf": {
    "conn": true,
    "lq": 100,
    "rssi_dbm": -72,
    "snr_db": 12,
    "ant": 1,
    "pwr_mw": 100,
    "rf_rate": "250Hz",
    "rx_vbat_mv": 16400,
    "rx_cap_mah": 450
  },
  "ch": [1500, 1500, 1000, 1500, 1500, 1500, 1500, 1500, 1500, 1500, 1500, 1500, 1500, 1500]
}
```

The CLI `status` command reports the active protocol:
```text
i6x> status
FlySky FS-i6X Rust Firmware v0.15.0
Protocol: CRSF / ExpressLRS (PD5 UART active)
{"vbat":5.18,...}
```
