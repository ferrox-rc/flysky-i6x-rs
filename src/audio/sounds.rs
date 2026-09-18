//! Sound events catalog and MicroSD track mappings for voice prompts.

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SoundEvent {
    Welcome = 1,
    Armed = 2,
    Disarmed = 3,
    LowBattery = 4,
    CritBattery = 5,
    RssiLow = 6,
    RssiCrit = 7,
    PreflightWarning = 8,
    Inactivity = 9,
    Failsafe = 10,
    FlightModeAcro = 11,
    FlightModeAngle = 12,
    FlightModeHorizon = 13,
    FlightModeRth = 14,
    FlightModeManual = 15,
    FlightModeHold = 16,
    Timer1Min = 17,
    Timer30s = 18,
    Timer10s = 19,
    TimerElapsed = 20,
    TrimCenter = 21,
    TrimLimit = 22,
    CalibStart = 23,
    CalibSuccess = 24,
}

impl SoundEvent {
    /// Returns the 1-based track index on the MicroSD card (e.g. 0001.mp3 -> 1).
    #[inline(always)]
    pub const fn track_index(self) -> u16 {
        self as u16
    }
}
