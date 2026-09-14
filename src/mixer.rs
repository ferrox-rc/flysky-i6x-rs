//! Flight Control, Input Conditioning, and 14-Channel Mixing Engine.
//!
//! Implements:
//! 1. Input conditioning: Dual Rates (30%..100%) and integer cubic Expo (-100%..+100%).
//! 2. Aircraft wing/tail templates: Normal, Elevon / Delta, V-Tail, and Flaperon.
//! 3. Auxiliary channel remapping for CH5..CH14.
//! 4. EdgeTX / OpenTX-style 14-channel freeform matrix mixer with ADD, MULTIPLY, and REPLACE modes.
//! 5. Digital trim integration and channel reversing.

use crate::input::{SwitchPos, Switches};
use crate::storage::ModelConfig;
use crate::trim::TrimController;

/// Physical source identifiers for auxiliary channels and mix lines.
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MixSource {
    None = 0,
    Roll = 1,
    Pitch = 2,
    Throttle = 3,
    Yaw = 4,
    Vra = 5,
    Vrb = 6,
    Sa = 7,
    Sb = 8,
    Sc = 9,
    Sd = 10,
    Max = 11,
    // 12..25 map to Channel 1..14
}

/// Wing and tail aircraft mixing templates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WingTailTemplate {
    Normal = 0,
    Elevon = 1,
    VTail = 2,
    Flaperon = 3,
}

impl WingTailTemplate {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Elevon,
            2 => Self::VTail,
            3 => Self::Flaperon,
            _ => Self::Normal,
        }
    }
}

/// Apply Dual Rate and integer cubic Expo to a normalized stick input (-1000..+1000).
///
/// Positive expo softens stick sensitivity around neutral center.
/// Negative expo increases sensitivity around neutral center.
pub fn apply_dr_expo(input: i16, rate: u8, expo: i8) -> i16 {
    let rate_clamped = rate.clamp(30, 100) as i32;
    let x = ((input as i32) * rate_clamped) / 100;

    if expo == 0 {
        return x.clamp(-1000, 1000) as i16;
    }

    let expo_val = expo.clamp(-100, 100) as i32;
    // Normalized cubic term in -1000..+1000 fits entirely within i32 (1000^3 = 10^9 < 2.14*10^9)
    let x_cubic = (x * x * x) / 1_000_000;

    let result = if expo_val > 0 {
        // Soften center: blend linear with cubic
        (x * (100 - expo_val) + x_cubic * expo_val) / 100
    } else {
        // Sharpen center: inverse cubic blend
        let abs_expo = -expo_val;
        x + ((x - x_cubic) * abs_expo) / 100
    };

    result.clamp(-1000, 1000) as i16
}

/// Check if a physical switch satisfies a mixer line activation condition.
pub fn is_switch_active(condition: u8, switches: &Switches) -> bool {
    match condition {
        0 => true, // Always active
        1 => switches.sa == SwitchPos::Up,
        2 => switches.sa == SwitchPos::Down,
        3 => switches.sb == SwitchPos::Up,
        4 => switches.sb == SwitchPos::Mid,
        5 => switches.sb == SwitchPos::Down,
        6 => switches.sc == SwitchPos::Up,
        7 => switches.sc == SwitchPos::Mid,
        8 => switches.sc == SwitchPos::Down,
        9 => switches.sd == SwitchPos::Up,
        10 => switches.sd == SwitchPos::Down,
        _ => true,
    }
}

/// Determine whether High Rates (true) or Low Rates (false) are active.
pub fn is_dr_high(dr_switch: u8, switches: &Switches) -> bool {
    match dr_switch {
        0 => true, // Default to High Rate if no switch assigned
        1 => switches.sa == SwitchPos::Up,
        2 => switches.sb == SwitchPos::Up,
        3 => switches.sc == SwitchPos::Up,
        4 => switches.sd == SwitchPos::Up,
        _ => true,
    }
}

/// Evaluate a source identifier to a normalized value (-1000..+1000).
pub fn evaluate_source(
    src: u8,
    cond_sticks: &[i16; 4],
    pots: &[i16; 2],
    switches: &Switches,
    channels: &[i32; 14],
) -> i32 {
    match src {
        0 => 0,
        1 => cond_sticks[0] as i32, // Roll
        2 => cond_sticks[1] as i32, // Pitch
        3 => cond_sticks[2] as i32, // Throttle
        4 => cond_sticks[3] as i32, // Yaw
        5 => pots[0] as i32,        // VRA
        6 => pots[1] as i32,        // VRB
        7 => if switches.sa == SwitchPos::Up { -1000 } else { 1000 },
        8 => match switches.sb {
            SwitchPos::Up => -1000,
            SwitchPos::Mid => 0,
            SwitchPos::Down => 1000,
        },
        9 => match switches.sc {
            SwitchPos::Up => -1000,
            SwitchPos::Mid => 0,
            SwitchPos::Down => 1000,
        },
        10 => if switches.sd == SwitchPos::Up { -1000 } else { 1000 },
        11 => 1000, // MAX
        12..=25 => {
            let ch_idx = (src - 12) as usize;
            channels[ch_idx]
        }
        _ => 0,
    }
}

