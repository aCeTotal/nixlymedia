use egui::{
    pos2, vec2, Align2, CentralPanel, Color32, Context, FontId, Key, Rect, Sense, Stroke,
};

use crate::app::{App, Screen, TvColumn};
use crate::player::mpv::TrackKind;
use crate::player::view::{Control, Origin, Phase, CONTROLS};
use crate::ui::progress;
use crate::ui::theme;

/* Neste-episode countdown: badge dukker opp når 40 s gjenstår og teller
 * 10 → 0; ved 0 byttes det direkte til neste episode. */
const AUTO_NEXT_TRIGGER_SECS: f64 = 40.0;
const AUTO_NEXT_COUNTDOWN_SECS: u64 = 10;
const WATCHED_SAVE_EVERY_SECS: u64 = 5;

pub fn draw(app: &mut App, ctx: &Context) {
    /* Watchdog kan ha bedt om full restart (frossen pipeline reload ikke
     * tinte). Kjør FØR poll så en fersk spiller bygges denne framen. */
    if let Some(mpv) = app.player.mpv.clone() {
        if let Some(pos) = mpv.take_restart_request() {
            app.player.restart_current(&app.api, pos);
        }
    }
    app.player.poll();
    /* Spol-tick før input/draw: bruker forventer at hver frame mens en
     * skulderknapp holdes inne hopper videre. Kall etter poll() så fasen
     * er fersk og start_seek_hold sin Phase::Playing-gate stemmer. */
    app.player.tick_seek_hold();
    /* Re-sjekk synlighets-overgang så fokus resettes hver gang baren
     * dukker opp igjen, uavhengig av om aktiviteten kom fra mus, tast
     * eller håndkontroller. */
    app.player.sync_controls_visibility();
    handle_keys(app, ctx);

    /* Watched-lagring + auto-next-tikk FØR panelet åpnes, samme sted som
     * Escape-veien over. Nedtellingen kan ende i start() eller
     * stop_and_return, og begge joiner render-tråden og river ned EGL-
     * context + wayland-objekter (subsurface, wp_content_type på PARENT-
     * surfacet — som er egui sitt eget toplevel). Det skal ikke skje midt
     * inne i en åpen CentralPanel-closure mens egui bygger frame'n. */
    if matches!(app.player.phase(), Phase::Playing) {
        tick_watched_and_autonext(app);
    }

    if let Some(mpv) = app.player.mpv.clone() {
        mpv.attach_repaint(ctx);
    }

    /* Aktiv spol-tilstand = be om kontinuerlig repaint slik at
     * tick_seek_hold fyrer selv mellom gamepad-events. mpv pauset
     * gir ingen render-callback ellers. */
    if app.player.is_seeking() {
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    }

    let activity = ctx.input(|i| {
        let moved = i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_)));
        let pressed = i.pointer.any_pressed();
        let keyed = i.events.iter().any(|e| {
            matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))
        });
        let scrolled = i.raw_scroll_delta != egui::Vec2::ZERO;
        moved || pressed || keyed || scrolled
    });
    if activity {
        app.player.nudge_controls();
    }

    CentralPanel::default()
        .frame(
            egui::Frame::none()
                .fill(Color32::TRANSPARENT)
                .inner_margin(egui::Margin::ZERO)
                .outer_margin(egui::Margin::ZERO),
        )
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            let resp = ui.interact(rect, ui.id().with("player_bg"), Sense::click_and_drag());
            if resp.clicked() && app.player.popup.is_none() {
                app.player.toggle_pause();
            }

            let phase = app.player.phase();
            match phase {
                Phase::Probing | Phase::Preloading => progress::draw(ui, &app.player),
                Phase::Error => {
                    let err = app
                        .player
                        .error
                        .lock()
                        .clone()
                        .unwrap_or_else(|| "Ukjent feil".into());
                    center_text(ui, &format!("Avspilling feilet: {err}"), theme::BAD);
                }
                Phase::Idle => center_text(ui, "Ingen film valgt", theme::TEXT_DIM),
                Phase::Playing => {}
            }

            if matches!(phase, Phase::Playing) {
                if app.player.controls_visible() {
                    draw_controls(app, ui);
                }
                if app.player.auto_next_started_at.is_some() {
                    draw_auto_next_badge(app, ui);
                }
            }
            if app.player.popup.is_some() {
                draw_popup(app, ui);
            }
        });
}

