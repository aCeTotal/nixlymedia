use egui::{CentralPanel, Context};

use crate::app::{App, Screen};
use crate::ui::{detail, grid, home, player, theme, toast};

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
            Screen::Home => home::draw(app, ui),
            Screen::MoviesGrid => grid::draw_movies(app, ui),
            Screen::TvShowsGrid => grid::draw_tvshows(app, ui),
            Screen::MovieDetail(id) => detail::draw_movie(app, ui, *id),
            Screen::TvShowDetail(key) => detail::draw_tvshow(app, ui, key.clone()),
            Screen::CollectionView(key) => grid::draw_collection(app, ui, key.clone()),
            Screen::Player => {}
        });

    toast::draw(app, ctx);
}
