//! Audio subsystem coordinating hardware piezo buzzer and DFPlayer Mini voice module.

pub mod dfplayer;
pub mod sounds;

pub use dfplayer::DfPlayer;
pub use sounds::SoundEvent;

use crate::buzzer::Buzzer;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AudioMode {
    Buzzer = 0,
    Voice = 1,
    Both = 2,
}

impl AudioMode {
    pub const fn from_u8(val: u8) -> Self {
        match val {
            1 => AudioMode::Voice,
            2 => AudioMode::Both,
            _ => AudioMode::Buzzer,
        }
    }
}

pub struct AudioSystem {
    pub buzzer: Buzzer,
    pub dfplayer: DfPlayer,
    pub mode: AudioMode,
    pub enabled: bool,
}

impl AudioSystem {
    pub const fn new() -> Self {
        Self {
            buzzer: Buzzer::new(),
            dfplayer: DfPlayer::new(),
            mode: AudioMode::Buzzer,
            enabled: true,
        }
    }

    /// Initialize both hardware buzzer (TIM1 PWM) and DFPlayer (USART3 on PC10 + PC14 BUSY).
    pub fn init(&mut self) {
        self.buzzer.init();
        self.dfplayer.init();
    }

    /// Update audio settings from persistent radio configuration.
    pub fn configure(&mut self, audio_enabled: bool, mode: u8, voice_volume: u8) {
        self.set_enabled(audio_enabled);
        self.mode = AudioMode::from_u8(mode);
        self.dfplayer.set_volume(voice_volume);
    }

    /// Enable or mute the entire audio subsystem.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.buzzer.enabled = enabled;
    }

    /// Periodic tick advancing active tones and non-blocking UART ring buffer.
    pub fn tick(&mut self, elapsed_ms: u16) {
        self.buzzer.tick(elapsed_ms);
        self.dfplayer.tick(elapsed_ms);
    }

    /// Dispatch a high-level sound event to DFPlayer voice and/or buzzer.
    pub fn event(&mut self, ev: SoundEvent) {
        if !self.enabled {
            return;
        }

        // 1. Trigger voice prompt on DFPlayer if Voice or Both
        if self.mode == AudioMode::Voice || self.mode == AudioMode::Both {
            self.dfplayer.play_track(ev.track_index());
        }

        // 2. Trigger corresponding buzzer tone if Buzzer or Both
        if self.mode == AudioMode::Buzzer || self.mode == AudioMode::Both {
            match ev {
                SoundEvent::Welcome => self.buzzer.click(),
                SoundEvent::Armed => self.buzzer.play_tone_pattern(2400, 60, 40, 1),
                SoundEvent::Disarmed => self.buzzer.play_tone_pattern(1800, 60, 40, 1),
                SoundEvent::LowBattery => self.buzzer.warn_battery(),
                SoundEvent::CritBattery => self.buzzer.play_tone_pattern(2800, 60, 40, 2),
                SoundEvent::RssiLow => self.buzzer.warn_rssi_low(),
                SoundEvent::RssiCrit => self.buzzer.warn_rssi_critical(),
                SoundEvent::PreflightWarning => self.buzzer.warn_preflight(),
                SoundEvent::Inactivity => self.buzzer.warn_inactivity(),
                SoundEvent::Failsafe => self.buzzer.play_tone_pattern(2800, 100, 50, 3),
                SoundEvent::TrimCenter => self.buzzer.trim_center(),
                SoundEvent::TrimLimit => self.buzzer.trim_limit(),
                SoundEvent::CalibStart => self.buzzer.play_tone(2200, 100),
                SoundEvent::CalibSuccess => self.buzzer.play_tone(2800, 150),
                SoundEvent::Timer1Min | SoundEvent::Timer30s | SoundEvent::Timer10s => {
                    self.buzzer.play_tone(2400, 70);
                }
                SoundEvent::TimerElapsed => {
                    self.buzzer.play_tone_pattern(2600, 100, 50, 3);
                }
                _ => {}
            }
        }
    }

    // --- Buzzer delegation & UI feedback ---

    /// Short tactile button click for UI navigation.
    pub fn click(&mut self) {
        if self.enabled {
            self.buzzer.click();
        }
    }

    pub fn play_tone(&mut self, freq_hz: u16, duration_ms: u16) {
        if self.enabled {
            self.buzzer.play_tone(freq_hz, duration_ms);
        }
    }

    pub fn play_tone_pattern(&mut self, freq_hz: u16, duration_ms: u16, pause_ms: u16, repeats: u8) {
        if self.enabled {
            self.buzzer.play_tone_pattern(freq_hz, duration_ms, pause_ms, repeats);
        }
    }

    #[allow(dead_code)]
    pub fn stop(&mut self) {
        self.buzzer.stop();
        self.dfplayer.stop();
    }

    pub fn trim_step(&mut self, step: i8) {
        if self.enabled {
            self.buzzer.trim_step(step);
        }
    }

    pub fn trim_center(&mut self) {
        self.event(SoundEvent::TrimCenter);
    }

    pub fn trim_limit(&mut self) {
        self.event(SoundEvent::TrimLimit);
    }

    pub fn warn_battery(&mut self) {
        self.event(SoundEvent::LowBattery);
    }

    pub fn warn_preflight(&mut self) {
        self.event(SoundEvent::PreflightWarning);
    }

    pub fn warn_inactivity(&mut self) {
        self.event(SoundEvent::Inactivity);
    }

    pub fn warn_rssi_low(&mut self) {
        self.event(SoundEvent::RssiLow);
    }

    pub fn warn_rssi_critical(&mut self) {
        self.event(SoundEvent::RssiCrit);
    }
}
