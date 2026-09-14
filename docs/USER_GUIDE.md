# FLYSKY FS-I6X USER GUIDE & OPERATIONS MANUAL

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
  - **Auto-Repeat**: Holding **`[UP]`** or **`[DOWN]`** for $\ge 300\text{ ms}$ automatically repeats every **70 ms** for rapid scrolling through lists, swift character selection, and fast curve point editing.
- **`[OK]`**: Enter submenu, toggle setting, confirm values, advance character cursor in naming editor.
  - **Hold `[OK]` for 1.2 seconds** on any flight dashboard page: Opens the **Settings Menu**.
  - **Hold `[OK]` during Power-On**: Launches **Stick Calibration** immediately.
- **`[CANCEL]` (`[ESC]`)**: Return to previous screen, exit edit mode, abort calibration, or complete one-way receiver binding.
- **`[BIND]` (Dedicated Button with Clean Separation Logic)**:
  - **Tap (`< 1.0s`) on Flight Screen**: Cycles through the 3 flight dashboard pages (`1/3` $\rightarrow$ `2/3` $\rightarrow$ `3/3` $\rightarrow$ `1/3`).
  - **Hold (`\ge 1.0s`) on Flight Screen**: Initiates AFHDS 2A receiver binding.
  - **Hold during Power-On**: Launches AFHDS 2A binding subprogram immediately at boot.
  - **In Menus & Editors**: Functions as **`[TAB]` / Cursor Advance** (advances name characters or curve points) without triggering RF binding.

---

## 2. Multi-Page Flight Dashboard

The main flight screen features 3 switchable display pages cycled by tapping **`[BIND]`**. All 3 pages share a pixel-perfect uniform layout:
- **Top Status Bar (`y = 0..10`)**: Displays active model name, RF/telemetry status, and steady filtered battery voltage (`X.YYV`).
- **Top Divider (`y = 11`)**: Full-width horizontal line (`Line(0, 11) -> (127, 11)`).
- **Content Area (`y = 12..54`)**: Page-specific controls, gauges, and telemetry.
- **Bottom Divider (`y = 55`)**: Full-width horizontal line (`Line(0, 55) -> (127, 55)`).
- **Footer Info Bar (`y = 56..63`)**: Small text (`FONT_4X6`) rendered at baseline 62 (`y = 57..62`) for maximum vertical clearance.

### Page 1/3: Primary Gimbals & Trims
```
+-------------------------------------------------------------+
| MODEL 01                  RF:OK                     5.18V   | <- Status Bar (y=0..10)
|-------------------------------------------------------------| <- Top Line (y=11)
| A [====|==.======]  +15%   | E [========.=|==]   -22%       |
| T [========.     ]   45%   | R [====|==.======]    0%       |
| A:U  B:M  C:D  D:U                             V: 5/ 8      | <- Switches & Pots (y=44..53)
|-------------------------------------------------------------| <- Bottom Line (y=55)
| P1/3                                      Hold OK:Menu      | <- Footer Bar (y=57..62)
+-------------------------------------------------------------+
```
- **Top Status Bar (y = 0..10)**:
  - **Left**: Active model name (up to 10 characters, e.g. `MODEL 01`).
  - **Center (`RF:OK` / `R: XX%` / `BIND` / `NO RF` / `E:XX`)**: RF link state, binding status, or downlink telemetry RSSI.
  - **Right (`X.YYV`)**: Internal battery voltage stabilized by an exponential moving average (EMA) filter to eliminate switching jitter on the hundredths digit.
- **Gimbal Gauges (y = 12..43)**: Live channel sliders for Roll (`A`), Pitch (`E`), Throttle (`T`), Yaw (`R`) with center ticks, trim position ticks (`.`), and percentage readouts.
- **Switches & Pots Line (y = 44..53)**: Position of switches SA..SD (`U`=Up, `M`=Middle, `D`=Down) and rotary pots VRA/VRB (`0`..`9`), positioned cleanly above the line 55 divider.
- **Bottom Footer (y = 57..62)**: Displays active trim adjustment (`TRM A:+04`) or `P1/3   Hold OK:Menu` in crisp small font (`FONT_4X6`).