fn tick_watched_and_autonext(app: &mut App) {
    let Some(mpv) = app.player.mpv.clone() else { return };
    let Some(id) = app.player.media_id else { return };
    let pos = mpv.time_pos().unwrap_or(0.0);
    let dur = mpv.duration().unwrap_or(0.0);
    if dur <= 0.0 {
        return;
    }
    app.watched.update(id, pos, dur);
    if app.player.last_watched_save.elapsed().as_secs() >= WATCHED_SAVE_EVERY_SECS {
        app.watched.flush();
        app.player.last_watched_save = std::time::Instant::now();
    }

    let is_episode = matches!(app.player.origin, Some(Origin::Episode { .. }));
    let remaining = dur - pos;
    let paused = mpv.paused();
    let arming = is_episode && remaining > 0.0 && remaining <= AUTO_NEXT_TRIGGER_SECS && !paused;

    if !arming {
        app.player.auto_next_started_at = None;
        return;
    }
    if app.player.auto_next_started_at.is_none() {
        app.player.auto_next_started_at = Some(std::time::Instant::now());
    }
    if let Some(t0) = app.player.auto_next_started_at {
        if t0.elapsed().as_secs() >= AUTO_NEXT_COUNTDOWN_SECS {
            app.player.auto_next_started_at = None;
            match next_up(app) {
                NextUp::Episode(..) => {
                    play_next_episode(app);
                }
                /* Siste tilgjengelige episode: badgen har vist "No more
                 * episodes" gjennom nedtellingen — lukk spilleren istf å
                 * nullstille og telle ned på nytt i det uendelige. */
                NextUp::NoMore => stop_and_return(app),
                /* Vet ikke: la episoden spille ut i fred. */
                NextUp::Unknown => {}
            }
        }
    }
}

/* Hva som venter etter denne episoden. Unknown = serie-/episodelisten er
 * ikke lastet (API-hikke), og da skal vi verken love neste episode eller
 * påstå at serien er slutt — "ingen data" er ikke det samme som "ingen
 * flere episoder". */
enum NextUp {
    Episode(crate::api::types::Episode, i32, usize),
    NoMore,
    Unknown,
}

fn next_up(app: &App) -> NextUp {
    let Some(Origin::Episode { show_key, season, episode_idx }) = app.player.origin.clone() else {
        return NextUp::Unknown;
    };
    let key = show_key;
    let Some(same_season) = app.episodes.get(&(key.clone(), season)) else {
        return NextUp::Unknown;
    };
    if let Some(ep) = same_season.get(episode_idx + 1) {
        return NextUp::Episode(ep.clone(), season, episode_idx + 1);
    }
    let Some(seasons_list) = app.seasons.get(&key) else {
        return NextUp::Unknown;
    };
    let Some(pos) = seasons_list.iter().position(|s| s.season == season) else {
        return NextUp::Unknown;
    };
    let Some(next_s) = seasons_list.get(pos + 1) else {
        return NextUp::NoMore;
    };
    match app.episodes.get(&(key.clone(), next_s.season)) {
        Some(eps) => match eps.first() {
            Some(ep) => NextUp::Episode(ep.clone(), next_s.season, 0),
            None => NextUp::NoMore,
        },
        None => NextUp::Unknown,
    }
}

