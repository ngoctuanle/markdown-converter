//! Right-hand pane showing the selected file's Markdown, raw or formatted.

use crate::file_list::{FileItem, FileStatus};
use crate::preview_text::{is_relative_path, percent_decode, placeholder_unrenderable_images};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Editor, EditorState, Textarea, TextareaState};
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, ImageSource, IntoElement, ParentElement, Render,
    SharedString, SharedUri, Styled, Window, div, relative,
};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewMode {
    #[default]
    Markdown,
    Preview,
}

/// The Markdown file on show.
struct Loaded {
    md_path: PathBuf,
    /// The file's text with undrawable pictures swapped for placeholders.
    preview_md: SharedString,
}

/// What the pane shows instead of a file.
#[derive(Debug, PartialEq, Eq)]
enum Notice {
    /// A short centred hint, e.g. "Not converted yet".
    Hint(String),
    /// A failed conversion's full error text, scrollable and selectable.
    Failure(String),
}

pub struct PreviewPane {
    mode: PreviewMode,
    /// Holds the raw text for Markdown mode.
    editor: Entity<EditorState>,
    /// Holds the error text of a failed file.
    failure_text: Entity<TextareaState>,
    current: Option<Loaded>,
    notice: Option<Notice>,
}

impl PreviewPane {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(true)
        });
        Self {
            mode: PreviewMode::default(),
            editor,
            failure_text: cx.new(|cx| TextareaState::new(window, cx)),
            current: None,
            notice: Some(Notice::Hint("Select a file to preview".into())),
        }
    }

    /// Shows `item`, or the placeholder when it is `None`.
    ///
    /// Called again whenever any conversion reports progress, so a file already on
    /// show is not read a second time.
    pub fn show(&mut self, item: Option<&FileItem>, window: &mut Window, cx: &mut Context<Self>) {
        let notice = match item.map(|item| &item.status) {
            None => Some(Notice::Hint("Select a file to preview".into())),
            Some(FileStatus::Waiting | FileStatus::Queued | FileStatus::Converting) => {
                Some(Notice::Hint("Not converted yet".into()))
            }
            Some(FileStatus::Failed(err)) => {
                Some(Notice::Failure(failure_text(&err.detail).to_string()))
            }
            Some(FileStatus::Done(outcome)) => {
                let path = &outcome.md_path;
                if self.notice.is_none()
                    && self.current.as_ref().is_some_and(|c| &c.md_path == path)
                {
                    return;
                }
                match std::fs::read_to_string(path) {
                    Ok(text) => {
                        self.current = Some(Loaded {
                            md_path: path.clone(),
                            preview_md: placeholder_unrenderable_images(&text).into(),
                        });
                        self.editor
                            .update(cx, |editor, cx| editor.set_value(text, window, cx));
                        None
                    }
                    Err(e) => Some(Notice::Hint(format!(
                        "Could not read {}: {e}",
                        path.display()
                    ))),
                }
            }
        };
        if let Some(notice) = &notice {
            if self.current.is_none() && self.notice.as_ref() == Some(notice) {
                return;
            }
            self.current = None;
            if let Notice::Failure(text) = notice {
                let text = text.clone();
                self.failure_text
                    .update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
        self.notice = notice;
        cx.notify();
    }

    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let selected = match self.mode {
            PreviewMode::Markdown => 0,
            PreviewMode::Preview => 1,
        };
        h_flex()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                TabBar::new("mode")
                    .segmented()
                    .selected_index(selected)
                    .child("Markdown")
                    .child("Preview")
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.mode = if *index == 0 {
                            PreviewMode::Markdown
                        } else {
                            PreviewMode::Preview
                        };
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("show-in-folder")
                    .icon(IconName::FolderOpen)
                    .label("Show in folder")
                    .disabled(self.current.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(current) = &this.current {
                            cx.reveal_path(&current.md_path);
                        }
                    })),
            )
    }

    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        match &self.notice {
            Some(Notice::Hint(hint)) => {
                return div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .text_color(cx.theme().muted_foreground)
                    .child(hint.clone())
                    .into_any_element();
            }
            Some(Notice::Failure(_)) => {
                return div()
                    .size_full()
                    .p_2()
                    .child(
                        Textarea::new(&self.failure_text)
                            .readonly(true)
                            .appearance(false)
                            .h(relative(1.)),
                    )
                    .into_any_element();
            }
            None => {}
        }
        let Some(current) = &self.current else {
            return div().into_any_element();
        };
        match self.mode {
            PreviewMode::Markdown => Editor::new(&self.editor)
                .readonly(true)
                .h(relative(1.))
                .into_any_element(),
            PreviewMode::Preview => {
                let base_dir = current
                    .md_path
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_default();
                TextView::markdown("md-preview", current.preview_md.clone())
                    .selectable(true)
                    .scrollable(true)
                    .image_source(move |uri: &SharedUri| -> ImageSource {
                        if is_relative_path(uri) {
                            ImageSource::from(base_dir.join(percent_decode(uri)))
                        } else {
                            uri.clone().into()
                        }
                    })
                    .p_4()
                    .size_full()
                    .into_any_element()
            }
        }
    }
}

/// The error text to show for a failed file; never blank.
fn failure_text(detail: &str) -> &str {
    if detail.trim().is_empty() {
        "Conversion failed"
    } else {
        detail
    }
}

impl Render for PreviewPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.render_header(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_text_falls_back_when_blank() {
        assert_eq!(failure_text("pandoc: bad zip\nline 2"), "pandoc: bad zip\nline 2");
        assert_eq!(failure_text(""), "Conversion failed");
        assert_eq!(failure_text("  \n"), "Conversion failed");
    }
}
