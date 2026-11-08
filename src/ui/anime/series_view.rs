use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{pos2, vec2, Align2, Color32, FontId, Id, Painter, Pos2, Rect, Stroke, Ui};

use crate::anime::model::{Episode, Season};
use crate::anime::next::{self, Kind, Resolve};
use crate::anime::state::{Load, SeriesPage};
use crate::app::App;
use crate::ui::detail::{paint_cover, paint_left_shade, paint_vertical_shade};
use crate::ui::theme;

use super::rail::{self, Rail};
use super::row::WIDE_PX;
use super::series::Zone;
use super::tile::{self, Art, Slot, Text};

const HERO_H: f32 = 600.0;
const PAD_X: f32 = 80.0;
const TEXT_W: f32 = 900.0;
const BUTTON_W: f32 = 420.0;
const BUTTON_H: f32 = 58.0;
const CHIP_H: f32 = 44.0;
const EP_W: f32 = 340.0;
const EP_GAP: f32 = 26.0;
const EP_H: f32 = EP_W * 9.0 / 16.0 + 64.0;
const HERO_PX: u32 = 1920;

struct Button<'a> {
    rect: Rect,
    text: &'a str,
    focused: bool,
}

struct Chips<'a> {
    seasons: &'a [Season],
    current: usize,
    active: bool,
}

/* One season's episode rail. */
struct Strip<'a> {
    page: &'a SeriesPage,
    list: &'a [Episode],
    rect: Rect,
}

pub fn draw(app: &App, ui: &mut Ui, id: &str) {
    let Some(Load::Ready(page)) = app.anime.series.get(id) else {
        return;
    };
    let resolved = super::series::resolve(app, id);
    let full = ui.max_rect();
    let painter = ui.painter();
    let hero = Rect::from_min_size(full.min, vec2(full.width(), HERO_H));
    if let Some(tex) = page
        .series
        .images
        .wide(HERO_PX)
        .and_then(|u| app.images.get(u))
    {
        paint_cover(painter, &tex, hero);
    }
    paint_left_shade(painter, hero);
    paint_vertical_shade(painter, hero);

    let x = full.left() + PAD_X;
    let title = Text {
        text: &page.series.title,
        size: 46.0,
        color: theme::TEXT,
        max_w: TEXT_W,
    };
    tile::line(painter, pos2(x, full.top() + 110.0), title);
    let meta = meta_line(page);
    let meta = Text {
        text: &meta,
        size: 19.0,
        color: theme::TEXT_DIM,
        max_w: TEXT_W,
    };
    tile::line(painter, pos2(x, full.top() + 176.0), meta);
    paragraph(
        painter,
        pos2(x, full.top() + 216.0),
        &page.series.description,
    );

    let f = &app.anime_ui.series;
    let mut y = hero.bottom() - 150.0;
    let label = play_label(page, app, resolved.as_ref());
    let rect = Rect::from_min_size(pos2(x, y), vec2(BUTTON_W, BUTTON_H));
    button(
        painter,
        Button {
            rect,
            text: &label,
            focused: f.zone == Zone::Play,
        },
    );
    y += BUTTON_H + 30.0;
    if page.seasons.len() > 1 {
        let chips = Chips {
            seasons: &page.seasons,
            current: f.season,
            active: f.zone == Zone::Seasons,
        };
        seasons(painter, pos2(x, y), chips);
        y += CHIP_H + 26.0;
    }
    let Some(season) = page.seasons.get(f.season) else {
        return;
    };
    let rect = Rect::from_min_max(pos2(x, y), pos2(full.right(), y + EP_H));
    let status = match app.anime.episodes.get(&season.id) {
        Some(Load::Ready(list)) => return episodes(app, ui, Strip { page, list, rect }),
        Some(Load::Failed(e)) => Text {
            text: e,
            size: 18.0,
            color: theme::BAD,
            max_w: TEXT_W,
        },
        _ => Text {
            text: "Henter episoder…",
            size: 18.0,
            color: theme::TEXT_DIM,
            max_w: TEXT_W,
        },
    };
    tile::line(painter, rect.min, status);
}

fn meta_line(page: &SeriesPage) -> String {
    let m = page.series.meta();
    let count = page.seasons.len();
    let mut parts: Vec<String> = m.series_launch_year.iter().map(u32::to_string).collect();
    parts.push(format!(
        "{count} sesong{}",
        if count == 1 { "" } else { "er" }
    ));
    parts.push(format!("{} episoder", m.episode_count));
    parts.extend(m.maturity_ratings.iter().flatten().take(1).cloned());
    parts.extend(m.tenant_categories.iter().flatten().take(3).cloned());
    parts.join("  ·  ")
}

