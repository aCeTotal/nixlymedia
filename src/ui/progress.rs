use egui::{pos2, vec2, Color32, FontId, Rect, Stroke, Ui};

use crate::player::bandwidth::BufferPolicy;
use crate::player::view::{Phase, PlayerView};
use crate::ui::theme;

const PANEL_W: f32 = 640.0;
const PANEL_H: f32 = 360.0;

pub fn draw(ui: &mut Ui, player: &PlayerView) {
    ui.ctx().request_repaint();
    let rect = ui.max_rect();
    let panel = Rect::from_center_size(rect.center(), vec2(PANEL_W, PANEL_H));
    ui.painter().rect_filled(
        panel,
        theme::RADIUS,
        Color32::from_rgba_premultiplied(15, 18, 28, 245),
    );
    ui.painter().rect_stroke(
        panel,
        theme::RADIUS,
        Stroke::new(1.5, theme::ACCENT),
    );

    let phase = player.phase();
    let (title, subtitle) = match phase {
        Phase::Probing => ("Tester båndbredde", "Måler hastighet mot mediaserveren"),
        Phase::Preloading => ("Forhåndslaster", "Bygger buffer for jevn avspilling"),
        _ => ("Klargjør", ""),
    };
    ui.painter().text(
        pos2(panel.left() + 28.0, panel.top() + 24.0),
        egui::Align2::LEFT_TOP,
        title,
        FontId::proportional(24.0),
        theme::TEXT,
    );
    ui.painter().text(
        pos2(panel.left() + 28.0, panel.top() + 56.0),
        egui::Align2::LEFT_TOP,
        subtitle,
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );

    let snap = player.progress_snapshot();
    let bitrate_mbps = player
        .probe_result
        .as_ref()
        .map(|r| r.bitrate_mbps)
        .unwrap_or((player.bitrate as f64) / 1_000_000.0);

    let (done, total, mbps, eta, big_label, color, decision) = match phase {
        Phase::Probing => {
            let mbps = snap.mbps_running;
            let elapsed = player.started_at.elapsed().as_secs_f64();
            let label = format!("{:.1} Mbps", mbps);
            let color = mbps_color(mbps, bitrate_mbps);
            let decision = if mbps <= 0.01 {
                "Starter måling…".to_string()
            } else {
                live_decision(mbps, bitrate_mbps)
            };
            (
                snap.bytes_done,
                snap.bytes_total.max(1),
                mbps,
                Some(elapsed),
                label,
                color,
                decision,
            )
        }
        Phase::Preloading => {
            let res = player.probe_result.as_ref();
            /* Mål i innholds-sekunder, mot resten av filen ved resume nær
             * slutt (speiler readiness-logikken i PlayerView::poll). */
            let target = res.map(|r| r.preload_seconds).unwrap_or(15).max(1) as f64;
            let rest = if player.duration > 0 {
                (player.duration as f64 - player.resume_pos - 2.0).max(1.0)
            } else {
                f64::INFINITY
            };
            let goal = target.min(rest);
            let buffered = player
                .mpv
                .as_ref()
                .and_then(|m| m.demuxer_cache_duration())
                .unwrap_or(0.0)
                .min(goal);
            /* Live nedlastingshastighet fra mpv-cachen — mer presis enn
             * probe-snapshotet, og viser at nedlastingen faktisk går. */
            let live_bps = player
                .mpv
                .as_ref()
                .and_then(|m| m.cache_speed())
                .unwrap_or(0.0);
            let mbps = if live_bps > 1.0 {
                live_bps * 8.0 / 1_000_000.0
            } else {
                res.map(|r| r.mbps).unwrap_or(0.0)
            };
            /* ETA: gjenstående innholds-sek × bitrate / hastighet. */
            let eta = if mbps > 0.05 {
                (goal - buffered).max(0.0) * bitrate_mbps / mbps
            } else {
                f64::INFINITY
            };
            let pct = ((buffered / goal) * 100.0).clamp(0.0, 100.0);
            let label = format!("{pct:.0}%");
            let color = match res.map(|r| r.policy) {
                Some(BufferPolicy::InstantStart) => theme::GOOD,
                Some(BufferPolicy::PreloadShort) => theme::GOLD,
                _ => theme::BAD,
            };
            let decision = res
                .map(|r| policy_text(r.policy, r.preload_seconds))
                .unwrap_or_default();
            (
                (buffered * 10.0) as u64,
                (goal * 10.0).max(1.0) as u64,
                mbps,
                Some(eta),
                label,
                color,
                decision,
            )
        }
        _ => (0, 1, 0.0, None, String::new(), theme::TEXT_DIM, String::new()),
    };

    let bar_top = panel.top() + 110.0;
    let bar = Rect::from_min_max(
        pos2(panel.left() + 28.0, bar_top),
        pos2(panel.right() - 28.0, bar_top + 26.0),
    );
    ui.painter().rect_filled(bar, 8.0, theme::PANEL);
    ui.painter()
        .rect_stroke(bar, 8.0, Stroke::new(1.0, theme::PANEL_HOVER));

    let frac = (done as f32) / (total.max(1) as f32);
    let frac = frac.clamp(0.0, 1.0);
    let fill = Rect::from_min_max(
        bar.left_top(),
        pos2(bar.left() + bar.width() * frac, bar.bottom()),
    );
    ui.painter().rect_filled(fill, 8.0, color);

    let t = (ui.ctx().input(|i| i.time) * 1.5).sin() as f32 * 0.5 + 0.5;
    let shine_x = bar.left() + bar.width() * frac;
    if frac < 1.0 {
        let shine = Rect::from_min_max(
            pos2((shine_x - 40.0).max(bar.left()), bar.top()),
            pos2(shine_x, bar.bottom()),
        );
        let a = (60.0 * t) as u8;
        ui.painter().rect_filled(
            shine,
            8.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, a),
        );
    }

    let pct = (frac * 100.0).round();
    ui.painter().text(
        pos2(bar.right(), bar.top() - 8.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{pct:.0}%"),
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );

    ui.painter().text(
        pos2(bar.left(), bar.top() - 8.0),
        egui::Align2::LEFT_BOTTOM,
        match phase {
            Phase::Probing => format!(
                "{:.1} / {:.1} MB",
                done as f64 / 1_000_000.0,
                total as f64 / 1_000_000.0
            ),
            Phase::Preloading => format!(
                "{:.0} / {:.0} s buffret",
                done as f64 / 10.0,
                total as f64 / 10.0
            ),
            _ => String::new(),
        },
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );

    ui.painter().text(
        pos2(panel.left() + 28.0, panel.top() + 170.0),
        egui::Align2::LEFT_TOP,
        big_label,
        FontId::proportional(42.0),
        color,
    );

    if let Some(e) = eta {
        let txt = match phase {
            Phase::Probing => format!("{:.1} s brukt", e),
            Phase::Preloading => {
                if !e.is_finite() {
                    "Beregner…".into()
                } else if e < 1.0 {
                    "Starter…".into()
                } else {
                    format!("Starter om ~{}", fmt_eta(e))
                }
            }
            _ => String::new(),
        };
        ui.painter().text(
            pos2(panel.right() - 28.0, panel.top() + 178.0),
            egui::Align2::RIGHT_TOP,
            txt,
            FontId::proportional(15.0),
            theme::TEXT,
        );
    }

    let info_top = panel.top() + 230.0;
    info_row(
        ui,
        panel,
        info_top,
        "Filbitrate",
        &format!("{:.1} Mbps", bitrate_mbps),
        theme::TEXT,
    );
    info_row(
        ui,
        panel,
        info_top + 24.0,
        /* Under preload er tallet live cache-speed, ikke probe-målingen. */
        match phase {
            Phase::Preloading => "Nedlasting",
            _ => "Målt linje",
        },
        &format!("{:.1} Mbps", mbps),
        color,
    );
    if let Some(res) = &player.probe_result {
        if let Some(sz) = res.file_size {
            info_row(
                ui,
                panel,
                info_top + 48.0,
                "Filstørrelse",
                &format!("{:.1} GB", sz as f64 / 1_000_000_000.0),
                theme::TEXT,
            );
        }
        info_row(
            ui,
            panel,
            info_top + 72.0,
            "Strategi",
            &policy_label(res.policy),
            color,
        );
    }

    ui.painter().text(
        pos2(panel.center().x, panel.bottom() - 22.0),
        egui::Align2::CENTER_CENTER,
        decision,
        FontId::proportional(14.0),
        color,
    );
}

