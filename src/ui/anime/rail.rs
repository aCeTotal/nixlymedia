use egui::{Id, Ui};

const SCROLL_ANIM: f32 = 0.32;

/* A scrolling strip of equal slots. */
pub struct Rail {
    pub id: Id,
    pub focus: usize,
    pub count: usize,
    pub stride: f32,
    pub view: f32,
}

/* Animated scroll keeping focus visible. */
pub fn offset(ui: &Ui, rail: Rail) -> f32 {
    let max = (rail.count as f32 * rail.stride - rail.view).max(0.0);
    let target = (rail.focus as f32 * rail.stride - rail.stride).clamp(0.0, max);
    ui.ctx()
        .animate_value_with_time(rail.id, target, SCROLL_ANIM)
}

/* Direction from a back/forward press. */
pub fn delta(back: bool, forward: bool) -> isize {
    forward as isize - back as isize
}

/* One focus step within bounds. */
pub fn step(focus: usize, count: usize, delta: isize) -> usize {
    if count == 0 {
        return 0;
    }
    focus.saturating_add_signed(delta).min(count - 1)
}