fn play_next_episode(app: &mut App) -> bool {
    let Some(Origin::Episode { show_key, .. }) = app.player.origin.clone() else {
        return false;
    };
    let key = show_key;
    let NextUp::Episode(ep, new_season, new_idx) = next_up(app) else { return false };
    let title = ep
        .episode_title
        .clone()
        .or(ep.tmdb_title.clone())
        .unwrap_or_else(|| ep.title.clone());
    let origin = Origin::Episode {
        show_key: key,
        season: new_season,
        episode_idx: new_idx,
    };
    /* Behold display-refresh over episode-byttet: neste episode har samme
     * kilde-fps, så undertrykk VideoStopped fra teardown av denne spilleren.
     * Uten dette restorer nixlytile max refresh og judrer til ny render-tråd
     * re-sender VideoPlaying + TV bytter mode på nytt. */
    if let Some(mpv) = &app.player.mpv {
        mpv.suppress_stopped_ipc();
    }
    app.player
        .start(&app.api, ep.id, &title, 0, ep.duration, origin, 0.0);
    true
}

fn draw_auto_next_badge(app: &App, ui: &mut egui::Ui) {
    let Some(t0) = app.player.auto_next_started_at else { return };
    let elapsed = t0.elapsed().as_secs_f64();
    let left = ((AUTO_NEXT_COUNTDOWN_SECS as f64) - elapsed).max(0.0).ceil() as i64;
    let rect = ui.max_rect();
    let w = 260.0;
    let h = 72.0;
    let box_rect = Rect::from_min_size(
        pos2(rect.right() - w - 28.0, rect.bottom() - h - 28.0),
        vec2(w, h),
    );
    ui.painter().rect_filled(
        box_rect,
        12.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 220),
    );
    ui.painter()
        .rect_stroke(box_rect, 12.0, Stroke::new(1.5, theme::ACCENT));
    /* Siste tilgjengelige episode: samme nedtelling, men den ender i at
     * spilleren lukkes — si det, ikke lov en episode som ikke finnes.
     * Unknown teller som "neste": vi vet ikke at serien er slutt. */
    let has_next = !matches!(next_up(app), NextUp::NoMore);
    ui.painter().text(
        box_rect.left_top() + vec2(18.0, 14.0),
        Align2::LEFT_TOP,
        if has_next { "Neste episode" } else { "No more episodes" },
        FontId::proportional(14.0),
        theme::TEXT,
    );
    ui.painter().text(
        box_rect.left_top() + vec2(18.0, 38.0),
        Align2::LEFT_TOP,
        if has_next {
            format!("om {} s", left)
        } else {
            format!("lukker om {} s", left)
        },
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );
    ui.painter().text(
        box_rect.right_center() + vec2(-32.0, 0.0),
        Align2::CENTER_CENTER,
        format!("{left}"),
        FontId::proportional(30.0),
        theme::ACCENT,
    );
}

pub fn handle_keys(app: &mut App, ctx: &Context) {
    let (left, right, up, down, enter, escape, ad_dec, ad_inc, shift) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowLeft),
            i.key_pressed(Key::ArrowRight),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
            i.key_pressed(Key::Escape),
            i.key_pressed(Key::OpenBracket),
            i.key_pressed(Key::CloseBracket),
            i.modifiers.shift,
        )
    });

    /* Audio-delay finjustering. [ / ] = ±10 ms; Shift+[ / Shift+] = ±50 ms.
     * Positive = lyd senere (lyd lå foran video). Verdien logges så bruker
     * ser hva som ble truffet og kan flytte default i config etterpå. */
    if ad_dec || ad_inc {
        let step = if shift { 0.050 } else { 0.010 };
        let delta = if ad_dec { -step } else { step };
        if let Some(mpv) = &app.player.mpv {
            let new_val = mpv.adjust_audio_delay(delta);
            crate::nlog!("audio-delay = {:.3} s", new_val);
        }
        app.player.nudge_controls();
    }

    if app.player.popup.is_some() {
        if up {
            app.player.popup_up();
        }
        if down {
            app.player.popup_down();
        }
        if enter {
            app.player.popup_confirm();
        }
        if escape {
            app.player.close_popup();
        }
    } else {
        if left {
            app.player.focus_left();
        }
        if right {
            app.player.focus_right();
        }
        if enter {
            activate_focused(app);
        }
        if escape {
            stop_and_return(app);
        }
    }
}

