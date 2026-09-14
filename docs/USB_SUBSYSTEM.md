# USB Subsystem Architecture & Operations Manual

A comprehensive technical reference and operations guide for the USB subsystem of `flysky-i6x-rs` on the FlySky FS-i6X transmitter.

---

## 1. Overview & System Philosophy

The standard FlySky FS-i6X transmitter PCB is equipped with a native USB port wired directly to the microcontroller's hardware USB Full-Speed (12 Mbps) peripheral. Previous custom firmwares (such as OpenI6X) offered USB joystick emulation, but constrained memory and monolithic codebases made flexible dual-personality support difficult.

`flysky-i6x-rs` introduces a pure `no_std` Rust USB subsystem designed with:
- **Zero Heap Allocations**: Powered by `usb-device`, `stm32-usbd`, `usbd-hid`, and `usbd-serial` using static memory structures and direct hardware packet buffers.
- **Dual-Personality Operation**:
  - **Flight Simulator Joystick (HID)**: High-rate (100 Hz), jitter-free, native 8-axis 16-button gamepad for drone and RC flight simulators.
  - **Telemetry Virtual COM Port (CDC-ACM)**: High-speed serial diagnostics streaming real-time receiver telemetry, channel pulses, and accepting interactive CLI commands.
  - **Composite Mode**: Simultaneous HID Joystick + CDC-ACM Virtual COM Port on a single USB cable.
  - **Off / Charge-Only Mode**: Completely turns off the USB transceiver and disconnects pull-up resistors to eliminate power draw while charging.
- **Safety First & Silent RF Standby**: When operating in Simulator mode, the 2.4 GHz RF power amplifier and A7105 transceiver are completely shut down (zero RF emission, extended battery life, and cool operation).
- **Hard Real-Time RF Inviolability**: The USB interrupt priority (`0xC0`) is strictly lower than the RF timing interrupts (`TIM16` at `0x80` and `EXTI2` at `0x80`), mathematically guaranteeing that USB communication never causes jitter or dropped packets on the radio link.

---

## 2. Hardware Architecture & Clocking

### Pin Assignments & Interface
The STM32F072VB microcontroller features a built-in USB 2.0 Full-Speed device controller:

| Pin | Function | Alternate Function | Notes |
| :--- | :--- | :--- | :--- |
| **`PA11`** | `USB_DM` (Data -) | `AF0` | Connected directly to Micro-USB receptacle |
| **`PA12`** | `USB_DP` (Data +) | `AF0` | Connected directly to Micro-USB receptacle |
| **Internal** | 1.5 kΩ Pull-up | Software-Controlled | Enabled via `DPPU` (bit 15) in `USB_BCDR` (`0x4000_5C58`) |

> [!NOTE]
> Unlike early STM32F103 designs which required an external 1.5 kΩ pull-up resistor or a transistor circuit on `PA12`, the STM32F072 contains an integrated 1.5 kΩ pull-up resistor directly on silicon. It is automatically engaged by firmware when USB is enabled.

### 48 MHz Clock Tree Integration
The USB Full-Speed physical layer requires a strictly regulated 48.000 MHz clock source with < 0.25% frequency tolerance:
1. **Clock Source**: External 8.000 MHz crystal (HSE) on the FS-i6X mainboard.
2. **PLL Multiplier**: `8 MHz * 6 = 48.000 MHz`.
3. **Clock Routing**: The PLL output is routed directly to the USB peripheral by asserting the `USBSW` bit (bit 7) in the `RCC_CFGR3` register (`0x4002_1030`) during `chip::init_system_clock()`:
   ```rust
   // Route PLL (48 MHz) to USB clock source (USBSW = 1 in RCC_CFGR3 bit 7)
   const RCC_CFGR3: *mut u32 = 0x4002_1030 as *mut u32;
   let cfgr3 = ptr::read_volatile(RCC_CFGR3);
   ptr::write_volatile(RCC_CFGR3, cfgr3 | (1 << 7));
   ```
4. **Packet Memory Area (PMA)**: 1024 bytes of dedicated SRAM located at `0x4000_6000`, accessed via 16-bit word accesses (`MemoryAccess::Word16x2`).

---

## 3. USB Operating Modes

The active USB mode is configured in **`Radio Setup -> USB Mode`** and saved persistently in Flash:

```
[ RADIO SETUP ]
-----------------------------
Thr Trim:        OFF (Lock)
Beeper:          ENABLED
BL Timer:        ALWAYS ON
BL Level:        100%
Contrast:        37
Bat Warn:        4.4V
USB Mode:        JOYSTICK  <-- Cycle with [OK]
-----------------------------
[OK] Toggle/Cycle   [ESC] Back
```

