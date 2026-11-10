mod app_view;
mod file_rows;
mod preview;

use app_view::AppView;
use gpui_kit::component::Theme;
use gpui_kit::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

/// Opens the main window and runs the app until it quits.
pub fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            Theme::sync_system_appearance(None, cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1100.), px(720.)),
                        cx,
                    ))),
                    // Keeps both the file list and the preview usable.
                    window_min_size: Some(size(px(800.), px(500.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Markdown Converter".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| AppView::new(window, cx)),
            )
            .expect("failed to open window");
            cx.activate(true);
        });
}
