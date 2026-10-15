//! Settings and Diagnostics Menu Subsystem for FlySky FS-i6X.
//!
//! Provides navigation, 20-model memory management, channel reversing,
//! 5/9-point throttle curve editing with real-time spline visualization,
//! configuration editing with Flash persistence, live channel monitoring,
//! raw ADC diagnostics, and native ExpressLRS / CRSF configuration.

pub use crate::ui::format;
pub use crate::ui::widgets;
pub mod screens;

use crate::adc;
use crate::buzzer::Buzzer;
use crate::display::St7567;
use crate::storage::RadioStorage;
use crate::trim::TrimController;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MenuState {
    Closed,
    MainMenu,
    ModelSelect,
    ModelSetup,
    DualRateExpo,
    ThrottleCurve,
    WingMixer,
    MixerLineEdit,
    AuxChannels,
    ChannelReverse,
    RadioSetup,
    InputSetup,
    TelemSetup,
    RxSetup,
    CrsfSetup,
    ChannelMonitor,
    DiagAnas,
    SystemInfo,
}

#[derive(Copy, Clone, Debug)]
pub struct NavKeys {
    pub ok: bool,
    pub cancel: bool,
    pub up: bool,
    pub down: bool,
    pub bind: bool,
    pub sw_change: Option<u8>,
    pub dr_change: Option<u8>,
}

pub struct MenuController {
    pub state: MenuState,
    pub return_state: MenuState,
    pub selected_item: usize,
    pub scroll_offset: usize,
    pub main_menu_selected: usize,
    pub main_menu_scroll: usize,
    pub page_idx: usize,
    pub sub_idx: usize,
    pub edit_val: i32,
    pub editing: bool,
    pub request_calibration: bool,
    pub request_bind: bool,
    prev_keys: u16,
    pub waiting_release: bool,
    up_hold_ms: u16,
    down_hold_ms: u16,
    repeat_timer_ms: u16,
    prev_switches: crate::input::Switches,
}

impl Default for MenuController {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuController {
    pub const fn new() -> Self {
        Self {
            state: MenuState::Closed,
            return_state: MenuState::Closed,
            selected_item: 0,
            scroll_offset: 0,
            main_menu_selected: 0,
            main_menu_scroll: 0,
            page_idx: 0,
            sub_idx: 0,
            edit_val: 0,
            editing: false,
            request_calibration: false,
            request_bind: false,
            prev_keys: 0xFFFF,
            waiting_release: false,
            up_hold_ms: 0,
            down_hold_ms: 0,
            repeat_timer_ms: 0,
            prev_switches: crate::input::Switches::DEFAULT,
        }
    }

    /// Open the main settings menu.
    pub fn open(&mut self, buzzer: &mut Buzzer) {
        self.state = MenuState::MainMenu;
        self.selected_item = 0;
        self.scroll_offset = 0;
        self.main_menu_selected = 0;
        self.main_menu_scroll = 0;
        self.page_idx = 0;
        self.sub_idx = 0;
        self.edit_val = 0;
        self.editing = false;
        self.request_calibration = false;
        self.request_bind = false;
        self.waiting_release = true;
        self.prev_keys = 0xFFFF;
        self.up_hold_ms = 0;
        self.down_hold_ms = 0;
        self.repeat_timer_ms = 0;
        buzzer.click();
    }

    /// Return safely to Main Menu, preserving cursor index and scroll offset.
    pub fn return_to_main_menu(&mut self) {
        self.state = MenuState::MainMenu;
        self.selected_item = self.main_menu_selected;
        self.scroll_offset = self.main_menu_scroll;
        self.editing = false;
        self.waiting_release = true;
    }

    /// Returns true if any menu or diagnostic screen is active.
    pub fn is_active(&self) -> bool {
        self.state != MenuState::Closed
    }