### Page 2/3: 14-Channel Dual Column Monitor
```
+-------------------------------------------------------------+
| MODEL 01                  RF:OK                     5.18V   | <- Status Bar (y=0..10)
|-------------------------------------------------------------| <- Top Line (y=11)
|  1: [==========] 1500  |   8: [==========] 1500             |
|  2: [==========] 1500  |   9: [==========] 1500             |
|  3: [====      ] 1200  |  10: [==========] 1500             |
|  4: [==========] 1500  |  11: [==========] 1500             |
|  5: [          ] 1000  |  12: [==========] 1500             |
|  6: [==========] 1500  |  13: [==========] 1500             |
|  7: [==========] 1500  |  14: [==========] 1500             |
|-------------------------------------------------------------| <- Bottom Line (y=55)
| P2/3                           14-CH MONITOR                | <- Footer Bar (y=57..62)
+-------------------------------------------------------------+
```
- Real-time graphic bars and microsecond pulse readouts ($1000 \dots 2000\,\mu\text{s}$) across all 14 AFHDS 2A channels simultaneously.
- Left column: CH 1..7 (Gimbals, SwA, SwB, VR1).
- Right column: CH 8..14 (VR2, SwC, SwD, Aux channels).
- Graphic bars are positioned at `y + 1` for pixel-perfect horizontal centering with the text labels, leaving 2px clearance above the line 55 divider.

### Page 3/3: Model & Telemetry Dashboard
```
+-------------------------------------------------------------+
| MODEL 01                  RF:OK                     5.18V   | <- Status Bar (y=0..10)
|-------------------------------------------------------------| <- Top Line (y=11)
| MODEL 01                                   AIRPLANE         |
| RxID: 1A2B3C4D                                              |
| TCrv: 9-PT                                 SMOOTH           |
| RX: 5.12V                                  RSSI: 98%        |
|-------------------------------------------------------------| <- Bottom Line (y=55)
| P3/3                         MODEL DASHBOARD                | <- Footer Bar (y=57..62)
+-------------------------------------------------------------+
```
- Full 10-character model name and synchronized model type (`AIRPLANE`, `GLIDER`, `HELI`, `QUAD`).
- Bound receiver 32-bit hex ID (`RxID`).
- Active throttle curve configuration (`5-PT` / `9-PT`, `LINEAR` / `SMOOTH`).
- Live telemetry readouts: Downlink RSSI percentage and receiver pack voltage (`RX: X.XXV`), or `AFHDS2A: DISCONNECTED` fitted within the screen width.

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
- **Field 0: Name Editor**: 10-character ASCII model name (e.g. `QUAD 5IN  `, `TRAINER   `, `FOAMY 3D `).
  - Press **`[OK]`** to enter editing mode.
  - Use **`[UP]`** / **`[DOWN]`** to cycle characters (`A-Z`, `0-9`, `-`, `_`, space).
  - Press **`[OK]`** or **`[BIND]`** to confirm current character and advance cursor to next character.
  - Advancing past character 10 confirms the entire name and moves focus to Field 1 (`Type`).
  - Press **`[CANCEL]` (`[ESC]`)** at any time to finish editing name and return to field selection.
- **Field 1: Model Type**:
  - Press **`[OK]`** to cycle between `AIRPLANE`, `GLIDER`, `HELI`, and `QUAD`.
- **Field 2: Bind RX**:
  - Displays currently bound receiver ID (`Rx: XXXXXXXX`).
  - Press **`[OK]`** on `[OK Bind]` to initiate AFHDS 2A receiver binding directly from Model Setup.
- **Field 3: Reset Defaults**:
  - Press **`[OK]`** on `[OK Defaults]` to restore default trims, standard channel directions, and linear curves for this model slot.

### Submenu 3: Channel Reverse (`CH REVERSE`)
- Lists all 14 channels (CH1:ROL, CH2:PIT, CH3:THR, CH4:YAW, SwA..SwD, VR1, VR2).
- Press **`[OK]`** to toggle between **`NOR`** (Normal) and **`REV`** (Reversed).
- Calculations use hardware-standard inversion: $\text{pulse} = 3000 - \text{pulse}$.
- Automatically saved to non-volatile Flash upon exit.

### Submenu 4: Throttle Curve Editor (`THR CURVE`)
Interactive curve engine with real-time on-screen curve visualization ($49 \times 37$ pixel plot) and selected-point indicator dot:

```
+------------------------------------+
| THROTTLE CURVE                     |
|------------------------------------+
|   Pts: 9-PT        +--------------+|
|   Crv: SMOOTH      |     .---*    ||
| > P3: 50% <        |    /         ||
|                    |   /  *       ||
|                    |  /           ||
|                    +--------------+|
|------------------------------------+
| [OK] Next Pt   [ESC] Done          |
+------------------------------------+
```

