//! Right-hand pane showing the selected file's Markdown, raw or formatted.

use crate::convert::ConvertOutcome;
use crate::file_list::{FileItem, FileStatus};
use crate::preview_text::{
    PREVIEW_LIMIT, PreviewText, is_relative_path, load_preview, percent_decode,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Editor, EditorState, Textarea, TextareaState};
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, ImageSource, IntoElement, ParentElement, Render,
    SharedString, SharedUri, Styled, Task, Window, div, relative,
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
    /// The conversion's warnings, e.g. pictures that weren't found.
    warnings: Vec<String>,
    /// Only the first `PREVIEW_LIMIT` bytes are shown.
    truncated: bool,
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
    /// The file being read in the background, if any.
    loading: Option<PathBuf>,
    /// Dropping it abandons the read.
    _load_task: Option<Task<()>>,
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
            loading: None,
            _load_task: None,
        }
    }

    /// Shows `item`, or the placeholder when it is `None`.
    ///
    /// Called again whenever any conversion reports progress, so a file already on
    /// show (or being read) is not read a second time.
    pub fn show(&mut self, item: Option<&FileItem>, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(item.map(|item| &item.status), Some(FileStatus::Done(_))) {
            self.loading = None;
            self._load_task = None;
        }
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
                let on_show = self.notice.is_none()
                    && self.current.as_ref().is_some_and(|c| &c.md_path == path);
                if on_show || self.loading.as_ref() == Some(path) {
                    return;
                }
                self.start_loading(outcome.clone(), window, cx);
                Some(Notice::Hint("Loading…".into()))
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

    /// Reads the file off the UI thread, then shows it unless the selection
    /// changed in the meantime.
    fn start_loading(
        &mut self,
        outcome: ConvertOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = outcome.md_path.clone();
        self.loading = Some(path.clone());
        let read = cx.background_spawn(async move { load_preview(&path, PREVIEW_LIMIT) });
        self._load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            this.update_in(cx, |this, window, cx| {
                this.finish_loading(outcome, result, window, cx)
            })
            .ok();
        }));
    }

    fn finish_loading(
        &mut self,
        outcome: ConvertOutcome,
        result: std::io::Result<PreviewText>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loading.as_ref() != Some(&outcome.md_path) {
            return; // A stale read: another file was selected since.
        }
        self.loading = None;
        match result {
            Ok(text) => {
                self.current = Some(Loaded {
                    md_path: outcome.md_path,
                    preview_md: text.formatted.into(),
                    warnings: outcome.warnings,
                    truncated: text.truncated,
                });
                self.editor
                    .update(cx, |editor, cx| editor.set_value(text.raw, window, cx));
                self.notice = None;
            }
            Err(e) => {
                self.notice = Some(Notice::Hint(format!(
                    "Could not read {}: {e}",
                    outcome.md_path.display()
                )));
            }
        }
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
        let content = match self.mode {
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
        };
        let muted = cx.theme().muted_foreground;
        let has_notes = current.truncated || !current.warnings.is_empty();
        v_flex()
            .size_full()
            .when(has_notes, |pane| {
                pane.child(
                    v_flex()
                        .px_4()
                        .py_2()
                        .gap_1()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .text_sm()
                        .text_color(muted)
                        .children(
                            current
                                .warnings
                                .iter()
                                .map(|warning| div().child(format!("Warning: {warning}"))),
                        )
                        .when(current.truncated, |notes| notes.child(truncation_note())),
                )
            })
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

/// Shown above a file that is longer than the preview loads.
fn truncation_note() -> String {
    format!(
        "Showing the first {} MB — use Show in folder for the full file",
        PREVIEW_LIMIT / (1024 * 1024)
    )
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
        assert_eq!(
            failure_text("pandoc: bad zip\nline 2"),
            "pandoc: bad zip\nline 2"
        );
        assert_eq!(failure_text(""), "Conversion failed");
        assert_eq!(failure_text("  \n"), "Conversion failed");
    }

    #[test]
    fn truncation_note_names_the_limit() {
        assert_eq!(
            truncation_note(),
            "Showing the first 2 MB — use Show in folder for the full file"
        );
    }
}
