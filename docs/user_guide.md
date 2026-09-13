# FlySky FS-i6X User Guide & Operations Manual

A comprehensive guide to operating the `flysky-i6x-rs` firmware on the FlySky FS-i6X transmitter.

---

## 1. Physical Controls & Faceplate Layout

```
                  +-----------------------------------+
                  |         FlySky FS-i6X (Rust)      |
                  |                                   |
                  |  [SW A]   [SW B]  [SW C]   [SW D] |
                  |  (2-pos) (3-pos) (3-pos)  (2-pos) |
                  |                                   |
                  |       (VRA/VR1)     (VRB/VR2)     |
                  |       Left Dial     Right Dial    |
                  |                                   |
                  |    [LEFT GIMBAL]   [RIGHT GIMBAL] |
                  |    Throttle (V)    Pitch / ELE (V)|
                  |    Yaw / RUD (H)   Roll / AIL (H) |
                  |                                   |
                  |     [T-TRIM]          [E-TRIM]    |
                  |     [R-TRIM]          [A-TRIM]    |
                  |                                   |
                  |            [ ST7567 LCD ]         |
                  |              128x64 Mono          |
                  |                                   |
                  |  [BIND]    [UP]    [DOWN]    [OK] |
                  |  (Direct)          [CANCEL / ESC] |
                  +-----------------------------------+
```

### Keypad & Navigation Buttons
- **`[UP]`** / **`[DOWN]`**: Navigate menu items, cycle characters, increment/decrement values.
- **`[OK]`**: Enter submenu, toggle setting, advance character cursor in naming editor.
  - **Hold `[OK]` for 1.2 seconds** on the main flight screen: Opens the **Settings Menu**.
  - **Hold `[OK]` during Power-On**: Launches **Stick Calibration** immediately.
- **`[CANCEL]` (`[ESC]`)**: Return to previous screen, abort calibration, or complete one-way receiver binding.
- **`[BIND]` (Dedicated Button)**: Tap at any time on the main screen to initiate AFHDS 2A binding.

---

## 2. Flight Dashboard Display

```
+-------------------------------------------------------------+
| M01                 R: 98%                          5.2V    | <- Status Bar
|-------------------------------------------------------------|
|  CH1 [   |   .   ]   1520us  |  CH3 [   . |      ]   1240us |
|  CH2 [       | . ]   1680us  |  CH4 [     . |    ]   1490us |
|-------------------------------------------------------------|
| A:U  B:M  C:D  D:U                             V: 5/ 8      | <- Switches & Pots
| TRM A:+04                                                   | <- Bottom Diagnostic
+-------------------------------------------------------------+
```

### Display Elements
1. **Top Status Bar (y = 0..10)**:
   - **Left (`M01`..`M20`)**: Active model memory slot.
   - **Center (`R: XX%` / `BINDING` / `E:XX`)**:
     - `R: XX%`: Telemetry downlink RSSI (Signal Strength $0\% \dots 100\%$).
     - `BINDING`: Transmitter is broadcasting bind frames.
     - `E:XX`: RF transceiver initialization error code (e.g. `E:00` indicates A7105 communication failure).
   - **Right (`X.XV`)**: Internal 4×AA battery pack voltage.
2. **Main Gimbal Gauges (y = 12..46)**:
   - Live visual sliders for CH1 (Roll), CH2 (Pitch), CH3 (Throttle), CH4 (Yaw).
   - Solid vertical bar indicates current stick deflection.
   - **Dotted tick mark (`.`)** indicates the active digital trim offset.
   - Real-time pulse width readout in microseconds ($1000 \dots 2000\,\mu\text{s}$).
3. **Switches & Pots Line (y = 48..54)**:
   - **Switches (`A:U B:M C:D D:U`)**: Real-time position of switches SA through SD (`U`=Up, `M`=Middle, `D`=Down).
   - **Pots (`V: X/ Y`)**: Position of rotary dials VRA (`X`) and VRB (`Y`) scaled $0 \dots 9$.
4. **Bottom Banner (y = 56..63)**:
   - Displays momentary trim feedback (e.g. `TRM A:+04`) when any trim switch is pressed.
   - Prompts for shortcuts (e.g. `Hold OK:Menu`, `[ESC] Finish Bind`).

---

## 3. Digital Trims & Audio Feedback

All 4 primary axes have dedicated digital rocker switches providing $\pm 25$ steps of trim authority ($\pm 100\,\mu\text{s}$):

### Trim Operation & Tones
- **Single Press**: Nudges trim by 1 step ($4\,\mu\text{s}$). A short tone sounds.
- **Pitch Shift**: Tone pitch rises as trim increases ($1500 \dots 2500\text{ Hz}$), giving instant acoustic feedback of direction.
- **Center Return**: When passing through `0` (neutral), a distinctive high-pitched double-length tone (`2800 Hz`) sounds.
- **End of Travel**: Attempting to move past $\pm 25$ sounds a low-frequency warning buzz (`1100 Hz`).
- **Auto-Repeat**: Holding any trim switch for $\ge 350\text{ ms}$ automatically repeats steps at 90 ms intervals.