/// Complete 4-stage mixer pipeline producing 14 AFHDS 2A microsecond pulses (1000..2000 µs).
#[allow(clippy::too_many_arguments)]
pub fn compute_channels(
    raw_roll: i16,
    raw_pitch: i16,
    curved_throttle: u16, // 0..1000 from throttle curve engine
    raw_yaw: i16,
    pots: &[i16; 2],
    switches: &Switches,
    model: &ModelConfig,
    trims: &TrimController,
    throttle_trim_mode: u8,
) -> [u16; 14] {
    // Stage 1 & 2: Input conditioning with Dual Rates & Expo
    let high_rate = is_dr_high(model.dr_switch, switches);
    let rates = if high_rate { model.dr_high } else { model.dr_low };
    let expos = if high_rate { model.expo_high } else { model.expo_low };

    let cond_roll = apply_dr_expo(raw_roll, rates[0], expos[0]);
    let cond_pitch = apply_dr_expo(raw_pitch, rates[1], expos[1]);
    let cond_yaw = apply_dr_expo(raw_yaw, rates[2], expos[2]);
    // Throttle input is normalized to -1000..+1000 from curved_throttle (0..1000)
    let cond_thr = (curved_throttle as i32 * 2 - 1000).clamp(-1000, 1000) as i16;

    let cond_sticks = [cond_roll, cond_pitch, cond_thr, cond_yaw];

    // Stage 3: Initialize channels (-1000..+1000)
    let mut ch_vals = [0i32; 14];

    // Apply Wing/Tail templates to primary channels
    let template = WingTailTemplate::from_u8(model.wing_tail_mix);
    match template {
        WingTailTemplate::Normal => {
            ch_vals[0] = cond_roll as i32;
            ch_vals[1] = cond_pitch as i32;
            ch_vals[2] = cond_thr as i32;
            ch_vals[3] = cond_yaw as i32;
        }
        WingTailTemplate::Elevon => {
            // Delta wing: Left Elevon (CH1) = (Pitch - Roll)/2, Right Elevon (CH2) = (Pitch + Roll)/2
            let p = cond_pitch as i32;
            let r = cond_roll as i32;
            ch_vals[0] = (p - r).clamp(-1000, 1000);
            ch_vals[1] = (p + r).clamp(-1000, 1000);
            ch_vals[2] = cond_thr as i32;
            ch_vals[3] = cond_yaw as i32;
        }
        WingTailTemplate::VTail => {
            // V-Tail: Left V-Tail (CH2) = (Pitch + Yaw)/2, Right V-Tail (CH4) = (Pitch - Yaw)/2
            let p = cond_pitch as i32;
            let y = cond_yaw as i32;
            ch_vals[0] = cond_roll as i32;
            ch_vals[1] = (p + y).clamp(-1000, 1000);
            ch_vals[2] = cond_thr as i32;
            ch_vals[3] = (p - y).clamp(-1000, 1000);
        }
        WingTailTemplate::Flaperon => {
            // Dual ailerons: CH1 Left Aileron, CH6 Right Aileron. Flaps driven from aux channel 6
            let r = cond_roll as i32;
            let flap = evaluate_source(model.aux_channels[1], &cond_sticks, pots, switches, &ch_vals);
            ch_vals[0] = (r + flap / 2).clamp(-1000, 1000);
            ch_vals[1] = cond_pitch as i32;
            ch_vals[2] = cond_thr as i32;
            ch_vals[3] = cond_yaw as i32;
            ch_vals[5] = (-r + flap / 2).clamp(-1000, 1000);
        }
    }

    // Populate Auxiliary Channels (CH5..CH14) from aux_channels mappings
    for (i, &src) in model.aux_channels.iter().enumerate() {
        let ch_idx = 4 + i;
        if ch_idx < 14 {
            // If Flaperon mode is active, CH6 (idx 5) is managed by template
            if template == WingTailTemplate::Flaperon && ch_idx == 5 {
                continue;
            }
            ch_vals[ch_idx] = evaluate_source(src, &cond_sticks, pots, switches, &ch_vals);
        }
    }

    // Stage 3b: Freeform matrix mixer lines
    for mix in model.mixes.iter() {
        if mix.target_ch == 0 || mix.target_ch > 14 {
            continue;
        }
        if !is_switch_active(mix.switch, switches) {
            continue;
        }

        let target_idx = (mix.target_ch - 1) as usize;
        let src_val = evaluate_source(mix.source, &cond_sticks, pots, switches, &ch_vals);

        let term = (src_val * (mix.weight as i32)) / 100 + (mix.offset as i32 * 10);

        match mix.mode {
            0 => {
                // ADD (+)
                ch_vals[target_idx] = (ch_vals[target_idx] + term).clamp(-1000, 1000);
            }
            1 => {
                // MULTIPLY (*)
                ch_vals[target_idx] = ((ch_vals[target_idx] * term) / 1000).clamp(-1000, 1000);
            }
            2 => {
                // REPLACE (:=)
                ch_vals[target_idx] = term.clamp(-1000, 1000);
            }
            _ => {}
        }
    }

    // Stage 4: Outputs (convert -1000..+1000 to 1000..2000 µs pulses)
    let mut rf_chs = [1500u16; 14];
    for (i, &val) in ch_vals.iter().enumerate() {
        rf_chs[i] = ((val / 2) + 1500).clamp(1000, 2000) as u16;
    }

    // Apply digital trims to primary flight channels
    rf_chs[0] = TrimController::apply(rf_chs[0], trims.values.roll);
    rf_chs[1] = TrimController::apply(rf_chs[1], trims.values.pitch);
    rf_chs[2] = TrimController::apply_throttle(rf_chs[2], trims.values.throttle, throttle_trim_mode);
    rf_chs[3] = TrimController::apply(rf_chs[3], trims.values.yaw);

    // Apply channel reversing bitmask
    let rev_mask = model.channel_reverse;
    for (ch, val) in rf_chs.iter_mut().enumerate() {
        if (rev_mask & (1 << ch)) != 0 {
            *val = 3000 - *val;
        }
    }

    rf_chs
}