### Mode Comparison Matrix

| Mode | HID Gamepad | Virtual COM Port | RF Transceiver State | Primary Use Case |
| :--- | :---: | :---: | :---: | :--- |
| **`JOYSTICK`** | **Active (100 Hz)** | Inactive | **Standby / Muted (0 mW)** | Flight simulators (Liftoff, Velocidrone, RealFlight) |
| **`SERIAL`** | Inactive | **Active (115200)** | **Active (100 mW Transmitting)** | Telemetry logging, blackbox monitoring, interactive CLI |
| **`COMPOSITE`**| **Active (100 Hz)** | **Active (115200)** | **Active (100 mW Transmitting)** | Dual-purpose simulator & ground control station |
| **`OFF`** | Inactive | Inactive | **Active (100 mW Transmitting)** | Charge-only, no USB enumeration, zero PC connection |

---

## 4. USB Gamepad / Joystick (HID)

When `JOYSTICK` or `COMPOSITE` mode is enabled, the transmitter enumerates on Windows, macOS, and Linux as a standard plug-and-play **DirectInput / SDL2 Human Interface Device (HID)** without requiring any external drivers or software.

### USB Identity
- **Vendor ID (VID)**: `0x0483` (STMicroelectronics)
- **Product ID (PID)**: `0x5710` (Custom Joystick) / `0x5750` (Composite)
- **Product Name**: `FS-i6X Joystick`
- **Manufacturer**: `FlySky`
- **Serial Number**: `FS-I6X-SIM`

### Report Descriptor (8 Axes, 16 Buttons)
The HID descriptor adheres strictly to the OpenI6X and EdgeTX mapping conventions to ensure immediate, zero-configuration compatibility across all major RC simulators:

```
                +---------------------------------------+
                |    18-Byte USB HID Report Format      |
                +---------------------------------------+
                | Byte 0..1   | Axis X: Roll (AIL)      |
                | Byte 2..3   | Axis Y: Pitch (ELE)     |
                | Byte 4..5   | Axis Z: Throttle (THR)  |
                | Byte 6..7   | Axis Rz: Yaw (RUD)      |
                | Byte 8..9   | Axis Rx: Dial VRA (VR1) |
                | Byte 10..11 | Axis Ry: Dial VRB (VR2) |
                | Byte 12..13 | Slider: Aux CH7         |
                | Byte 14..15 | Dial: Aux CH8           |
                | Byte 16..17 | 16 Digital Buttons      |
                +---------------------------------------+
```

#### Axis Normalization
All stick and potentiometer channels are calculated through the flight mixer pipeline (incorporating calibration, dual rates, expos, and trims), clamped to 1000..2000 µs, and linearly scaled to 16-bit signed USB axis units:
$$ \text{USB Axis} = \left( \frac{\text{Pulse}_{\mu s} - 1000}{1000} \right) \times 65535 - 32768 $$

#### Button Mapping (Switches SA..SD)
Physical switches are mapped to digital buttons to support simulator functions such as Flight Modes, Arming, Turtle Mode, and Reset:

| Button | Source | Condition |
| :---: | :--- | :--- |
| **Button 1** | Switch SA | Down position |
| **Button 2** | Switch SA | Up position |
| **Button 3** | Switch SB | Up position |
| **Button 4** | Switch SB | Middle position |
| **Button 5** | Switch SB | Down position |
| **Button 6** | Switch SC | Up position |
| **Button 7** | Switch SC | Middle position |
| **Button 8** | Switch SC | Down position |
| **Button 9** | Switch SD | Down position |
| **Button 10** | Switch SD | Up position |
| **Buttons 11..16** | Aux Channels | Spare / Reserved |

### Tested Flight Simulators
The native gamepad mode has been verified with:
- **Liftoff: FPV Drone Racing** (Steam / PC / Mac)
- **VelociDrone FPV Racing Simulator**
- **RealFlight Evolution / RF9**
- **FPV Freerider / FPV Freerider Recharged**
- **Uncrashed: FPV Drone Simulator**
- **DRL Simulator (Drone Racing League)**
- **AccuRC 2 Precision Flight Simulator**

---

## 5. Virtual COM Port (CDC-ACM) & Interactive CLI

When `SERIAL` or `COMPOSITE` mode is selected, the transmitter exposes a standard virtual serial port (`/dev/ttyACM0` on Linux, `COMx` on Windows).

### Connection Parameters
- **Baud Rate**: 115,200 bps (Virtual CDC-ACM transfers at full 12 Mbps USB speed regardless of baud setting).
- **Data Bits**: 8
- **Parity**: None (N)
- **Stop Bits**: 1
- **Flow Control**: None

