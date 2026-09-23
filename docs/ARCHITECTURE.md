# SOFTWARE ARCHITECTURE & TIMING MODEL

`flysky-i6x-rs` uses a bare-metal, `no_std` reactive architecture designed for deterministic RF packet timing and sub-millisecond control latency on the STM32F072VB Cortex-M0 microcontroller.

---

## 1. Clock Tree Configuration

| Component | Setting | Notes |
| :--- | :--- | :--- |
| **Primary Oscillator** | **HSE Crystal @ 8.000 MHz** | Clean external quartz crystal on pins `PD0`/`PD1` |
| **PLL Multiplier** | **PLLMUL = 6** | 8.000 MHz * 6 = **48.000 MHz** core clock |
| **Flash Latency** | **1 Wait State (`LATENCY = 1`)** | Mandatory for Cortex-M0 operation above 24 MHz |
| **System Busses** | **AHB = 48 MHz, APB = 48 MHz** | Prescalers set to 1 for maximum peripheral throughput |
| **USB Physical Clock** | **USBSW = 1 (PLLCLK)** | Routes 48.000 MHz PLL directly to USB peripheral (`RCC_CFGR3` bit 7) |

Implemented in [`src/chip/mod.rs`](../src/chip/mod.rs) using pure PAC direct register access (`pac::RCC`).

---

## 2. Real-Time Concurrency & Safety Model

```mermaid
flowchart TD
    subgraph Hardware_Timers ["Deterministic Interrupts & Peripherals"]
        TIM16["TIM16 ISR @ 259.74 Hz (3.850 ms)<br>Pulls fresh PENDING_CHANNELS<br>Transmits 38-byte AFHDS 2A Packet via SPI"]
        EXTI["EXTI2_3 ISR (A7105 GIO2)<br>Handles TX Finished / RX Telemetry Available"]
        DMA["DMA1 Channel 1 (Autonomous)<br>Scans all 11 ADC channels in 0.23 ms"]
        USB_IRQ["USB Full-Speed Interrupt<br>Low priority (0xC0), handles host bus events"]
        PWM_Audio["TIM1 Channel 1 (Hardware PWM @ PA8)<br>Drives Piezo Buzzer Audio Frequencies"]
        PWM_BL["TIM3 Channel 4 (Hardware PWM @ PC9)<br>1 kHz Backlight Dimming Mod"]
        IWDG["Hardware Watchdog (pac::IWDG @ 40 kHz LSI)<br>2.0s Hard Timeout, APB1 Freeze in Debug"]
    end

    subgraph Main_Thread ["Main Execution Loop (~500 Hz)"]
        ADC_Poll["1. Read DMA Buffer (input::poll)<br>Adaptive Jitter Filter + Calibrations"]
        Keys["2. Scan Key Matrix (boot::scan_keys)<br>Digital Trims & Navigation Shortcuts"]
        Trims["3. Apply Digital Trims<br>Roll, Pitch, Throttle (Option 1/2), Yaw"]
        Curves["4. Curve Engine (curve::evaluate_curve)<br>5/9-Point Catmull-Rom Spline Interpolation"]
        Rev["5. Channel Reversing<br>14-bit mask: pulse = 3000 - pulse"]
        USB_Poll["6. USB Subsystem Poll (usb::poll)<br>100 Hz HID Gamepad & CDC Serial CLI"]
        RF_Update["7. Update rf::set_channels(&rf_chs)<br>Pushes latest channels to atomic buffer (RF standby if Sim)"]
        Menu_Router["8. Menu State Machine & Wizards<br>Model Select, Setup, Curves, Calib"]
        LCD_Draw["9. Draw Framebuffer & Strobe LCD<br>ST7567 8-bit parallel bus (~1.2 ms)"]
        WDT_Feed["10. Pet Hardware Watchdog (watchdog::feed)<br>Prevents 2.0s hardware reset"]
    end

    DMA -.-> ADC_Poll
    ADC_Poll --> Keys --> Trims --> Curves --> Rev --> USB_Poll --> RF_Update --> Menu_Router --> LCD_Draw --> WDT_Feed
    RF_Update -. Atomic Buffer .-> TIM16
    WDT_Feed -. Reload .-> IWDG
```

