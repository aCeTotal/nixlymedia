use std::sync::{mpsc, Arc};

use gilrs::{Button, EventType, Gilrs};
use parking_lot::Mutex;

use crate::app::{App, Screen};

pub struct Gamepad {
    rx: Option<mpsc::Receiver<Action>>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
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

fn map_button(btn: Button) -> Option<Action> {
    Some(match btn {
        Button::DPadUp => Action::Up,
        Button::DPadDown => Action::Down,
        Button::DPadLeft => Action::Left,
        Button::DPadRight => Action::Right,
        Button::South => Action::Confirm,
        Button::East => Action::Back,
        Button::West => Action::PlayPause,
        Button::North => Action::SubsMenu,
        Button::LeftTrigger | Button::LeftTrigger2 => Action::Skip(-10.0),
        Button::RightTrigger | Button::RightTrigger2 => Action::Skip(10.0),
        Button::Start => Action::Menu,
        _ => return None,
    })
}

impl Gamepad {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
        let ctx_thread = ctx.clone();
        let spawn = std::thread::Builder::new()
            .name("nixlymedia-gamepad".into())
            .spawn(move || {
                let mut gilrs = match Gilrs::new() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                loop {
                    let mut got_any = false;
                    while let Some(ev) = gilrs.next_event() {
                        if let EventType::ButtonPressed(btn, _) = ev.event {
                            if let Some(action) = map_button(btn) {
                                if tx.send(action).is_err() {
                                    return;
                                }
                                got_any = true;
                            }
                        }
                    }
                    if got_any {
                        if let Some(ctx) = ctx_thread.lock().as_ref() {
                            ctx.request_repaint();
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            });
        let rx = match spawn {
            Ok(_) => Some(rx),
            Err(_) => None,
        };
        Self {
            rx,
            ctx,
            events: Vec::new(),
        }
    }

    pub fn attach_ctx(&self, ctx: &egui::Context) {
        let mut g = self.ctx.lock();
        if g.is_none() {
            *g = Some(ctx.clone());
        }
    }

    pub fn poll(&mut self) {
        self.events.clear();
        if let Some(rx) = &self.rx {
            while let Ok(action) = rx.try_recv() {
                self.events.push(action);
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