### Throttle Trim Safety Modes
Configurable in `Radio Setup`:
1. **`OFF (Lock)` (Recommended for Betaflight / INAV / Multirotors)**:
   - Throttle trim buttons are locked out. Pressing them sounds a warning tone without modifying output.
   - Prevents accidental disarm failures or motor spin-ups caused by bumping the throttle trim.
2. **`IDLE` (Traditional Glow / Nitro Engines)**:
   - Throttle trim only affects the lower half of the throttle stick range ($1000 \dots 1500\,\mu\text{s}$), leaving maximum full-throttle output unchanged at $2000\,\mu\text{s}$.
3. **`LINEAR` (Electric Aircraft)**:
   - Throttle trim shifts the entire $1000 \dots 2000\,\mu\text{s}$ range symmetrically.

---

## 4. Menu Navigation & Subsystem Breakdown

Hold **`[OK]` for 1.2 seconds** from the main flight screen to open the Settings Menu.

```
+------------------------------------+
| SETTINGS MENU                      |
|------------------------------------|
| > 1. MODEL SELECT                  |
|   2. MODEL SETUP                   |
|   3. CH REVERSE                    |
|   4. THR CURVE                     |
|   5. RADIO SETUP                   |
|   6. STICK CALIB                   |
|   7. RX SETUP & BIND               |
|   8. CHANNEL MONITOR               |
|   9. DIAG ANAS                     |
|  10. SYSTEM INFO                   |
|------------------------------------|
| [UP/DN] Move  [OK] Sel  [ESC] Exit |
+------------------------------------+
```

### Submenu 1: Model Select (`MODEL SELECT`)
- Displays all 20 model memory slots (`M01` through `M20`).
- The currently loaded model is marked with `[*]`.
- Use **`[UP]`** / **`[DOWN]`** to scroll through models; press **`[OK]`** to activate.
- **Real-Time Switching**: Switching models immediately applies the selected model's trims, channel reversing mask, throttle curve, and receiver ID.

### Submenu 2: Model Setup (`MODEL SETUP`)
- **Name Editor**: 10-character ASCII model name (e.g. `QUAD 5IN  `, `TRAINER   `, `FOAMY 3D `).
  - Use **`[UP]`** / **`[DOWN]`** to cycle through characters (`A-Z`, `0-9`, `-`, `_`, space).
  - Press **`[OK]`** to advance to the next character.
- **Model Type**: Cycle between `AIRPLANE`, `HELI`, `MULTIROTOR`, and `GLIDER`.
- **Reset Model**: Restores default trims, linear curves, and standard channel directions for this model.

### Submenu 3: Channel Reverse (`CH REVERSE`)
- Lists all 14 channels (CH1:ROL, CH2:PIT, CH3:THR, CH4:YAW, SwA..SwD, VR1, VR2).
- Press **`[OK]`** to toggle between **`NOR`** (Normal) and **`REV`** (Reversed).
- Calculations use hardware-standard inversion: $\text{pulse} = 3000 - \text{pulse}$.
- Automatically saved to non-volatile Flash upon exit.

### Submenu 4: Throttle Curve Editor (`THR CURVE`)
Interactive curve engine with real-time on-screen curve visualization ($44 \times 36$ pixel plot):

```
+------------------------------------+
| THROTTLE CURVE                     |
|------------------------------------+
| > Pts: 5 POINTS    +--------------+|
|   Crv: SMOOTH      |     .---*    ||
|   P1:  0%          |    /         ||
|   P2: 25%          |   /          ||
|   P3: 50%          |  *           ||
|   P4: 75%          | *            ||
|   P5:100%          +--------------+|
|------------------------------------+
| [UP/DN] Value  [OK] Next  [ESC] End|
+------------------------------------+
```

- **`Pts:`**: Toggle between **`5 POINTS`** ($0\%, 25\%, 50\%, 75\%, 100\%$) and **`9 POINTS`** ($0\%, 12.5\%, \dots, 100\%$).
- **`Crv:`**: Toggle between **`LINEAR`** (piecewise linear interpolation) and **`SMOOTH`** (**Catmull-Rom cubic Hermite spline** smoothing).
- **Point Values (`P1` .. `P9`)**:
  - Select any point and use **`[UP]`** / **`[DOWN]`** to adjust from $0\%$ to $100\%$.
  - The live graph instantly reflects changes, drawing a continuous curve line and placing a marker at the current physical throttle stick position.