pub fn activate_focused(app: &mut App) {
    match app.player.focused_control() {
        Control::Stop => stop_and_return(app),
        Control::SkipBack => app.player.seek(-10.0),
        Control::PlayPause => app.player.toggle_pause(),
        Control::SkipFwd => app.player.seek(10.0),
        Control::Subs => app.player.open_popup(TrackKind::Sub),
        Control::Audio => app.player.open_popup(TrackKind::Audio),
    }
}

fn center_text(ui: &mut egui::Ui, text: &str, color: Color32) {
    let rect = ui.max_rect();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::proportional(18.0),
        color,
    );
}

fn draw_controls(app: &mut App, ui: &mut egui::Ui) {
    let rect = ui.max_rect();

    let top_h = 64.0;
    let top_rect = Rect::from_min_size(rect.left_top(), vec2(rect.width(), top_h));
    ui.painter().rect_filled(top_rect, 0.0, Color32::from_rgba_premultiplied(0, 0, 0, 180));
    ui.painter().text(
        top_rect.left_top() + vec2(28.0, 20.0),
        egui::Align2::LEFT_TOP,
        &app.player.title,
        FontId::proportional(20.0),
        theme::TEXT,
    );

    let close_rect = Rect::from_min_size(top_rect.right_top() - vec2(60.0, -16.0), vec2(44.0, 32.0));
    let close_resp = ui.interact(close_rect, ui.id().with("player_close"), Sense::click());
    let close_bg = if close_resp.hovered() { theme::BAD } else { Color32::from_black_alpha(0) };
    ui.painter().rect_filled(close_rect, 6.0, close_bg);
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "✕",
        FontId::proportional(20.0),
        theme::TEXT,
    );
    if close_resp.clicked() {
        stop_and_return(app);
        return;
    }

    let bottom_h = 130.0;
    let bottom_rect = Rect::from_min_size(
        pos2(rect.left(), rect.bottom() - bottom_h),
        vec2(rect.width(), bottom_h),
    );
    ui.painter().rect_filled(bottom_rect, 0.0, Color32::from_rgba_premultiplied(0, 0, 0, 210));

    let (pos_s, dur_s) = if let Some(mpv) = &app.player.mpv {
        (mpv.time_pos().unwrap_or(0.0), mpv.duration().unwrap_or(0.0))
    } else {
        (0.0, 0.0)
    };

    let bar_pad = 32.0;
    let bar_rect = Rect::from_min_max(
        pos2(bottom_rect.left() + bar_pad, bottom_rect.top() + 22.0),
        pos2(bottom_rect.right() - bar_pad, bottom_rect.top() + 32.0),
    );
    ui.painter().rect_filled(bar_rect, 5.0, theme::PANEL);
    let frac = if dur_s > 0.0 { (pos_s / dur_s).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let fill = Rect::from_min_max(
        bar_rect.left_top(),
        pos2(bar_rect.left() + bar_rect.width() * frac, bar_rect.bottom()),
    );
    ui.painter().rect_filled(fill, 5.0, theme::ACCENT);

    let bar_resp = ui.interact(
        bar_rect.expand2(vec2(0.0, 10.0)),
        ui.id().with("seek_bar"),
        Sense::click_and_drag(),
    );
    if let Some(p) = bar_resp.interact_pointer_pos() {
        if bar_resp.dragged() || bar_resp.clicked() {
            let rel = ((p.x - bar_rect.left()) / bar_rect.width()).clamp(0.0, 1.0);
            if let Some(mpv) = &app.player.mpv {
                let target = (rel as f64) * dur_s;
                mpv.seek(target - pos_s);
            }
        }
    }

    let time_y = bottom_rect.top() + 46.0;
    ui.painter().text(
        pos2(bar_rect.left(), time_y),
        egui::Align2::LEFT_TOP,
        fmt_time(pos_s),
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );
    ui.painter().text(
        pos2(bar_rect.right(), time_y),
        egui::Align2::RIGHT_TOP,
        fmt_time(dur_s),
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );

    let btn_size = vec2(70.0, 46.0);
    let btn_y = bottom_rect.top() + 72.0;
    let gap = 12.0;
    let total_w = CONTROLS.len() as f32 * btn_size.x + (CONTROLS.len() as f32 - 1.0) * gap;
    let start_x = bottom_rect.center().x - total_w / 2.0;

    let focused = app.player.control_focus;
    let paused = app.player.mpv.as_ref().map(|m| m.paused()).unwrap_or(true);

    let mut activated: Option<Control> = None;
    let mut hover_idx: Option<usize> = None;

    for (i, c) in CONTROLS.iter().copied().enumerate() {
        let r = Rect::from_min_size(pos2(start_x + i as f32 * (btn_size.x + gap), btn_y), btn_size);
        let id = ui.id().with(("ctrl", i));
        let rsp = ui.interact(r, id, Sense::click());
        let is_focus = i == focused;
        let is_stop = c == Control::Stop;

        let bg = if is_stop && (rsp.hovered() || is_focus) {
            theme::BAD
        } else if is_focus {
            theme::ACCENT_SOFT
        } else if rsp.hovered() {
            theme::PANEL_HOVER
        } else {
            theme::PANEL
        };
        ui.painter().rect_filled(r, 10.0, bg);
        if is_focus {
            ui.painter().rect_stroke(r, 10.0, Stroke::new(2.5, theme::ACCENT));
        }
        let label = match c {
            Control::Stop => "■",
            Control::SkipBack => "⏮",
            Control::PlayPause => if paused { "▶" } else { "⏸" },
            Control::SkipFwd => "⏭",
            Control::Subs => "Subs",
            Control::Audio => "Audio",
        };
        let font = match c {
            Control::Subs | Control::Audio => FontId::proportional(15.0),
            _ => FontId::proportional(22.0),
        };
        let txt_color = if is_focus && !is_stop { theme::BG } else { theme::TEXT };
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, font, txt_color);

        if rsp.hovered() {
            hover_idx = Some(i);
        }
        if rsp.clicked() {
            activated = Some(c);
            app.player.control_focus = i;
        }
    }

    let pointer_moved = ui
        .ctx()
        .input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    if let Some(i) = hover_idx {
        if app.player.popup.is_none() && pointer_moved {
            app.player.control_focus = i;
        }
    }

    if let Some(c) = activated {
        match c {
            Control::Stop => {
                stop_and_return(app);
                return;
            }
            Control::SkipBack => app.player.seek(-10.0),
            Control::PlayPause => app.player.toggle_pause(),
            Control::SkipFwd => app.player.seek(10.0),
            Control::Subs => app.player.open_popup(TrackKind::Sub),
            Control::Audio => app.player.open_popup(TrackKind::Audio),
        }
    }
}