### Interrupt Priorities & Safety Systems
- **Hardware Watchdog (`pac::IWDG`)**: Independent 2.0-second hardware watchdog running off the 40 kHz internal low-speed oscillator (LSI) with prescaler `/128` (312.5 Hz tick rate) and reload count `625`. Prior to watchdog key registration, LSI clock stabilization is confirmed via `pac::RCC.csr.lsirdy`. To support non-intrusive SWD debugging via ST-Link or probe-rs, `DBGMCU_APB1_FZ.DBG_IWDG_STOP` is set so the watchdog timer freezes when the core is halted. The watchdog is refreshed (`watchdog::feed()`) strictly at the bottom of the main execution loop (~500 Hz).
- **High Priority (RF Transmission)**: `TIM16` fires strictly every **3.850 ms** (259.74 Hz). It pulls the latest pre-computed channel microsecond pulses from `PENDING_CHANNELS` and initiates A7105 SPI transmission. Priority = `0x80`.
- **Medium Priority (Radio Event)**: `EXTI2_3` fires on A7105 GIO2 line transitions (packet transmission complete or downlink telemetry packet received). Priority = `0x80`.
- **Autonomous DMA**: `DMA1_CH1` transfers all 11 ADC channels directly into circular SRAM buffers with zero CPU intervention.
- **Hardware Timers**: `TIM1` generates non-blocking audio frequencies on `PA8`; `TIM3` generates 1 kHz PWM brightness control on `PC9`.
- **Low Priority (USB Physical Layer)**: The USB interrupt is assigned priority `0xC0`. Because RF interrupts have higher priority (`0x80`), USB transactions or host bus stalls can never preempt or delay an over-the-air packet.
- **Background / Main Loop**: Decoupled control loop architecture; the real-time flight control pipeline (ADC sampling, lightweight 4-sample filtering with dynamic deadband bypass, matrix mixer, D/R & expo, throttle curves) executes in under 30 µs at multi-kHz pass rates, updating double-buffered `PENDING_CHANNELS` for RF transmission, while ST7567 LCD frame rendering and SPI flushing are throttled to a smooth 30 Hz (~33 ms).

---

## 3. Channel Latency & Data Freshness

1. **Continuous ADC Scan**: All 11 channels (4 sticks, 4 switches, 2 pots, battery) are digitized continuously by ADC1 via DMA in **0.23 ms** (252 cycles * 11 / 12 MHz).
2. **Double-Buffered Channels**: The flight loop updates `PENDING_CHANNELS` within critical sections (`cortex_m::interrupt::free`).
3. **Guaranteed Fresh Packets**: Because the decoupled flight control loop runs at kHz rates while the RF transmitter transmits at ~260 Hz, **every over-the-air packet carries fresh, up-to-date stick data** with end-to-end latency under **2.0 ms**.

---

## 4. Memory Footprint
 
Measured on release builds (`thumbv6m-none-eabi`, opt-level = "z", LTO = "fat"):
- **Application Flash Partition (`memory.x`)**: **120 KB** (`0x0800_0000 .. 0x0801_DFFF`, Pages 0–59) allocated for firmware code.
- **Firmware Binary**: **~89.4 KB** (.text 89,432 bytes + .data 1,876 bytes = 91.3 KB flash total).
- **Free Program Space**: **~30.8 KB** (~25.7% free headroom) remaining within the 120 KB partition for future expansions.
- **Non-Volatile Storage (Flash Pages 60–63)**: **8 KB** (`0x0801_E000 .. 0x0802_0000`, 4 × 2048-byte pages) managed as a log-structured append-only storage engine via `sequential-storage`. Writes complete in **~2.8 ms** with zero page erases on routine updates, wear-levelled across all 4 pages.
- **SRAM (16 KB total)**: **~3.0 KB** static allocation (`.data` 1,876 bytes + `.bss` 1,128 bytes) + 1024-byte LCD framebuffer + 1024-byte USB Packet Memory Area (PMA). **Over 81% of SRAM remains free**, with **> 6.3 KB** guaranteed stack margin preventing any stack-on-static collision.
- **Zero Heap & In-Place Loading**: Entirely static allocation; no dynamic heap allocations, no `alloc` crate, and zero pass-by-value stack instantiation for model storage structures.

---

## 5. Throttle Curve Engine & Catmull-Rom Spline Math (`src/curve.rs`)

The curve engine transforms normalized stick inputs (0..1000) into tailored output curves:
- **Modes**: 5-point (0%, 25%, 50%, 75%, 100%) and 9-point (0%, 12.5%, 25%, ..., 100%).
- **Linear Interpolation**:
  ```text
  val = y0 + ((y1 - y0) * delta_x) / x_span
  ```
- **Catmull-Rom Cubic Spline Smoothing**:
  To achieve smooth, C1-continuous throttle response without flat inflection points or aggressive step changes, the engine evaluates standard Catmull-Rom cubic Hermite splines:
  ```text
  P(t) = 0.5 * (2*P1 + (-P0 + P2)*t + (2*P0 - 5*P1 + 4*P2 - P3)*t^2 + (-P0 + 3*P1 - 3*P2 + P3)*t^3)
  ```
- **Deterministic Fixed-Point Execution**:
  Implemented using integer-only fixed-point arithmetic (`t` scaled by 1024). Spline interpolation executes in **< 60 CPU clock cycles** (< 1.25 µs at 48 MHz), meaning spline smoothing imposes zero perceptible latency on the control loop.