- **Field 0 (`Pts:`)**: Toggle between **`5-PT`** and **`9-PT`**. The UI dynamically updates the point range indicator (`Pts: 1..5` in 5-point mode, `Pts: 1..9` in 9-point mode). Switching from 5-point to 9-point mode automatically **resamples** midpoint values between existing points (e.g. `[0, 25, 50, 75, 100]` $\rightarrow$ `[0, 12, 25, 37, 50, 62, 75, 87, 100]`), preventing flat-zero dropoffs.
- **Field 1 (`Crv:`)**: Toggle between **`LINEAR`** (piecewise linear interpolation) and **`SMOOTH`** (**Catmull-Rom cubic Hermite spline** smoothing).
- **Field 2.. (`P1` .. `Pn`)**:
  - While navigating (`!editing`), scroll with **`[UP]`** / **`[DOWN]`** and press **`[OK]`** to enter point-editing mode (`> Pn: XX% <`).
  - While editing:
    - **`[UP]`** / **`[DOWN]`**: Adjust point value between $0\%$ and $100\%$ (with auto-repeat when held).
    - **`[OK]`**: Confirm current point and advance to next point (`P1` $\rightarrow$ `P2` $\rightarrow$ `...`). On the last point, exits edit mode.
    - **`[BIND]`**: Tabs to the next point (wraps to `P1`).
    - **`[CANCEL]` (`[ESC]`)**: Exits point-editing mode.
  - A real-time $3 \times 3$ pixel dot indicator is plotted directly on the curve graph at the coordinates of the actively selected point.

### Submenu 5: Radio Setup (`RADIO SETUP`)
- **`Thr Trim:`**: Toggle between `OFF (Lock)`, `IDLE`, and `LINEAR`.
- **`Audio:`**: Toggle beeper sound between `ENABLED` and `MUTED`.
- **`BL Timer:`**: LCD backlight auto-shutoff timeout: `ALWAYS ON`, `15 SEC`, `30 SEC`, or `60 SEC`. Touching any key or moving any stick wakes the backlight instantly.
- **`BL Level:`**: Backlight brightness level from `10%` to `100%` in 10% steps (supports both stock transistors and the `PC9` hardware PWM dimming mod).

### Submenu 6: Stick Calibration (`STICK CALIB`)
Launches the interactive 2-step calibration wizard (see Section 5 below).

### Submenu 7: RX Setup & Bind (`RX SETUP & BIND`)
- Displays current RF protocol (`AFHDS 2A`).
- Displays active model index and bound receiver ID (e.g. `Rx ID: 1A2B3C4D`).
- Press **`[OK]`** to trigger receiver binding mode directly.

### Submenu 8: Channel Monitor (`CHANNEL MONITOR`)
- Displays live pulse widths ($1000 \dots 2000\,\mu\text{s}$) across all 14 channels with 40-pixel horizontal graphic bar indicators and exact microsecond numbers.
- Press **`[UP]`** / **`[DOWN]`** to toggle between Page 1 (CH1..CH7) and Page 2 (CH8..CH14).

### Submenu 9: Analog Diagnostics (`DIAG ANAS`)
- Multi-page graphic diagnostics screen matching the `CHANNEL MONITOR` layout with 40-pixel graphic fill bars and exact 4-digit raw decimal ADC counts ($0 \dots 4095$):
  - **Page 1 (`ANALOG (1-6)`)**: Stick gimbals & switches: `RH:AIL`, `RV:ELE`, `LV:THR`, `LH:RUD`, `SW:SA`, `SW:SB`.
  - **Page 2 (`ANALOG (7-11)`)**: Rotary pots, switches, & battery: `POT:V1`, `POT:V2`, `SW:SC`, `SW:SD`, `VBAT`.
- Press **`[UP]`** / **`[DOWN]`** to switch between Page 1 and Page 2.
- Press **`[CANCEL]` (`[ESC]`)** to return to the Main Menu.

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

The firmware provides 4 convenient ways to initiate AFHDS 2A binding with clean separation logic:
1. **Hold `[BIND]` during Power-On**: Boots directly into RF bind mode.
2. **Hold `[BIND]` ($\ge 1.0\text{s}$) on Flight Screen**: Initiates binding from any flight dashboard page.
3. **`MODEL SETUP` Menu**: Select Field 2 (`Bind RX`) and press **`[OK]`**.
4. **`RX SETUP & BIND` Menu**: Press **`[OK]`**.

### Method A: Two-Way Telemetry Receivers (FS-iA6B, FS-iA10B)
1. Power on the receiver with a bind plug inserted into the `B/VCC` port (LED flashes rapidly).
2. Initiate binding on the transmitter using any of the 4 methods above.
3. The transmitter broadcasts bind packets and prompts `BINDING`.
4. Once the receiver receives the hopping table, it sends its unique ID back.
5. The transmitter automatically captures the ID, plays a rising 2-tone chime, saves to the active model profile in Flash, and switches to normal transmission. The top status bar shows `RF:OK` and live RSSI.
6. Remove the bind plug and power cycle the receiver.

### Method B: One-Way Receivers (FS-A8S, Fli14, FS-iA6)
1. Hold the receiver's bind button while powering on (LED flashes rapidly).
2. Initiate binding on the transmitter.
3. Once the receiver's LED turns solid (indicating it has locked onto the transmitter's frequency hopping table), press **`[CANCEL]` (`[ESC]`)**.
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
