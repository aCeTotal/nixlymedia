use gilrs::{Button, EventType, Gilrs};

use crate::app::{App, Screen};

pub struct Gamepad {
    gilrs: Option<Gilrs>,
    pub events: Vec<Action>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    PlayPause,
    SubsMenu,
    Skip(f64),
    Menu,
}

impl Gamepad {
    pub fn new() -> Self {
        Self {
            gilrs: Gilrs::new().ok(),
            events: Vec::new(),
        }
    }

    pub fn poll(&mut self) {
        self.events.clear();
        let Some(g) = &mut self.gilrs else { return };
        while let Some(ev) = g.next_event() {
            if let EventType::ButtonPressed(btn, _) = ev.event {
                match btn {
                    Button::DPadUp => self.events.push(Action::Up),
                    Button::DPadDown => self.events.push(Action::Down),
                    Button::DPadLeft => self.events.push(Action::Left),
                    Button::DPadRight => self.events.push(Action::Right),
                    Button::South => self.events.push(Action::Confirm),
                    Button::East => self.events.push(Action::Back),
                    Button::West => self.events.push(Action::PlayPause),
                    Button::North => self.events.push(Action::SubsMenu),
                    Button::LeftTrigger | Button::LeftTrigger2 => {
                        self.events.push(Action::Skip(-10.0))
                    }
                    Button::RightTrigger | Button::RightTrigger2 => {
                        self.events.push(Action::Skip(10.0))
                    }
                    Button::Start => self.events.push(Action::Menu),
                    _ => {}
                }
            }
        }
    }
}

pub fn route(app: &mut App) {
    let actions: Vec<Action> = app.gamepad.events.drain(..).collect();
    for a in actions {
        if matches!(app.screen, Screen::Player) {
            app.player.nudge_controls();
            if app.player.popup.is_some() {
                match a {
                    Action::Up => app.player.popup_up(),
                    Action::Down => app.player.popup_down(),
                    Action::Confirm => app.player.popup_confirm(),
                    Action::Back => app.player.close_popup(),
                    Action::PlayPause => app.player.toggle_pause(),
                    Action::Skip(s) => app.player.seek(s),
                    _ => {}
                }
            } else {
                match a {
                    Action::Left => app.player.focus_left(),
                    Action::Right => app.player.focus_right(),
                    Action::Confirm => crate::ui::player::activate_focused(app),
                    Action::PlayPause => app.player.toggle_pause(),
                    Action::Skip(s) => app.player.seek(s),
                    Action::SubsMenu => {
                        app.player.open_popup(crate::player::mpv::TrackKind::Sub);
                    }
                    Action::Back => crate::ui::player::stop_and_return(app),
                    Action::Menu => app.player.nudge_controls(),
                    _ => {}
                }
            }
        } else {
            match a {
                Action::Back => app.go_back(),
                Action::Up
                | Action::Down
                | Action::Left
                | Action::Right
                | Action::Confirm => app.nav_actions.push(a),
                _ => {}
            }
        }
    }
}
