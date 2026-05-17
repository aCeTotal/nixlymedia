use egui::{Key, Ui};

use crate::app::App;
use crate::input::gamepad::Action;

#[allow(dead_code)]
#[derive(Default, Clone, Copy)]
pub struct Nav {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub enter: bool,
    pub back: bool,
}

pub fn read(app: &mut App, ui: &Ui) -> Nav {
    let (kl, kr, ku, kd, ke, kb) = ui.ctx().input(|i| {
        (
            i.key_pressed(Key::ArrowLeft),
            i.key_pressed(Key::ArrowRight),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
            i.key_pressed(Key::Escape) || i.key_pressed(Key::Backspace),
        )
    });
    let mut n = Nav {
        left: kl,
        right: kr,
        up: ku,
        down: kd,
        enter: ke,
        back: kb,
    };
    for a in app.nav_actions.drain(..) {
        match a {
            Action::Left => n.left = true,
            Action::Right => n.right = true,
            Action::Up => n.up = true,
            Action::Down => n.down = true,
            Action::Confirm => n.enter = true,
            _ => {}
        }
    }
    n
}
