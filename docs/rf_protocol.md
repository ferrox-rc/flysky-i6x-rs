# AFHDS 2A Protocol & A7105 RF Driver

Technical documentation for the Amiccom A7105 2.4 GHz FSK transceiver driver and the FlySky AFHDS 2A (Automatic Frequency Hopping Digital System 2nd Gen) protocol implementation.

---

## 1. Hardware Interface

The A7105 transceiver is connected to the STM32F072VB via SPI1 and auxiliary control pins:

| Pin | Function | Description |
| :--- | :--- | :--- |
| `PB3` | **SPI1_SCK** | SPI Clock (up to 12 MHz) |
| `PB4` | **SPI1_MISO** | Data from A7105 |
| `PB5` | **SPI1_MOSI** | Data to A7105 |
| `PE12` | **CSN** | Chip Select (Active LOW) |
| `PB2` | **GIO2 (IRQ)** | Active LOW interrupt: TX complete / RX sync detected |
| `PE10` | **RF0** | TR Switch Control 0 |
| `PE11` | **RF1** | TR Switch Control 1 |

Implemented in [`src/rf/spi.rs`](../src/rf/spi.rs) and [`src/rf/a7105.rs`](../src/rf/a7105.rs).

### Antenna Diversity & TR Switching
- **TX Mode**: `PE10 = 0`, `PE11 = 1`
- **RX Mode**: `PE10 = 1`, `PE11 = 0`
- **Idle / Sleep**: `PE10 = 0`, `PE11 = 0`

---

## 2. Hopping Table Generation (FHSS)

AFHDS 2A hops across **16 unique radio frequency channels** spaced across the 2.400–2.483 GHz band. The channels are generated deterministically from the transmitter's unique 32-bit ID (derived by XOR-folding the 96-bit silicon UID at `0x1FFFF7AC`):

```rust
// Pseudo-random LCG hopping table generator
let mut seed = tx_id;
for i in 0..16 {
    seed = seed.wrapping_mul(214013).wrapping_add(2531011);
    let ch = (((seed >> 16) & 0x7FFF) % 160 + 1) as u8;
    // Ensures minimum 5-channel spacing between consecutive hops
    ...
}
```

Timing: The radio hops to the next channel in the table every **3.850 ms** ($259.74\text{ Hz}$), driven by the calibrated `TIM16` timer (`PSC = 47`, `ARR = 3849`).

---

## 3. Over-the-Air Packet Structure

All AFHDS 2A frames start with a 38-byte payload:

```
[0..3]   Transmitter ID (32-bit LE)
[4..7]   Receiver ID (32-bit LE, or 0xFFFFFFFF in Bind)
[8]      Packet Command Byte
[9..36]  Payload (Sticks, Settings, or Bind parameters)
[37]     Checksum
```

### Packet Types

| Command | Name | Description |
| :--- | :--- | :--- |
| `0x58` | **PACKET_STICKS** | Transmits 14 channels encoded as 16-bit microsecond pulse widths ($1000 \dots 2000\,\mu\text{s}$) |
| `0x56` | **PACKET_FAILSAFE** | Broadcasts failsafe pulse positions for all channels |
| `0xAA` | **PACKET_SETTINGS** | Configures receiver output modes (i-BUS, S.BUS, PWM, PPM) |
| `0xBB` | **PACKET_BIND1** | Transmits TX ID and prompts receiver for handshake |
| `0xBC` | **PACKET_BIND2** | Sends 16-channel hopping table and binds receiver ID |

---

## 4. 4-Phase Bidirectional Bind Sequence

```mermaid
sequenceDiagram
    participant TX as FS-i6X Transmitter
    participant RX as Receiver (e.g. FS-iA6B)

    Note over TX: Enter Bind Mode (Hold BIND on boot or tap BIND button)
    TX->>RX: Broadcast PACKET_BIND1 (Cmd 0xBB, RX ID = 0xFFFFFFFF)
    RX-->>TX: Reply with Receiver Unique ID
    Note over TX: Phase 2: Capture Receiver ID
    TX->>RX: PACKET_BIND2 (Cmd 0xBC, includes full 16-ch hopping table)
    RX-->>TX: Reply acknowledging hopping table
    Note over TX: Phase 4: Save RX ID to Flash & Switch to Normal Hopping Mode
```

### Cancelling Binding
- If binding is initiated accidentally or without an active receiver nearby, the bottom status bar prompts: `[ESC] Abort Bind`.
- Pressing the **Cancel / Exit** key (`ESC`, bit 11) immediately invokes `rf::set_bind_mode(false)`, restores standard power, and returns to normal flight transmission.

---

## 5. Downlink Telemetry (i-BUS Telemetry)

Immediately following transmission of each stick packet, the A7105 is switched to RX mode for a short reception window (~1.2 ms):
- **RSSI**: Signal strength percentage ($0 \dots 100\%$).
- **RX Battery Voltage**: Receiver bus voltage parsed from incoming telemetry frames, displayed as `RX: X.XXV` on the flight dashboard.
- **Lost Packet Counter**: If no valid telemetry packets are received for $> 52$ consecutive frames (~200 ms), telemetry is marked disconnected.
