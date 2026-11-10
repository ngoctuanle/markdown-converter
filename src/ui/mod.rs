use gpui_kit::component::Theme;
use gpui_kit::{
    App, AppContext, Bounds, Context, IntoElement, Render, Styled, Subscription, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, px, size,
};

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

/// Placeholder root view; later tasks fill it in.
pub struct AppView {
    _appearance: Subscription,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system_appearance(Some(window), cx)
        });
        Self { _appearance }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}