    /// Process navigation keys, update menu state, and render display.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        lcd: &mut St7567,
        keys: u16,
        switches: &crate::input::Switches,
        storage: &mut RadioStorage,
        trims: &mut TrimController,
        raw_adc: &[u16; adc::NUM_CHANNELS],
        rf_chs: &[u16; crate::mixer::NUM_CHANNELS],
        buzzer: &mut Buzzer,
    ) {
        // Key release tracking (bit 10: OK, bit 11: Cancel, bit 9: Up, bit 8: Down, bit 12: Bind)
        if self.waiting_release
            && (keys & ((1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12))) == 0
        {
            self.waiting_release = false;
        }

        let newly_pressed = if self.waiting_release {
            0
        } else {
            keys & !self.prev_keys
        };
        self.prev_keys = keys;

        let raw_down = (keys & (1 << 8)) != 0 && !self.waiting_release;
        let raw_up = (keys & (1 << 9)) != 0 && !self.waiting_release;

        let mut down_pressed = (newly_pressed & (1 << 8)) != 0;
        let mut up_pressed = (newly_pressed & (1 << 9)) != 0;

        // Auto-repeat when UP or DOWN is held (quick traversal of lists, values, and characters)
        if raw_down {
            self.down_hold_ms = self.down_hold_ms.saturating_add(20);
            if self.down_hold_ms >= 300 {
                self.repeat_timer_ms = self.repeat_timer_ms.saturating_add(20);
                if self.repeat_timer_ms >= 70 {
                    down_pressed = true;
                    self.repeat_timer_ms = 0;
                }
            }
        } else {
            self.down_hold_ms = 0;
        }

        if raw_up {
            self.up_hold_ms = self.up_hold_ms.saturating_add(20);
            if self.up_hold_ms >= 300 {
                self.repeat_timer_ms = self.repeat_timer_ms.saturating_add(20);
                if self.repeat_timer_ms >= 70 {
                    up_pressed = true;
                    self.repeat_timer_ms = 0;
                }
            }
        } else {
            self.up_hold_ms = 0;
        }

        if !raw_down && !raw_up {
            self.repeat_timer_ms = 0;
        }

        let sw_change = switches.detect_condition_change(&self.prev_switches);
        let dr_change = switches.detect_dr_switch_change(&self.prev_switches);
        self.prev_switches = *switches;

        let nav_keys = NavKeys {
            ok: (newly_pressed & (1 << 10)) != 0,
            cancel: (newly_pressed & (1 << 11)) != 0,
            up: up_pressed,
            down: down_pressed,
            bind: (newly_pressed & (1 << 12)) != 0,
            sw_change,
            dr_change,
        };

        if self.state == MenuState::Closed {
            return;
        }

        lcd.clear_buffer();

        match self.state {
            MenuState::Closed => {}
            MenuState::MainMenu => {
                screens::main_menu::update(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::ModelSelect => {
                screens::model::update_select(self, lcd, &nav_keys, storage, trims, buzzer);
            }
            MenuState::ModelSetup => {
                screens::model::update_setup(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::DualRateExpo => {
                screens::mixer::update_dual_rate(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::ThrottleCurve => {
                screens::mixer::update_throttle_curve(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::WingMixer => {
                screens::mixer::update_wing_mixer(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::MixerLineEdit => {
                screens::mixer::update_mixer_line_edit(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::AuxChannels => {
                screens::channels::update_aux_channels(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::ChannelReverse => {
                screens::channels::update_channel_reverse(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::RadioSetup => {
                screens::setup::update_radio_setup(self, lcd, &nav_keys, storage, trims, buzzer);
            }
            MenuState::InputSetup => {
                screens::setup::update_input_setup(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::TelemSetup => {
                screens::setup::update_telem_setup(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::RxSetup => {
                screens::setup::update_rx_setup(self, lcd, &nav_keys, storage, buzzer);
            }
            MenuState::CrsfSetup => {
                screens::crsf::update(self, lcd, &nav_keys, buzzer);
            }
            MenuState::ChannelMonitor => {
                screens::channels::update_channel_monitor(self, lcd, &nav_keys, storage, rf_chs, buzzer);
            }
            MenuState::DiagAnas => {
                screens::diag::update_diag_anas(self, lcd, &nav_keys, raw_adc, switches, buzzer);
            }
            MenuState::SystemInfo => {
                screens::diag::update_system_info(self, lcd, &nav_keys, buzzer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_return_to_main_menu_preserves_position() {
        let mut ctrl = MenuController::new();
        let mut buzzer = Buzzer::new();

        ctrl.open(&mut buzzer);
        assert_eq!(ctrl.state, MenuState::MainMenu);
        assert_eq!(ctrl.selected_item, 0);
        assert_eq!(ctrl.scroll_offset, 0);

        // Simulate navigating to item 7 ("Radio Setup")
        ctrl.selected_item = 7;
        ctrl.scroll_offset = 5;
        ctrl.main_menu_selected = 7;
        ctrl.main_menu_scroll = 5;

        // Enter sub-menu: indices reset
        ctrl.state = MenuState::RadioSetup;
        ctrl.selected_item = 0;
        ctrl.scroll_offset = 0;
        ctrl.editing = true;

        // Return to main menu
        ctrl.return_to_main_menu();

        assert_eq!(ctrl.state, MenuState::MainMenu);
        assert_eq!(ctrl.selected_item, 7);
        assert_eq!(ctrl.scroll_offset, 5);
        assert!(!ctrl.editing);
        assert!(ctrl.waiting_release);
    }

    #[test]
    fn test_telem_setup_navigation_and_cycling() {
        let mut ctrl = MenuController::new();
        let mut buzzer = Buzzer::new();
        let mut lcd = St7567::new();
        let mut storage = RadioStorage::default_factory();

        ctrl.state = MenuState::TelemSetup;
        ctrl.selected_item = 0;
        ctrl.scroll_offset = 0;

        assert_eq!(storage.radio.telem_slots, [0, 1, 2, 3]);

        let mut nav = NavKeys {
            ok: true,
            cancel: false,
            up: false,
            down: false,
            bind: false,
            sw_change: None,
            dr_change: None,
        };

        // Press OK on slot 0 -> cycle to 1 (LQ / RSSI)
        screens::setup::update_telem_setup(&mut ctrl, &mut lcd, &nav, &mut storage, &mut buzzer);
        assert_eq!(storage.radio.telem_slots[0], 1);

        // Move down to slot 1
        nav.ok = false;
        nav.down = true;
        screens::setup::update_telem_setup(&mut ctrl, &mut lcd, &nav, &mut storage, &mut buzzer);
        assert_eq!(ctrl.selected_item, 1);

        // Press OK on slot 1 -> cycle to 2 (TX BAT)
        nav.down = false;
        nav.ok = true;
        screens::setup::update_telem_setup(&mut ctrl, &mut lcd, &nav, &mut storage, &mut buzzer);
        assert_eq!(storage.radio.telem_slots[1], 2);

        // Press Cancel -> return to RadioSetup with item 13 selected
        nav.ok = false;
        nav.cancel = true;
        screens::setup::update_telem_setup(&mut ctrl, &mut lcd, &nav, &mut storage, &mut buzzer);
        assert_eq!(ctrl.state, MenuState::RadioSetup);
        assert_eq!(ctrl.selected_item, 13);
    }

    #[test]
    fn test_telem_setup_full_cycle_and_timer() {
        let mut ctrl = MenuController::new();
        let mut buzzer = Buzzer::new();
        let mut lcd = St7567::new();
        let mut storage = RadioStorage::default_factory();

        ctrl.state = MenuState::TelemSetup;
        ctrl.selected_item = 0;
        ctrl.scroll_offset = 0;

        let nav = NavKeys {
            ok: true,
            cancel: false,
            up: false,
            down: false,
            bind: false,
            sw_change: None,
            dr_change: None,
        };

        // Cycle through all 13 sensor slots on slot 0: starts at 0
        for expected in 1..=12 {
            screens::setup::update_telem_setup(&mut ctrl, &mut lcd, &nav, &mut storage, &mut buzzer);
            assert_eq!(storage.radio.telem_slots[0], expected);
        }
        // Slot 11 is TIMER, Slot 12 is OFF
        assert_eq!(screens::setup::TELEM_SENSOR_NAMES[11], "TIMER");
        assert_eq!(screens::setup::TELEM_SENSOR_NAMES[12], "OFF");

        // Next cycle wraps back to 0 (RX BAT)
        screens::setup::update_telem_setup(&mut ctrl, &mut lcd, &nav, &mut storage, &mut buzzer);
        assert_eq!(storage.radio.telem_slots[0], 0);
        assert_eq!(screens::setup::TELEM_SENSOR_NAMES[0], "RX BAT");
    }

    #[test]
    fn test_telem_setup_no_text_overlap() {
        for slot in screens::setup::SLOT_NAMES {
            let label_end_x = 4 + (slot.len() * 6);
            for sensor in screens::setup::TELEM_SENSOR_NAMES {
                let val_start_x = 124 - (sensor.len() * 6);
                assert!(
                    val_start_x > label_end_x,
                    "Sensor '{}' (starts at x={}) overlaps slot '{}' (ends at x={})",
                    sensor,
                    val_start_x,
                    slot,
                    label_end_x
                );
            }
        }
    }
}