fn draw_popup(app: &mut App, ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    let Some(popup) = &app.player.popup else { return };
    let kind_label = match popup.kind {
        TrackKind::Sub => "Undertekst",
        TrackKind::Audio => "Lyd",
    };

    let row_h = 36.0;
    let title_h = 52.0;
    let bottom_pad = 14.0;
    let h_pad = 12.0;
    let label_font = FontId::proportional(14.0);
    let title_font = FontId::proportional(18.0);

    let max_text_w = (rect.width() - 160.0).max(220.0);
    let widest_track = popup
        .tracks
        .iter()
        .map(|t| {
            let s = format!("● {}", t.display());
            ui.fonts(|f| f.layout_no_wrap(s, label_font.clone(), theme::TEXT).size().x)
        })
        .fold(0.0_f32, f32::max);
    let title_w = ui.fonts(|f| {
        f.layout_no_wrap(kind_label.to_string(), title_font.clone(), theme::TEXT)
            .size()
            .x
    });
    let content_w = widest_track.max(title_w).min(max_text_w);
    let panel_w = (content_w + (h_pad + 12.0) * 2.0)
        .max(280.0)
        .min(rect.width() - 80.0);

    let max_panel_h = (rect.height() - 220.0).max(200.0);
    let row_count = popup.tracks.len().max(1);
    let needed_h = row_count as f32 * row_h + title_h + bottom_pad;
    let panel_h = needed_h.min(max_panel_h);
    let visible_rows = (((panel_h - title_h - bottom_pad) / row_h).floor() as usize).max(1);

    let focused_idx = app.player.control_focus.min(CONTROLS.len() - 1);
    let btn_size = vec2(70.0, 46.0);
    let gap = 12.0;
    let total_w = CONTROLS.len() as f32 * btn_size.x + (CONTROLS.len() as f32 - 1.0) * gap;
    let start_x = rect.center().x - total_w / 2.0;
    let anchor_x = start_x + focused_idx as f32 * (btn_size.x + gap) + btn_size.x / 2.0;
    let bottom_of_btns = rect.bottom() - 130.0 + 72.0;

    let panel_left = (anchor_x - panel_w / 2.0)
        .max(rect.left() + 20.0)
        .min(rect.right() - panel_w - 20.0);
    let panel_top = bottom_of_btns - panel_h - 16.0;
    let panel = Rect::from_min_size(pos2(panel_left, panel_top), vec2(panel_w, panel_h));

    ui.painter().rect_filled(
        panel,
        theme::RADIUS,
        Color32::from_rgba_premultiplied(15, 18, 28, 240),
    );
    ui.painter()
        .rect_stroke(panel, theme::RADIUS, Stroke::new(1.5, theme::ACCENT));

    ui.painter().text(
        pos2(panel.left() + 18.0, panel.top() + 18.0),
        egui::Align2::LEFT_TOP,
        kind_label,
        title_font,
        theme::TEXT,
    );

    let list_top = panel.top() + title_h;
    let list_bottom = panel.bottom() - bottom_pad;
    let clip_rect = Rect::from_min_max(
        pos2(panel.left() + h_pad, list_top),
        pos2(panel.right() - h_pad, list_bottom),
    );
    let mut activate: Option<()> = None;
    let mut hover_idx: Option<usize> = None;

    if popup.tracks.is_empty() {
        ui.painter().text(
            pos2(panel.center().x, panel.center().y),
            egui::Align2::CENTER_CENTER,
            "Ingen spor",
            FontId::proportional(14.0),
            theme::TEXT_DIM,
        );
    } else {
        let tracks = popup.tracks.clone();
        let cur_idx = popup.idx;
        let total = tracks.len();
        let half = visible_rows / 2;
        let scroll_start = if total <= visible_rows {
            0
        } else if cur_idx < half {
            0
        } else if cur_idx + visible_rows - half >= total {
            total - visible_rows
        } else {
            cur_idx - half
        };
        let scroll_end = (scroll_start + visible_rows).min(total);

        let row_painter = ui.painter_at(clip_rect);
        for (slot, i) in (scroll_start..scroll_end).enumerate() {
            let t = &tracks[i];
            let r = Rect::from_min_size(
                pos2(panel.left() + h_pad, list_top + slot as f32 * row_h),
                vec2(panel.width() - h_pad * 2.0, row_h - 4.0),
            );
            let id = ui.id().with(("popup", i));
            let rsp = ui.interact(r, id, Sense::click());
            let is_focus = i == cur_idx;
            let bg = if is_focus {
                theme::ACCENT_SOFT
            } else if rsp.hovered() {
                theme::PANEL_HOVER
            } else {
                theme::PANEL
            };
            row_painter.rect_filled(r, 8.0, bg);
            let txt_color = if is_focus { theme::BG } else { theme::TEXT };
            let prefix = if t.selected { "● " } else { "  " };
            let full = format!("{prefix}{}", t.display());
            let text_max_w = r.width() - 24.0;
            let display_str = elide_to_width(ui, &full, &label_font, text_max_w);
            let galley = ui.fonts(|f| {
                f.layout_no_wrap(display_str, label_font.clone(), txt_color)
            });
            row_painter.galley(
                pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0),
                galley,
                txt_color,
            );
            if rsp.hovered() {
                hover_idx = Some(i);
            }
            if rsp.clicked() {
                activate = Some(());
                if let Some(p) = &mut app.player.popup {
                    p.idx = i;
                }
            }
        }

        if total > visible_rows {
            if scroll_start > 0 {
                ui.painter().text(
                    pos2(panel.right() - 14.0, list_top - 4.0),
                    egui::Align2::RIGHT_BOTTOM,
                    "▲",
                    FontId::proportional(11.0),
                    theme::TEXT_DIM,
                );
            }
            if scroll_end < total {
                ui.painter().text(
                    pos2(panel.right() - 14.0, list_bottom + 4.0),
                    egui::Align2::RIGHT_TOP,
                    "▼",
                    FontId::proportional(11.0),
                    theme::TEXT_DIM,
                );
            }
        }
    }

    let pointer_moved = ui.ctx().input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    if let Some(i) = hover_idx {
        if pointer_moved {
            if let Some(p) = &mut app.player.popup {
                p.idx = i;
            }
        }
    }
    if activate.is_some() {
        app.player.popup_confirm();
    }
}