fn info_row(ui: &Ui, panel: Rect, y: f32, label: &str, value: &str, value_color: Color32) {
    ui.painter().text(
        pos2(panel.left() + 28.0, y),
        egui::Align2::LEFT_TOP,
        label,
        FontId::proportional(13.0),
        theme::TEXT_DIM,
    );
    ui.painter().text(
        pos2(panel.right() - 28.0, y),
        egui::Align2::RIGHT_TOP,
        value,
        FontId::proportional(14.0),
        value_color,
    );
}

fn mbps_color(mbps: f64, bitrate: f64) -> Color32 {
    let safe = if bitrate > 0.1 { bitrate } else { 8.0 };
    let r = mbps / safe;
    if r >= 1.5 {
        theme::GOOD
    } else if r >= 1.0 {
        theme::GOLD
    } else if mbps <= 0.01 {
        theme::TEXT_DIM
    } else {
        theme::BAD
    }
}

fn live_decision(mbps: f64, bitrate: f64) -> String {
    let safe = if bitrate > 0.1 { bitrate } else { 8.0 };
    let r = mbps / safe;
    /* Speiler INSTANT_RATIO (1.3) i bandwidth::compute_policy. */
    if r >= 1.3 {
        format!("Bra linje · {:.1}× bitrate — starter direkte", r)
    } else if r >= 1.0 {
        format!("Marginal linje · {:.1}× bitrate — kort buffer først", r)
    } else {
        format!("Smal linje · {:.1}× bitrate — laster ned buffer først", r)
    }
}

fn policy_text(p: BufferPolicy, preload: u32) -> String {
    match p {
        BufferPolicy::InstantStart => "Direktestart — linje god nok".into(),
        BufferPolicy::PreloadShort | BufferPolicy::PreloadLong => format!(
            "Laster ned {preload} s buffer — resten lastes videre under avspilling"
        ),
    }
}

fn fmt_eta(secs: f64) -> String {
    let s = secs.round().max(0.0) as u64;
    if s >= 60 {
        format!("{}:{:02} min", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

fn policy_label(p: BufferPolicy) -> String {
    match p {
        BufferPolicy::InstantStart => "Direktestart".into(),
        BufferPolicy::PreloadShort => "Kort buffer".into(),
        BufferPolicy::PreloadLong => "Lang buffer".into(),
    }
}
