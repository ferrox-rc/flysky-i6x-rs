# Software Architecture & Timing Model

`flysky-i6x-rs` uses a bare-metal, `no_std` reactive architecture designed for deterministic RF packet timing and sub-millisecond control latency on the STM32F072VB Cortex-M0 microcontroller.

---

## 1. Clock Tree Configuration

| Component | Setting | Notes |
| :--- | :--- | :--- |
| **Primary Oscillator** | **HSE Crystal @ 8.000 MHz** | Clean external quartz crystal on pins `PD0`/`PD1` |
| **PLL Multiplier** | **PLLMUL = 6** | $8.000\text{ MHz} \times 6 = \mathbf{48.000\text{ MHz}}$ core clock |
| **Flash Latency** | **1 Wait State (`LATENCY = 1`)** | Mandatory for Cortex-M0 operation above 24 MHz |
| **System Busses** | **AHB = 48 MHz, APB = 48 MHz** | Prescalers set to 1 for maximum peripheral throughput |

Implemented in [`src/chip/mod.rs`](../src/chip/mod.rs).

---

## 2. Real-Time Concurrency Model

```mermaid
flowchart TD
    subgraph Hardware_Timers [Deterministic Interrupts & Peripherals]
        TIM16["TIM16 ISR @ 259.74 Hz (3.850 ms)<br>Pulls fresh PENDING_CHANNELS<br>Transmits 38-byte AFHDS 2A Packet via SPI"]
        EXTI["EXTI2_3 ISR (A7105 GIO2)<br>Handles TX Finished / RX Telemetry Available"]
        DMA["DMA1 Channel 1 (Autonomous)<br>Scans all 11 ADC channels in 0.23 ms"]
        PWM_Audio["TIM1 Channel 1 (Hardware PWM @ PA8)<br>Drives Piezo Buzzer Audio Frequencies"]
        PWM_BL["TIM3 Channel 4 (Hardware PWM @ PC9)<br>1 kHz Backlight Dimming Mod"]
    end

    subgraph Main_Thread [Main Execution Loop (~500 Hz)]
        ADC_Poll["1. Read DMA Buffer (input::poll)<br>Adaptive Jitter Filter + Calibrations"]
        Keys["2. Scan Key Matrix (boot::scan_keys)<br>Digital Trims & Navigation Shortcuts"]
        Trims["3. Apply Digital Trims<br>Roll, Pitch, Throttle (Option 1/2), Yaw"]
        Curves["4. Curve Engine (curve::evaluate_curve)<br>5/9-Point Catmull-Rom Spline Interpolation"]
        Rev["5. Channel Reversing<br>14-bit mask: pulse = 3000 - pulse"]
        RF_Update["6. Update rf::set_channels(&rf_chs)<br>Pushes latest channels to atomic buffer"]
        Menu_Router["7. Menu State Machine & Wizards<br>Model Select, Setup, Curves, Calib"]
        LCD_Draw["8. Draw Framebuffer & Strobe LCD<br>ST7567 8-bit parallel bus (~1.2 ms)"]
    end

    DMA -.-> ADC_Poll
    ADC_Poll --> Keys --> Trims --> Curves --> Rev --> RF_Update
    RF_Update -. Atomic Buffer .-> TIM16
```

### Interrupt Priorities
- **High Priority (RF Transmission)**: `TIM16` fires strictly every **3.850 ms** (259.74 Hz). It pulls the latest pre-computed channel microsecond pulses from `PENDING_CHANNELS` and initiates A7105 SPI transmission.
- **Medium Priority (Radio Event)**: `EXTI2_3` fires on A7105 GIO2 line transitions (packet transmission complete or downlink telemetry packet received).
- **Autonomous DMA**: `DMA1_CH1` transfers all 11 ADC channels directly into circular SRAM buffers with zero CPU intervention.
- **Hardware Timers**: `TIM1` generates non-blocking audio frequencies on `PA8`; `TIM3` generates 1 kHz PWM brightness control on `PC9`.
- **Background / Main Loop**: Runs at ~500 Hz (every 1.5–2.0 ms), continuously sampling ADC values, applying trims, curves, and channel reversing, and writing the 1024-byte framebuffer to the ST7567 LCD.

---

## 3. Channel Latency & Data Freshness

1. **Continuous ADC Scan**: All 11 channels (4 sticks, 4 switches, 2 pots, battery) are digitized continuously by ADC1 via DMA in **0.23 ms** ($252\text{ cycles} \times 11 / 12\text{ MHz}$).
2. **Double-Buffered Channels**: The main loop updates `PENDING_CHANNELS` within critical sections (`cortex_m::interrupt::free`).
3. **Guaranteed Fresh Packets**: Because the main processing loop runs at ~500 Hz while the RF transmitter transmits at ~260 Hz, **every over-the-air packet carries fresh, up-to-date stick data** with end-to-end latency under **2.0 ms**.

---

## 4. Memory Footprint

Measured on release builds (`thumbv6m-none-eabi`, opt-level 3 / z):
- **Firmware Binary (Flash Pages 0–17)**: **35.4 KB** out of **128 KB** available.
- **Free Program Space (Flash Pages 18–61)**: **~88 KB** remaining for future extensions.
- **Non-Volatile Storage (Flash Pages 62–63)**: **2,688 bytes** allocated for global radio configuration and 20 full model profiles (1,408 bytes free headroom).
- **SRAM**: **228 bytes** static allocation (`.data` + `.bss`) + 1024-byte framebuffer in RAM. **Over 90% of SRAM remains free**.
- **Zero Heap**: Entirely static allocation; no dynamic heap allocations, no `alloc` crate, no risk of heap fragmentation.

---

## 5. Throttle Curve Engine & Catmull-Rom Spline Math (`src/curve.rs`)

The curve engine transforms normalized stick inputs ($0 \dots 1000$) into tailored output curves:
- **Modes**: 5-point ($0\%, 25\%, 50\%, 75\%, 100\%$) and 9-point ($0\%, 12.5\%, 25\%, \dots, 100\%$).
- **Linear Interpolation**:
  $$\text{val} = y_0 + \frac{(y_1 - y_0) \times \Delta x}{x_{\text{span}}}$$
- **Catmull-Rom Cubic Spline Smoothing**:
  To achieve smooth, C1-continuous throttle response without flat inflection points or aggressive step changes, the engine evaluates standard Catmull-Rom cubic Hermite splines:
  $$P(t) = 0.5 \times \left(2 P_1 + (-P_0 + P_2) t + (2 P_0 - 5 P_1 + 4 P_2 - P_3) t^2 + (-P_0 + 3 P_1 - 3 P_2 + P_3) t^3\right)$$
- **Deterministic Fixed-Point Execution**:
  Implemented using integer-only fixed-point arithmetic ($t$ scaled by 1024). Spline interpolation executes in **$< 60$ CPU clock cycles** ($< 1.25\,\mu\text{s}$ at 48 MHz), meaning spline smoothing imposes zero perceptible latency on the control loop.