### Autonomous Telemetry Streaming
Every 50 ms (20 Hz), the transmitter automatically broadcasts an ASCII telemetry packet formatted for easy parsing by ground control software or logging scripts:
```
TELEM: VBAT=5180mV RSSI=98% RX_V=4980mV TX_PKT=15820 RX_PKT=15798 ERR=22
```

### Interactive CLI Commands
Users can open a serial terminal (PuTTY, Tera Term, Minicom, or screen) to interact directly with the radio:

```bash
$ picocom -b 115200 /dev/ttyACM0
```

| Command | Description | Example Output |
| :--- | :--- | :--- |
| **`help`** | Displays available serial CLI commands | `Commands: help, status, channels, telemetry, reboot` |
| **`status`** | System health, voltage, active model, and link state | `STATUS: Model=0 (MODEL 01), Vbat=5.18V, RF=Active, Telem=Connected` |
| **`channels`** | Real-time microsecond pulse widths for CH1..CH14 | `CH: 1500 1500 1150 1500 1000 1500 1500 1500 1000 1000 1500 1500 1500 1500` |
| **`telemetry`** | Full downlink sensor metrics and packet odometer | `TELEM: RSSI=95%, RxBatt=5.02V, Errors=4, LinkQuality=99%` |
| **`reboot`** | Safely triggers a software system reset | `Rebooting transmitter...` |

---

## 6. Safety & Silent RF Standby

### Zero-RF Simulator Mode
Operating an RF transmitter indoors in close proximity to a computer monitor, router, and user's body is undesirable:
1. It creates unnecessary 2.4 GHz spectrum pollution.
2. It draws significant battery current (~80–120 mA) through the RF power amplifier.
3. The internal RF module produces waste heat over long simulator sessions.

When **`JOYSTICK`** mode is active and the USB cable is connected:
- The A7105 transceiver is immediately strobed into **`STROBE_STANDBY`** (`0xA0`).
- The antenna front-end switches are set to **`RF_MODE_OFF`** (`0b0000_0011`), turning off both the power amplifier (PA) and low-noise amplifier (LNA).
- The `TIM16` packet transmission interrupt handler returns immediately without pulsing the radio or modifying the A7105 FIFO.
- The flight screen replaces the RF indicator with **`U:SIM`**.
- As soon as the USB cable is unplugged or the mode is switched to `SERIAL`, normal RF transmission automatically resumes.

### Deterministic Real-Time Concurrency
In embedded safety-critical avionics, flight-control tasks must never suffer priority inversion or starvation from peripheral I/O:
- **`TIM16` RF Interrupt (Priority 0x80)**: Fires every 3.850 ms (260 Hz) to transmit AFHDS 2A packets. Highest peripheral priority.
- **`EXTI2` RF Packet Ready (Priority 0x80)**: Handles A7105 TX/RX FIFO events.
- **`SysTick` Monotonic Timer (Priority 0x40)**: Drives system timekeeping.
- **`USB` Full-Speed Interrupt (Priority 0xC0)**: Lowest peripheral priority.

If a large USB transfer or host polling stall occurs, the hardware NVIC preempts the USB routine instantaneously to execute the RF transmission. **The radio link is 100% mathematically protected.**

---

## 7. Storage Compatibility & Configuration

To guarantee backwards compatibility with existing users' saved models, all additions are strictly size-invariant:

### `RadioConfig` Binary Layout (Strictly 128 Bytes)
```
Offset  0..4   : Magic (0x46534B59 "FSKY")
Offset  4..8   : Version (CONFIG_VERSION = 4)
Offset  8      : Active Model (0..19)
Offset  9      : Throttle Trim Method
Offset 10      : Audio Enabled
Offset 11      : Backlight Timeout
Offset 12      : Backlight Brightness
Offset 13      : Vbat Warn Threshold
Offset 14      : LCD Contrast
Offset 15      : usb_mode (0: Joystick, 1: Serial, 2: Composite, 3: Off) <-- REPLACED _pad0
Offset 16..48  : Stick Calibrations (4 x 8B)
Offset 48..64  : Pot Calibrations (2 x 8B)
Offset 64..128 : Reserved (64B)
```

### Protocol Selection (`ModelConfig`)
In Menu Item 9 (**`Protocol Setup`**), the redundant bind menu was upgraded to support per-model protocol selection:
- **`AFHDS 2A`**: Built-in A7105 transceiver. Pressing `[OK]` triggers receiver binding.
- **`CRSF / ELRS`**: Prepares the radio for external Crossfire / ExpressLRS transmitters attached via the rear expansion bay (`PD5` TX / `PA15` RX @ 416,666 bps 8N1).