fn paragraph(painter: &Painter, at: Pos2, text: &str) {
    let format = TextFormat::simple(FontId::proportional(19.0), theme::TEXT);
    let mut job = LayoutJob::single_section(text.to_owned(), format);
    job.wrap = TextWrapping {
        max_width: TEXT_W,
        max_rows: 4,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    painter.galley(at, painter.layout_job(job), theme::TEXT);
}

fn play_label(page: &SeriesPage, app: &App, resolved: Option<&Resolve>) -> String {
    let Some(Resolve::Play {
        season,
        episode,
        kind,
    }) = resolved
    else {
        return "Henter…".into();
    };
    let verb = match kind {
        Kind::Start => "Spill av",
        Kind::Resume => "Fortsett",
        Kind::Next => "Neste episode",
    };
    let Some(s) = page.seasons.get(*season) else {
        return format!("▶  {verb}");
    };
    let ep = app
        .anime
        .episodes
        .get(&s.id)
        .and_then(Load::ready)
        .and_then(|l| l.get(*episode));
    let at = ep.map_or_else(String::new, |e| {
        format!("  ·  S{} E{}", s.season_number, e.label())
    });
    format!("▶  {verb}{at}")
}

fn button(painter: &Painter, b: Button) {
    let (fill, color) = if b.focused {
        (theme::ACCENT, Color32::BLACK)
    } else {
        (theme::PANEL_ACTIVE, theme::TEXT)
    };
    painter.rect_filled(b.rect, theme::RADIUS_SM, fill);
    let at = b.rect.left_center() + vec2(24.0, 0.0);
    painter.text(
        at,
        Align2::LEFT_CENTER,
        b.text,
        FontId::proportional(22.0),
        color,
    );
}

fn seasons(painter: &Painter, at: Pos2, chips: Chips) {
    let mut x = at.x;
    for (i, s) in chips.seasons.iter().enumerate() {
        let text = if s.title.is_empty() {
            format!("Sesong {}", s.season_number)
        } else {
            s.title.clone()
        };
        let galley = painter.layout_no_wrap(text, FontId::proportional(18.0), theme::TEXT);
        let rect = Rect::from_min_size(pos2(x, at.y), vec2(galley.size().x + 36.0, CHIP_H));
        let selected = i == chips.current;
        let lit = selected && chips.active;
        let fill = match (selected, lit) {
            (_, true) => theme::ACCENT,
            (true, false) => theme::PANEL_ACTIVE,
            _ => theme::PANEL,
        };
        painter.rect_filled(rect, CHIP_H / 2.0, fill);
        if selected {
            painter.rect_stroke(rect, CHIP_H / 2.0, Stroke::new(1.5, theme::ACCENT));
        }
        let color = if lit { Color32::BLACK } else { theme::TEXT };
        painter.galley(rect.center() - galley.size() / 2.0, galley, color);
        x = rect.right() + 14.0;
    }
}

fn episodes(app: &App, ui: &Ui, strip: Strip) {
    let Strip { page, list, rect } = strip;
    let f = &app.anime_ui.series;
    let tracked = app.anime.tracker.get(&page.series.id);
    let upto = tracked.and_then(|t| next::position(page, &app.anime.episodes, t));
    let stride = EP_W + EP_GAP;
    let focus = f.episode.min(list.len().saturating_sub(1));
    let id = Id::new(("anime_eps", &page.series.id, f.season));
    let off = rail::offset(
        ui,
        Rail {
            id,
            focus,
            count: list.len(),
            stride,
            view: rect.width(),
        },
    );
    for (i, ep) in list.iter().enumerate() {
        let x = rect.left() + i as f32 * stride - off;
        if x > rect.right() || x + EP_W < rect.left() {
            continue;
        }
        let progress = match (upto, tracked) {
            (Some((s, e)), _) if f.season < s || (f.season == s && i < e) => Some(1.0),
            (Some((s, e)), Some(t)) if f.season == s && i == e => {
                Some(if t.finished() { 1.0 } else { t.progress() })
            }
            _ => None,
        };
        let title = format!("{}. {}", ep.label(), ep.title);
        let subtitle = format!("{} min", ep.duration_secs().div_ceil(60));
        let tex = ep.images.wide(WIDE_PX).and_then(|u| app.images.get(u));
        let slot = Slot {
            rect: Rect::from_min_size(pos2(x, rect.top()), vec2(EP_W, EP_H)),
            focused: f.zone == Zone::Episodes && i == focus,
            id: Id::new(("anime_ep", &ep.id)),
        };
        tile::wide(
            ui,
            slot,
            Art {
                title: &title,
                subtitle: &subtitle,
                progress,
                tex,
            },
        );
    }
}
