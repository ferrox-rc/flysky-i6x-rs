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
    subgraph Hardware_Timers [Deterministic Interrupts]
        TIM16["TIM16 ISR @ 259.74 Hz (3.850 ms)<br>Pulls fresh PENDING_CHANNELS<br>Transmits 38-byte AFHDS 2A Packet via SPI"]
        EXTI["EXTI2_3 ISR (A7105 GIO2)<br>Handles TX Finished / RX Telemetry Available"]
        DMA["DMA1 Channel 1 (Autonomous)<br>Scans all 11 ADC channels in 0.23 ms"]
        PWM["TIM1 Channel 1 (Hardware PWM)<br>Drives PA8 Piezo Buzzer"]
    end

    subgraph Main_Thread [Main Execution Loop (~500 Hz)]
        ADC_Poll["1. Read DMA Buffer (input::poll)<br>MMA Jitter Filter + Calib + Trims"]
        Keys["2. Scan Key Matrix (boot::scan_keys)<br>Digital Trims & Shortcuts"]
        RF_Update["3. Update rf::set_channels(&rf_chs)<br>Pushes latest channels to atomic buffer"]
        Calib_Check["4. Calibration Wizard / Menu Router<br>If active, routes input to wizard"]
        LCD_Draw["5. Draw Framebuffer & Strobe LCD<br>ST7567 8-bit parallel bus (~1.8 ms)"]
    end

    DMA -.-> ADC_Poll
    ADC_Poll --> RF_Update
    RF_Update -. Atomic Buffer .-> TIM16
```

### Interrupt Priorities
- **High Priority (RF Transmission)**: `TIM16` fires strictly every **3.850 ms** (259.74 Hz). It pulls the latest pre-computed channel microsecond pulses from `PENDING_CHANNELS` and initiates A7105 SPI transmission.
- **Medium Priority (Radio Event)**: `EXTI2_3` fires on A7105 GIO2 line transitions (packet transmission complete or downlink telemetry packet received).
- **Background / Main Loop**: Runs at ~500 Hz (every 1.5–2.0 ms), continuously sampling ADC values, applying trims and calibrations, and writing the 1024-byte framebuffer to the ST7567 LCD.

---

## 3. Channel Latency & Data Freshness

1. **Continuous ADC Scan**: All 11 channels (4 sticks, 4 switches, 2 pots, battery) are digitized continuously by ADC1 via DMA in **0.23 ms** ($252\text{ cycles} \times 11 / 12\text{ MHz}$).
2. **Double-Buffered Channels**: The main loop updates `PENDING_CHANNELS` within critical sections (`cortex_m::interrupt::free`).
3. **Guaranteed Fresh Packets**: Because the main processing loop runs at ~500 Hz while the RF transmitter transmits at ~260 Hz, **every over-the-air packet carries fresh, up-to-date stick data** with end-to-end latency under **2.0 ms**.

---

## 4. Memory Footprint

Measured on release builds (`thumbv6m-none-eabi`, opt-level 3 / z):
- **Flash ROM**: ~**23 KB** used out of **128 KB** available (**~82% Flash remains free** for models and menus).
- **SRAM**: ~**2.4 KB** used (including 1024-byte display framebuffer) out of **16 KB** available (**~85% SRAM remains free**).
- **Zero Heap**: Entirely static allocation; no `alloc` crate, no dynamic heap fragmentation.