### Submenu 5: Radio Setup (`RADIO SETUP`)
- **`Thr Trim:`**: Toggle between `OFF (Lock)`, `IDLE`, and `LINEAR`.
- **`Audio:`**: Toggle beeper sound between `ENABLED` and `MUTED`.
- **`BL Timer:`**: LCD backlight auto-shutoff timeout: `ALWAYS ON`, `15 SEC`, `30 SEC`, or `60 SEC`. Touching any key or moving any stick wakes the backlight instantly.
- **`BL Level:`**: Backlight brightness level from `10%` to `100%` in 10% steps (supports both stock transistors and the `PC9` hardware PWM dimming mod).

### Submenu 6: Stick Calibration (`STICK CALIB`)
Launches the interactive 2-step calibration wizard (see Section 5 below).

### Submenu 7: RX Setup & Bind (`RX SETUP & BIND`)
- Displays current RF protocol (`AFHDS 2A`).
- Displays active model index and bound receiver ID (e.g. `Bound RX: 0x2A3B4C5D`).
- Press **`[OK]`** to enter binding mode.

### Submenu 8: Channel Monitor (`CHANNEL MONITOR`)
- Displays live pulse widths ($1000 \dots 2000\,\mu\text{s}$) across all 14 channels with horizontal graphic bar indicators.
- Press **`[OK]`** to toggle between Page 1 (CH1..CH7) and Page 2 (CH8..CH14).

### Submenu 9: Analog Diagnostics (`DIAG ANAS`)
- Displays raw 12-bit ADC counts ($0 \dots 4095$) for all 11 physical inputs in real time:
  - Gimbals: Roll, Pitch, Throttle, Yaw
  - Dials: VRA, VRB
  - Switches: SA, SB, SC, SD
  - Power: Battery sensing divider

### Submenu 10: System Information (`SYSTEM INFO`)
- Displays MCU type (`STM32F072VB` or `APM32F072VB`), 96-bit silicon UID, firmware version, Flash memory map, and storage statistics.

---

## 5. Gimbal & Potentiometer Calibration Procedure

Calibration ensures gimbals reach full travel without clipping or deadzones:

1. **Enter Calibration**:
   - Hold **`[OK]` for 1.2s** on the flight screen $\rightarrow$ select `STICK CALIB`.
   - Alternatively, **hold `[OK]` while switching on the radio**.
2. **Step 1: Center Position**:
   - Let Roll, Pitch, and Yaw return to center springs.
   - Move Throttle (friction stick) to the physical middle (50%).
   - Center rotary dials VRA and VRB.
   - Press **`[OK]`** to capture neutral centers.
3. **Step 2: Limit Travel**:
   - Move both gimbals in wide circular motions touching all four corners.
   - Rotate VRA and VRB back and forth across their full rotation.
   - Watch the on-screen gauges fill out. Once an axis has measured sufficient travel, its indicator changes from `--` to `OK`.
   - When all axes display `OK`, press **`[OK]`** to save.
4. **Completion**:
   - The radio applies OpenTX-standard ~1.6% margin tolerances, plays a 2-tone success chime, and saves calibrations to Flash.

---

## 6. Receiver Binding Instructions

### Method A: Two-Way Telemetry Receivers (FS-iA6B, FS-iA10B)
1. Power on the receiver with a bind plug inserted into the `B/VCC` port (LED flashes rapidly).
2. Tap the dedicated **`[BIND]`** button on the transmitter faceplate (or select `RX SETUP & BIND` in the menu).
3. The transmitter broadcasts bind packets and prompts `BINDING`.
4. Once the receiver receives the hopping table, it sends its unique ID back.
5. The transmitter automatically captures the ID, plays a rising 2-tone chime, saves to the active model profile in Flash, and switches to normal transmission. The top status bar shows `RF:OK` and live RSSI.
6. Remove the bind plug and power cycle the receiver.

### Method B: One-Way Receivers (FS-A8S, Fli14, FS-iA6)
1. Hold the receiver's bind button while powering on (LED flashes rapidly).
2. Tap **`[BIND]`** on the transmitter faceplate.
3. Once the receiver's LED turns solid (indicating it has locked onto the transmitter's frequency hopping table), press **`[CANCEL]` (`[ESC]`)** or tap **`[BIND]`** again.
4. The transmitter saves the bind state to Flash, double-beeps, and begins transmitting regular channel data.

---

## 7. Firmware Flashing & DFU Recovery

The firmware binary can be flashed via USB without specialized hardware programmer probes:

1. **Enter DFU Bootloader**:
   - Hold **Roll Left + Yaw Right** inward towards the power switch while switching on the radio.
   - The screen remains black, and the transmitter enumerates over USB as `0483:df11` (STM32 BOOTLOADER).
2. **Flash Binary**:
   ```bash
   dfu-util -a0 -s 0x08000000:leave -d 0483:df11 -D target/flysky-i6x-rs.bin
   ```
3. The radio will immediately reboot into the new firmware upon completion.
