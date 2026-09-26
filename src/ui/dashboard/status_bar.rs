//! Top status bar widget for the flight dashboard.

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
    image::Image,
};
use embedded_icon::icons::mdi::size12px::Battery;
use embedded_icon::NewIcon;

use crate::buzzer::Buzzer;
use crate::display::St7567;
use crate::rf::{self, afhds2a::TelemetryData};
use crate::storage::RadioStorage;
use crate::ui::format::format_vbat;
use crate::usb;

const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

#[allow(clippy::too_many_arguments)]
pub fn render(
    lcd: &mut St7567,
    storage: &RadioStorage,
    battery_mv: u16,
    rf_ok: bool,
    is_binding: bool,
    telem: &TelemetryData,
    buzzer: &mut Buzzer,
    blink_phase: u8,
    vbat_alarm_timer: &mut u32,
    rssi_alarm_timer: &mut u32,
    min_rssi: &mut u8,
    min_rx_v_mv: &mut u16,
    telem_seen: &mut bool,
) {
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let sep_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let is_crsf = storage.active_model().rf_protocol == 1;

    // --- Model Name (x = 2..60, y = 9) ---
    let act_idx = storage.radio.active_model as usize;
    let raw_name = core::str::from_utf8(&storage.active_model().name).unwrap_or("").trim();
    let mut m_buf = *b"M00";
    m_buf[1] = b'0' + ((act_idx + 1) / 10) as u8;
    m_buf[2] = b'0' + ((act_idx + 1) % 10) as u8;
    let m_fallback = core::str::from_utf8(&m_buf).unwrap_or("M01");
    let m_display = if raw_name.is_empty() { m_fallback } else { raw_name };
    Text::new(m_display, Point::new(2, 9), text_style).draw(lcd).ok();

    // --- RF / Telemetry / Link Status (x = 65..95, y = 9) ---
    if !rf_ok && !is_crsf {
        let id = rf::get_last_chip_id();
        let mut err_buf = *b"E:00";
        err_buf[2] = HEX_CHARS[((id >> 4) & 0x0F) as usize];
        err_buf[3] = HEX_CHARS[(id & 0x0F) as usize];
        let err_str = core::str::from_utf8(&err_buf).unwrap_or("E:??");
        Text::new(err_str, Point::new(68, 9), text_style).draw(lcd).ok();
    } else if is_binding {
        Text::new("BIND", Point::new(68, 9), text_style).draw(lcd).ok();
    } else if usb::is_sim_mode() {
        Text::new("U:SIM", Point::new(65, 9), text_style).draw(lcd).ok();
    } else if telem.connected {
        let mut rssi_buf = *b"R:  %";
        let r = telem.rssi.min(100);
        if r >= 100 {
            Text::new("R:100", Point::new(65, 9), text_style).draw(lcd).ok();
        } else {
            rssi_buf[2] = b'0' + (r / 10);
            rssi_buf[3] = b'0' + (r % 10);
            let r_str = core::str::from_utf8(&rssi_buf).unwrap_or("R:--%");
            Text::new(r_str, Point::new(65, 9), text_style).draw(lcd).ok();
        }
    } else if is_crsf {
        Text::new("CRSF", Point::new(66, 9), text_style).draw(lcd).ok();
    } else if rf::is_bound() {
        Text::new("RF:OK", Point::new(65, 9), text_style).draw(lcd).ok();
    } else {
        Text::new("NO RF", Point::new(65, 9), text_style).draw(lcd).ok();
    }

    // --- Battery Voltage Alarm & Display (x = 98..127, y = 9) ---
    let mut vbat_buf = [0u8; 6];
    let vbat_str = format_vbat(battery_mv, &mut vbat_buf);
    let vbat_warn_mv = (storage.radio.vbat_warn_deci as u16) * 100;
    let vbat_is_low = battery_mv < vbat_warn_mv;

    if vbat_is_low {
        if *vbat_alarm_timer >= 8000 {
            *vbat_alarm_timer = 0;
            buzzer.warn_battery();
        } else {
            *vbat_alarm_timer += 33;
        }

        if (blink_phase & 0x10) != 0 {
            Rectangle::new(Point::new(97, 0), Size::new(31, 10))
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(lcd)
                .ok();
            let inv_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
            Text::new(vbat_str, Point::new(98, 9), inv_style).draw(lcd).ok();
        } else {
            Text::new(vbat_str, Point::new(98, 9), text_style).draw(lcd).ok();
        }
    } else {
        *vbat_alarm_timer = 7000;
        let bat_icon = Battery::new(BinaryColor::On);
        let _ = Image::new(&bat_icon, Point::new(86, 0)).draw(lcd);
        Text::new(vbat_str, Point::new(98, 9), text_style).draw(lcd).ok();
    }

    // --- Telemetry RSSI Range Alarms & Stats Tracking ---
    if telem.connected {
        if !*telem_seen {
            *telem_seen = true;
            *min_rssi = telem.rssi;
            if telem.rx_voltage_mv > 0 {
                *min_rx_v_mv = telem.rx_voltage_mv;
            }
        } else {
            if telem.rssi < *min_rssi {
                *min_rssi = telem.rssi;
            }
            if telem.rx_voltage_mv > 0 && telem.rx_voltage_mv < *min_rx_v_mv {
                *min_rx_v_mv = telem.rx_voltage_mv;
            }
        }

        if telem.rssi < 20 {
            if *rssi_alarm_timer >= 3000 {
                *rssi_alarm_timer = 0;
                buzzer.warn_rssi_critical();
            } else {
                *rssi_alarm_timer += 33;
            }
        } else if telem.rssi < 40 {
            if *rssi_alarm_timer >= 6000 {
                *rssi_alarm_timer = 0;
                buzzer.warn_rssi_low();
            } else {
                *rssi_alarm_timer += 33;
            }
        } else {
            *rssi_alarm_timer = 0;
        }
    } else {
        *rssi_alarm_timer = 0;
    }

    // Header divider line (y = 11)
    Line::new(Point::new(0, 11), Point::new(127, 11)).into_styled(sep_style).draw(lcd).ok();
}
