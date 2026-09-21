//! Native ExpressLRS / TBS CRSF configuration menu screen.

use embedded_graphics::{
    mono_font::{ascii::FONT_4X6, ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};

use crate::buzzer::Buzzer;
use crate::crsf::{self, ElrsConfigState};
use crate::display::St7567;
use crate::menu::widgets;
use crate::menu::{MenuController, MenuState, NavKeys};

pub fn update(
    ctrl: &mut MenuController,
    lcd: &mut St7567,
    keys: &NavKeys,
    buzzer: &mut Buzzer,
) {
    if keys.cancel {
        ctrl.state = MenuState::RxSetup;
        ctrl.selected_item = 2;
        ctrl.waiting_release = true;
        buzzer.click();
        return;
    }

    let engine = crsf::get_config_engine();
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let text_style_small = MonoTextStyle::new(&FONT_4X6, BinaryColor::On);
    let fill_style = PrimitiveStyle::with_fill(BinaryColor::On);

    match engine.state {
        ElrsConfigState::Idle | ElrsConfigState::Discovering => {
            if keys.ok {
                crsf::start_config();
                buzzer.click();
            }

            widgets::draw_header(lcd, "ELRS CONFIG");
            Text::new("Connecting to module...", Point::new(4, 26), text_style).draw(lcd).ok();
            Text::new("Waiting for ping response", Point::new(4, 38), text_style_small).draw(lcd).ok();

            widgets::draw_footer_small(lcd, "[OK] Retry   [ESC] Back");
        }
        ElrsConfigState::Connected | ElrsConfigState::LoadingParam(_) => {
            widgets::draw_header(lcd, "ELRS CONFIG");
            Text::new("Loading parameters...", Point::new(4, 26), text_style).draw(lcd).ok();

            if let ElrsConfigState::LoadingParam(id) = engine.state {
                let mut pbuf = [b' '; 18];
                pbuf[..6].copy_from_slice(b"Param ");
                pbuf[6] = b'0' + ((id / 10) % 10);
                pbuf[7] = b'0' + (id % 10);
                pbuf[8..12].copy_from_slice(b" of ");
                pbuf[12] = b'0' + ((engine.param_count / 10) % 10);
                pbuf[13] = b'0' + (engine.param_count % 10);
                let p_str = core::str::from_utf8(&pbuf[..14]).unwrap_or("Loading...");
                Text::new(p_str, Point::new(4, 38), text_style_small).draw(lcd).ok();
            }

            widgets::draw_footer_small(lcd, "Please wait...  [ESC] Back");
        }
        ElrsConfigState::Ready => {
            let count = engine.params_len;
            if count == 0 {
                if keys.ok {
                    crsf::start_config();
                    buzzer.click();
                }
                widgets::draw_header(lcd, "ELRS CONFIG");
                Text::new("No parameters found", Point::new(8, 28), text_style).draw(lcd).ok();
                widgets::draw_footer_small(lcd, "[OK] Retry   [ESC] Back");
            } else {
                widgets::navigate_4slot_list(
                    &mut ctrl.selected_item,
                    &mut ctrl.scroll_offset,
                    count,
                    keys.up,
                    keys.down,
                    buzzer,
                );

                if keys.ok {
                    let p = &engine.params[ctrl.selected_item];
                    if p.param_type == crsf::protocol::CRSF_TYPE_SELECT {
                        crsf::cycle_param(ctrl.selected_item);
                        buzzer.click();
                    } else if p.param_type == crsf::protocol::CRSF_TYPE_COMMAND {
                        crsf::trigger_command(ctrl.selected_item);
                        buzzer.play_tone(2400, 80);
                    }
                }

                // Header: device name if available
                let dev_name = if engine.device_name_len > 0 {
                    core::str::from_utf8(&engine.device_name[..engine.device_name_len as usize]).unwrap_or("ELRS SETUP")
                } else {
                    "ELRS SETUP"
                };
                widgets::draw_header(lcd, dev_name);

                // Render up to 4 parameters
                for slot in 0..4 {
                    let idx = ctrl.scroll_offset + slot;
                    if idx >= count {
                        break;
                    }
                    let y = 14 + (slot as i32 * 9);
                    let is_sel = idx == ctrl.selected_item;
                    let style = if is_sel {
                        Rectangle::new(Point::new(2, y), Size::new(124, 9)).into_styled(fill_style).draw(lcd).ok();
                        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off)
                    } else {
                        text_style
                    };

                    let p = &engine.params[idx];
                    let name = core::str::from_utf8(&p.name[..p.name_len as usize]).unwrap_or("Param");

                    if p.param_type == crsf::protocol::CRSF_TYPE_SELECT {
                        Text::new(name, Point::new(4, y + 7), style).draw(lcd).ok();
                        let mut opt_buf = [0u8; 16];
                        let opt = p.current_option_str(&mut opt_buf);
                        Text::new(opt, Point::new(72, y + 7), style).draw(lcd).ok();
                    } else if p.param_type == crsf::protocol::CRSF_TYPE_COMMAND {
                        let mut cmd_buf = [b' '; 20];
                        cmd_buf[0] = b'[';
                        let n_len = (p.name_len as usize).min(16);
                        cmd_buf[1..1 + n_len].copy_from_slice(&p.name[..n_len]);
                        cmd_buf[1 + n_len] = b']';
                        let c_str = core::str::from_utf8(&cmd_buf[..2 + n_len]).unwrap_or("[Cmd]");
                        Text::new(c_str, Point::new(14, y + 7), style).draw(lcd).ok();
                    } else {
                        Text::new(name, Point::new(4, y + 7), style).draw(lcd).ok();
                    }
                }

                widgets::draw_footer_small(lcd, "[OK] Cycle/Run  [ESC] Back");
            }
        }
    }
}
