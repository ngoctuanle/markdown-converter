//! The file list on the left: one row per file, and the drop target for new files.

use super::app_view::AppView;
use crate::convert::ConvertOutcome;
use crate::file_list::{FileItem, FileList, FileStatus};
use crate::queue::JobId;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Context, ExternalPaths, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, transparent_black,
};

pub(super) fn file_list(
    files: &FileList,
    selected: Option<JobId>,
    cx: &Context<AppView>,
) -> impl IntoElement + use<> {
    let muted = cx.theme().muted_foreground;
    div()
        .id("file-list")
        .size_full()
        .border_2()
        .border_color(transparent_black())
        .drag_over::<ExternalPaths>(|style, _, _, cx| {
            style.border_color(cx.theme().primary).bg(cx.theme().accent)
        })
        .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
            this.add_paths(paths.paths().to_vec(), window, cx)
        }))
        .when(files.items().is_empty(), |list| {
            list.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop files here"),
            )
        })
        .children(
            files
                .items()
                .iter()
                .map(|item| file_row(item, selected == Some(item.id), cx)),
        )
        .overflow_y_scrollbar()
}

fn file_row(item: &FileItem, selected: bool, cx: &Context<AppView>) -> impl IntoElement + use<> {
    let id = item.id;
    let theme = cx.theme();
    let (border, accent, muted) = (theme.border, theme.accent, theme.muted_foreground);
    let name = item
        .source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| item.source.display().to_string());
    let detail = match &item.status {
        FileStatus::Done(outcome) => Some(done_summary(outcome)),
        FileStatus::Failed(err) => Some(err.reason.clone()),
        _ => None,
    };
    let action = match &item.status {
        FileStatus::Waiting | FileStatus::Queued => Some(
            Button::new(("remove", id))
                .ghost()
                .xsmall()
                .label("×")
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.remove(id, window, cx)
                })),
        ),
        FileStatus::Failed(_) => Some(
            Button::new(("retry", id))
                .ghost()
                .xsmall()
                .label("Retry")
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.retry(id, cx)
                })),
        ),
        FileStatus::Converting | FileStatus::Done(_) => None,
    };

    v_flex()
        .id(("row", id))
        .px_3()
        .py_2()
        .gap_0p5()
        .border_b_1()
        .border_color(border)
        .cursor_pointer()
        .when(selected, |row| row.bg(accent))
        .on_click(cx.listener(move |this, _, window, cx| this.select(id, window, cx)))
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(status_label(&item.status, cx))
                .children(action),
        )
        .when_some(detail, |row, text| {
            row.child(div().text_xs().text_color(muted).truncate().child(text))
        })
}

fn status_label(status: &FileStatus, cx: &Context<AppView>) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let color = match status {
        FileStatus::Waiting | FileStatus::Queued => theme.muted_foreground,
        FileStatus::Converting => theme.foreground,
        FileStatus::Done(_) => theme.success,
        FileStatus::Failed(_) => theme.danger,
    };
    h_flex()
        .gap_1()
        .text_color(color)
        .when(matches!(status, FileStatus::Converting), |label| {
            label.child(Spinner::new().xsmall())
        })
        .child(status.label())
}

/// The second line of a Done row: `→ out.md · 3 images`, plus warnings if any.
fn done_summary(outcome: &ConvertOutcome) -> String {
    let md_name = outcome
        .md_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut text = format!("→ {md_name} · {} images", outcome.pictures);
    if !outcome.warnings.is_empty() {
        text.push_str(&format!(" · {} warnings", outcome.warnings.len()));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn done_summary_shows_name_pictures_and_warnings() {
        let mut outcome = ConvertOutcome {
            md_path: PathBuf::from("/out/report-converted.md"),
            assets_dir: None,
            pictures: 3,
            warnings: vec![],
        };
        assert_eq!(done_summary(&outcome), "→ report-converted.md · 3 images");
        outcome.warnings = vec!["a".into(), "b".into()];
        assert_eq!(
            done_summary(&outcome),
            "→ report-converted.md · 3 images · 2 warnings"
        );
    }
}
