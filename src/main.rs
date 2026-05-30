mod api;
mod app;
mod cache;
mod config;
mod input;
mod iptv;
mod library;
mod log;
mod player;
mod ui;

fn main() -> eframe::Result<()> {
    log::init();
    let runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(4)
            .build()
            .expect("tokio runtime"),
    ));
    let handle = runtime.handle().clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Nixly Media")
            .with_app_id("nixlymedia")
            .with_fullscreen(true)
            .with_maximized(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_active(true),
        vsync: true,
        multisampling: 0,
        ..Default::default()
    };

    eframe::run_native(
        "Nixly Media",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, handle)))),
    )
}
