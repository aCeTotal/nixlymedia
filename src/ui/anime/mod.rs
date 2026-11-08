pub mod genre;
pub mod home;
mod rail;
pub mod row;
pub mod series;
mod series_view;
mod tile;

#[derive(Default)]
pub struct AnimeUi {
    pub row: usize,
    pub cols: [usize; row::COUNT],
    pub genre: usize,
    pub series: series::Focus,
}
