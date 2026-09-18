# DFPlayer Mini Voice Audio & Sound Pack Hardware Guide

This document details the hardware modification and MicroSD card sound pack setup for the **FlySky FS-i6X** running `flysky-i6x-rs` firmware.

---

## 1. Overview & Pin Mapping

The DFPlayer Mini provides voice announcements (warnings, flight mode switches, arm/disarm status, telemetry alarms, timers, and calibration feedback) while preserving full compatibility with the existing piezo buzzer.

| Signal | DFPlayer Mini Pin | FlySky i6X Mainboard Connection | STM32F072 Pin | Function |
| :--- | :--- | :--- | :--- | :--- |
| **TX Data** | **RX** (Pin 2) | **`AUX3`** solder pad (via **1 kΩ resistor**) | `PC10` | Alternate Function 1 (`USART3_TX`), 9600 bps 8N1 |
| **Status** | **BUSY** (Pin 16) | **`PC14`** test pad / via | `PC14` | Active-LOW status input (Pull-up: LOW = Playing, HIGH = Idle) |
| **Power** | **VCC** (Pin 1) | **5V** switched rail (or 5V step-up/regulator) | — | 3.3V - 5.0V DC (5.0V recommended for audio power) |
| **Ground** | **GND** (Pin 7 / 10) | **GND** ground plane / pad | — | System Ground |
| **Speaker** | **SPK_1** / **SPK_2** | 8 Ω 1 W - 2 W dynamic speaker | — | Differential speaker output |

> [!IMPORTANT]
> **Noise Filtering**: Always place a **1 kΩ resistor** in series between `AUX3` (`PC10`) and the DFPlayer's `RX` pin. This suppresses high-frequency serial reflections and audio amplifier ground hum.

---

## 2. Zero Peripheral Conflicts

The DFPlayer Mini integration has zero overlap with any existing hardware features:
- **Internal RF (A7105)**: Dedicated to `SPI1` (`PE12`..`PE15`) and `TIM16`.
- **External CRSF / ExpressLRS**: Dedicated to `USART2` (`PD5` TX, `PA15` RX, `PC13` Power Switch).
- **USB Controller**: Dedicated to hardware `USB` peripheral (`PA11`/`PA12`).
- **Piezo Buzzer**: Dedicated to `TIM1_CH1` (`PA8`).
- **DFPlayer Mini Voice**: Dedicated solely to **`USART3` (`PC10`)** and **`PC14`**.

---

## 3. MicroSD Card Sound Pack Setup

The DFPlayer Mini module contains its own on-board MicroSD slot.

### Formatting
- Use a MicroSD card (FAT16 or FAT32 formatted, 32 GB or smaller).
- Create sound files encoded as standard 16-bit 44.1 kHz or 32 kHz MP3 or WAV.

### File Naming and Index
Sound tracks must be named with 4-digit numbers in the root directory (or inside a folder named `/MP3/`):

| Track Number | File Name | Event Trigger | Description |
| :---: | :--- | :--- | :--- |
| **1** | `0001.mp3` | **Welcome** | Radio power-on greeting |
| **2** | `0002.mp3` | **Armed** | Switch SA flipped to armed position |
| **3** | `0003.mp3` | **Disarmed** | Switch SA flipped to disarmed position |
| **4** | `0004.mp3` | **Low Battery** | Radio battery voltage falls below alarm threshold |
| **5** | `0005.mp3` | **Critical Battery** | Radio battery critically low (< 4.1 V) |
| **6** | `0006.mp3` | **RSSI Low** | Receiver downlink telemetry RSSI < 40% |
| **7** | `0007.mp3` | **RSSI Critical** | Receiver downlink telemetry RSSI < 20% |
| **8** | `0008.mp3` | **Pre-flight Warning** | Startup safety alarm (throttle raised or switches armed) |
| **9** | `0009.mp3` | **Inactivity Alert** | 10 minutes without physical control movement |
| **10** | `0010.mp3` | **Failsafe** | Telemetry / RF link lost |
| **11** | `0011.mp3` | **Acro Mode** | Switch SC toggled to Acro |
| **12** | `0012.mp3` | **Angle Mode** | Switch SC toggled to Angle / Stabilized |
| **13** | `0013.mp3` | **Horizon Mode** | Switch SC toggled to Horizon |
| **14** | `0014.mp3` | **Return To Home** | Return-to-home mode active |
| **15** | `0015.mp3` | **Manual Mode** | Direct pass-through manual control |
| **16** | `0016.mp3` | **Position Hold** | GPS position hold active |
| **17** | `0017.mp3` | **Timer 1 Minute** | Countdown timer at 1:00 |
| **18** | `0018.mp3` | **Timer 30 Seconds** | Countdown timer at 0:30 |
| **19** | `0019.mp3` | **Timer 10 Seconds** | Countdown timer at 0:10 |
| **20** | `0020.mp3` | **Timer Elapsed** | Countdown timer reached 0:00 |
| **21** | `0021.mp3` | **Trim Center** | Any trim rocker hits mechanical center (0) |
| **22** | `0022.mp3` | **Trim Limit** | Any trim reaches minimum (-25) or maximum (+25) |
| **23** | `0023.mp3` | **Calibration Start** | Analog stick calibration wizard started |
| **24** | `0024.mp3` | **Calibration Success** | Analog stick calibration saved to Flash |

---

## 4. Software Configuration

In the transmitter settings menu:
1. Long-press **`[OK]`** from the main flight screen to enter settings.
2. Select **`8. Radio Setup`**.
3. Configure the audio options:
   - **`Audio Dev`**:
     - `BUZZER`: Default stock behavior (Piezo buzzer only).
     - `VOICE`: Voice announcements via DFPlayer Mini.
     - `BOTH`: Concurrent voice announcements and buzzer tones.
   - **`Voice Vol`**: `0..30` (Adjustable in steps of 5, default 20/30).
4. Press **`[ESC]`** to save to Flash memory.
