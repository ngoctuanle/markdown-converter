mod app_view;
mod file_rows;
mod preview;

use app_view::AppView;
use gpui_kit::component::Theme;
use gpui_kit::{
    App, AppContext, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, actions, px, size,
};

actions!(markdown_converter, [Quit]);

#[cfg(target_os = "macos")]
const QUIT_KEY: &str = "cmd-q";
#[cfg(not(target_os = "macos"))]
const QUIT_KEY: &str = "ctrl-q";

/// Opens the main window and runs the app until it quits.
pub fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            Theme::sync_system_appearance(None, cx);

            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
            cx.bind_keys([KeyBinding::new(QUIT_KEY, Quit, None)]);
            cx.set_menus([Menu::new("Markdown Converter").items([MenuItem::action("Quit", Quit)])]);
            // Closing the window quits: there is nothing to do without it.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

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