pub fn stop_and_return(app: &mut App) {
    if let (Some(id), Some(mpv)) = (app.player.media_id, app.player.mpv.clone()) {
        let pos = mpv.time_pos().unwrap_or(0.0);
        let dur = mpv.duration().unwrap_or(0.0);
        if dur > 0.0 {
            app.watched.update(id, pos, dur);
        }
    }
    app.watched.flush();
    let origin = app.player.origin.clone();
    app.teardown_hdr_surface();
    app.player.shutdown();
    app.subsurface = None;
    match origin {
        Some(Origin::Episode {
            show_key,
            season,
            episode_idx,
        }) => {
            let season_idx = app
                .seasons
                .get(&show_key)
                .and_then(|v| v.iter().position(|s| s.season == season))
                .unwrap_or(0);
            app.tv_focus.switch_show(&show_key);
            app.tv_focus.season_idx = season_idx;
            app.tv_focus.episode_idx = episode_idx;
            app.tv_focus.column = TvColumn::Episodes;
            /* La seasons::auto_select_next flytte fokus til neste usette ut
             * fra oppdatert watched-state (over er bare fallback). */
            app.tv_focus.auto_selected = false;
            app.selected_season.insert(show_key.clone(), season);
            while !app.history.is_empty()
                && !matches!(app.history.last(), Some(Screen::TvShowDetail(_)))
            {
                app.history.pop();
            }
            if app
                .history
                .last()
                .map(|s| matches!(s, Screen::TvShowDetail(_)))
                .unwrap_or(false)
            {
                app.go_back();
            } else {
                app.screen = Screen::TvShowDetail(show_key);
            }
        }
        Some(Origin::Movie(id)) => {
            while !app.history.is_empty()
                && !matches!(app.history.last(), Some(Screen::MovieDetail(_)))
            {
                app.history.pop();
            }
            if app
                .history
                .last()
                .map(|s| matches!(s, Screen::MovieDetail(_)))
                .unwrap_or(false)
            {
                app.go_back();
            } else {
                app.screen = Screen::MovieDetail(id);
            }
        }
        None => app.go_back(),
    }
}

fn elide_to_width(ui: &egui::Ui, s: &str, font: &FontId, max_w: f32) -> String {
    let measure = |t: &str| -> f32 {
        ui.fonts(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x)
    };
    if measure(s) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let cand: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&cand) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut out: String = chars[..lo].iter().collect();
    out.push('…');
    out
}

fn fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "00:00".into();
    }
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
