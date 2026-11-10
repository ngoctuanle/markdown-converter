//! Right-hand pane showing the selected file's output. A stub until Task 11.

use crate::file_list::FileItem;
use gpui_kit::{Context, IntoElement, Render, Styled, Window, div};

pub struct PreviewPane;

impl PreviewPane {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        PreviewPane
    }

    /// Shows `item`, or the placeholder when it is `None`.
    pub fn show(
        &mut self,
        _item: Option<&FileItem>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }
}

impl Render for PreviewPane {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}
