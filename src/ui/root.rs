use egui::{vec2, CentralPanel, Context, Id, UiBuilder};

use crate::app::{App, Screen};
use crate::input::gamepad::Action;
use crate::ui::{detail, grid, player, theme, toast};

pub fn draw(app: &mut App, ctx: &Context) {
    if matches!(app.screen, Screen::Player) {
        player::draw(app, ctx);
        toast::draw(app, ctx);
        return;
    }

    let back = ctx.input(|i| {
        i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Backspace)
    });
    if back {
        app.go_back();
    }

    let frame = match &app.screen {
        Screen::MovieDetail(_) | Screen::TvShowDetail(_) => {
            egui::Frame::none().fill(theme::BG)
        }
        _ => egui::Frame::none()
            .fill(theme::BG)
            .inner_margin(egui::Margin::symmetric(28.0, 20.0)),
    };
    CentralPanel::default()
        .frame(frame)
        .show(ctx, |ui| match &app.screen {
            Screen::MoviesGrid | Screen::TvShowsGrid => draw_library(app, ui),
            Screen::MovieDetail(id) => detail::draw_movie(app, ui, *id),
            Screen::TvShowDetail(key) => detail::draw_tvshow(app, ui, key.clone()),
            Screen::CollectionView(key) => grid::draw_collection(app, ui, key.clone()),
            Screen::Player => {}
        });

    toast::draw(app, ctx);
}

fn draw_library(app: &mut App, ui: &mut egui::Ui) {
    handle_lib_switch(app, ui);

    let target = if matches!(app.screen, Screen::TvShowsGrid) {
        1.0
    } else {
        0.0
    };
    let t = ui
        .ctx()
        .animate_value_with_time(Id::new("lib_slide"), target, 0.38);

    let rect = ui.available_rect_before_wrap();
    let w = rect.width();
    let gap = 60.0;
    let stride = w + gap;
    let movies_rect = rect.translate(vec2(-t * stride, 0.0));
    let tv_rect = rect.translate(vec2((1.0 - t) * stride, 0.0));

    let movies_active = matches!(app.screen, Screen::MoviesGrid);

    let layout = *ui.layout();
    let mut m_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("lib_movies")
            .max_rect(movies_rect)
            .layout(layout),
    );
    m_ui.set_clip_rect(rect);
    grid::draw_movies(app, &mut m_ui, movies_active);

    let mut t_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("lib_tvshows")
            .max_rect(tv_rect)
            .layout(layout),
    );
    t_ui.set_clip_rect(rect);
    grid::draw_tvshows(app, &mut t_ui, !movies_active);

    if (t - target).abs() > 0.001 {
        ui.ctx().request_repaint();
    }
}

fn handle_lib_switch(app: &mut App, ui: &egui::Ui) {
    let mut prev = false;
    let mut next = false;
    app.nav_actions.retain(|a| match a {
        Action::LibPrev => {
            prev = true;
            false
        }
        Action::LibNext => {
            next = true;
            false
        }
        _ => true,
    });
    let (kp, kn) = ui.ctx().input(|i| {
        (
            i.key_pressed(egui::Key::PageUp) || i.key_pressed(egui::Key::Q),
            i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::E),
        )
    });
    prev |= kp;
    next |= kn;
    match app.screen {
        Screen::MoviesGrid if next => {
            app.switch_lib(Screen::TvShowsGrid);
            if !app.tvshows_loaded {
                app.fetch_tvshows();
            }
        }
        Screen::TvShowsGrid if prev => {
            app.switch_lib(Screen::MoviesGrid);
            if !app.movies_loaded {
                app.fetch_movies();
            }
        }
        _ => {}
    }
}
