use egui::{
    style::{ScrollAnimation, Selection, WidgetVisuals, Widgets},
    Color32, Context, FontFamily, FontId, Rangef, Rounding, Stroke, Style, TextStyle, Vec2,
    Visuals,
};

pub const BG: Color32 = Color32::from_rgb(12, 14, 22);
pub const PANEL: Color32 = Color32::from_rgb(22, 26, 38);
pub const PANEL_HOVER: Color32 = Color32::from_rgb(34, 40, 56);
pub const PANEL_ACTIVE: Color32 = Color32::from_rgb(46, 54, 74);
pub const ACCENT: Color32 = Color32::from_rgb(0, 180, 255);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0, 140, 220);
pub const TEXT: Color32 = Color32::from_rgb(232, 236, 244);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 158, 175);
pub const GOOD: Color32 = Color32::from_rgb(80, 220, 140);
pub const BAD: Color32 = Color32::from_rgb(240, 90, 90);
pub const GOLD: Color32 = Color32::from_rgb(255, 195, 70);

pub const RADIUS: f32 = 14.0;
pub const RADIUS_SM: f32 = 8.0;

pub fn install(ctx: &Context) {
    let mut style = Style::default();
    let mut v = Visuals::dark();

    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = PANEL;
    v.selection = Selection {
        bg_fill: ACCENT_SOFT,
        stroke: Stroke::new(1.0, ACCENT),
    };
    v.hyperlink_color = ACCENT;

    let r = Rounding::same(RADIUS_SM);
    v.widgets = Widgets {
        noninteractive: WidgetVisuals {
            bg_fill: PANEL,
            weak_bg_fill: PANEL,
            bg_stroke: Stroke::new(0.0, Color32::TRANSPARENT),
            fg_stroke: Stroke::new(1.0, TEXT),
            rounding: r,
            expansion: 0.0,
        },
        inactive: WidgetVisuals {
            bg_fill: PANEL,
            weak_bg_fill: PANEL,
            bg_stroke: Stroke::new(0.0, Color32::TRANSPARENT),
            fg_stroke: Stroke::new(1.0, TEXT),
            rounding: r,
            expansion: 0.0,
        },
        hovered: WidgetVisuals {
            bg_fill: PANEL_HOVER,
            weak_bg_fill: PANEL_HOVER,
            bg_stroke: Stroke::new(1.0, ACCENT),
            fg_stroke: Stroke::new(1.2, TEXT),
            rounding: r,
            expansion: 1.0,
        },
        active: WidgetVisuals {
            bg_fill: PANEL_ACTIVE,
            weak_bg_fill: PANEL_ACTIVE,
            bg_stroke: Stroke::new(1.5, ACCENT),
            fg_stroke: Stroke::new(1.4, TEXT),
            rounding: r,
            expansion: 1.0,
        },
        open: WidgetVisuals {
            bg_fill: PANEL_ACTIVE,
            weak_bg_fill: PANEL_ACTIVE,
            bg_stroke: Stroke::new(1.0, ACCENT),
            fg_stroke: Stroke::new(1.2, TEXT),
            rounding: r,
            expansion: 0.0,
        },
    };

    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(14.0, 10.0);
    style.spacing.window_margin = egui::Margin::same(16.0);
    style.spacing.scroll.bar_width = 0.0;
    style.spacing.scroll.bar_inner_margin = 0.0;
    style.spacing.scroll.bar_outer_margin = 0.0;
    style.spacing.scroll.floating = true;
    style.spacing.scroll.foreground_color = false;
    style.scroll_animation = ScrollAnimation {
        points_per_second: 4500.0,
        duration: Rangef::new(0.16, 0.32),
    };

    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(32.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Body,
        FontId::new(15.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(15.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );

    ctx.set_style(style);
}

pub fn rating_color(rating: f32) -> Color32 {
    if rating >= 7.5 {
        GOOD
    } else if rating >= 6.0 {
        GOLD
    } else if rating > 0.0 {
        BAD
    } else {
        TEXT_DIM
    }
}
