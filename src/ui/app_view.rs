//! The main window: top bar, file list and preview, and the conversion loop.

use super::file_rows::file_list;
use super::preview::PreviewPane;
use crate::convert::ConvertRequest;
use crate::file_list::{FileList, FileStatus};
use crate::pandoc::Pandoc;
use crate::queue::{Converter, JobId};
use crate::settings::Settings;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, WindowExt, h_flex, h_resizable, resizable_panel, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, ExternalPaths, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, Render, SharedString, Styled, Subscription, Task, Window, div, px,
};
use std::path::{Path, PathBuf};

/// How many skipped file names the warning lists before "…".
const MAX_SKIPPED_NAMES: usize = 5;

pub struct AppView {
    files: FileList,
    converter: Option<Converter>,
    settings: Settings,
    settings_path: Option<PathBuf>,
    selected: Option<JobId>,
    preview: Entity<PreviewPane>,
    _subs: Vec<Subscription>,
    _event_task: Option<Task<()>>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            gpui_kit::component::Theme::sync_system_appearance(Some(window), cx)
        });

        let (converter, event_task) = match Pandoc::locate() {
            Ok(pandoc) => {
                let (converter, rx) = Converter::start(pandoc);
                let task = cx.spawn_in(window, async move |this, cx| {
                    while let Ok(ev) = rx.recv().await {
                        let applied = this.update_in(cx, |this, window, cx| {
                            this.files.apply(ev);
                            this.refresh_preview(window, cx);
                            cx.notify();
                        });
                        if applied.is_err() {
                            break;
                        }
                    }
                });
                (Some(converter), Some(task))
            }
            Err(_) => {
                // The window's notification layer exists only once the window is built.
                window.defer(cx, |window, cx| {
                    window.push_notification(
                        Notification::error("Bundled pandoc not found — reinstall the app")
                            .autohide(false),
                        cx,
                    );
                });
                (None, None)
            }
        };

        let settings_path = Settings::default_path();
        let settings = settings_path
            .as_deref()
            .map(Settings::load_from)
            .unwrap_or_default();

        Self {
            files: FileList::default(),
            converter,
            settings,
            settings_path,
            selected: None,
            preview: cx.new(|cx| PreviewPane::new(window, cx)),
            _subs: vec![appearance],
            _event_task: event_task,
        }
    }

    /// Adds files and folders. During a running batch the new files join it.
    pub(super) fn add_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let running = self.files.is_batch_running();
        let report = self.files.add_paths(&paths);
        if running {
            let jobs = self.files.queue_ids(&report.added);
            self.enqueue(jobs);
        }
        if !report.unsupported.is_empty() {
            window.push_notification(
                Notification::warning(skipped_message(&report.unsupported)),
                cx,
            );
        }
        cx.notify();
    }

    fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = picked.await else { return };
            this.update_in(cx, |this, window, cx| match result {
                Ok(Some(paths)) => this.add_paths(paths, window, cx),
                Ok(None) => {}
                Err(e) => show_dialog_error(e, window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Asks for the fixed output folder. Picking one also turns the fixed location on;
    /// cancelling changes nothing.
    fn pick_output_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = picked.await else { return };
            this.update_in(cx, |this, window, cx| match result {
                Ok(Some(mut dirs)) if !dirs.is_empty() => {
                    this.settings.fixed_output_dir = Some(dirs.swap_remove(0));
                    this.settings.fixed_output = true;
                    this.save_settings(window, cx);
                    cx.notify();
                }
                Ok(_) => {}
                Err(e) => show_dialog_error(e, window, cx),
            })
            .ok();
        })
        .detach();
    }

    fn set_fixed_output(&mut self, checked: bool, window: &mut Window, cx: &mut Context<Self>) {
        if checked && self.settings.fixed_output_dir.is_none() {
            self.pick_output_dir(window, cx);
            return;
        }
        // Unchecking keeps the folder for next time.
        self.settings.fixed_output = checked;
        self.save_settings(window, cx);
        cx.notify();
    }

    fn save_settings(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = &self.settings_path else {
            return;
        };
        if let Err(e) = self.settings.save_to(path) {
            window.push_notification(
                Notification::error(format!("Couldn't save settings: {e}")),
                cx,
            );
        }
    }

    fn convert(&mut self, cx: &mut Context<Self>) {
        if self.converter.is_none() {
            return;
        }
        let jobs = self.files.take_for_convert();
        self.enqueue(jobs);
        cx.notify();
    }

    pub(super) fn retry(&mut self, id: JobId, cx: &mut Context<Self>) {
        if self.converter.is_none() {
            return;
        }
        if let Some(job) = self.files.retry(id) {
            self.enqueue(vec![job]);
        }
        cx.notify();
    }

    /// Removes a Waiting row, or a Queued one whose job hasn't started yet.
    pub(super) fn remove(&mut self, id: JobId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.files.get(id) else {
            return;
        };
        let removable = match item.status {
            FileStatus::Waiting => true,
            // A job that already started must keep its row so its result has somewhere to go.
            FileStatus::Queued => self.converter.as_ref().is_some_and(|c| c.cancel(id)),
            _ => false,
        };
        if removable && self.files.remove(id) {
            self.forget_selection_if_gone(window, cx);
        }
        cx.notify();
    }

    fn clear_finished(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.files.clear_finished();
        self.forget_selection_if_gone(window, cx);
        cx.notify();
    }

    pub(super) fn select(&mut self, id: JobId, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.refresh_preview(window, cx);
        cx.notify();
    }

    fn forget_selection_if_gone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_some_and(|id| self.files.get(id).is_none()) {
            self.selected = None;
            self.refresh_preview(window, cx);
        }
    }

    /// Re-sends the selected item to the preview, e.g. after its conversion finished.
    fn refresh_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let item = self.selected.and_then(|id| self.files.get(id));
        self.preview
            .update(cx, |preview, cx| preview.show(item, window, cx));
    }

    fn enqueue(&self, jobs: Vec<(JobId, PathBuf)>) {
        let Some(converter) = &self.converter else {
            return;
        };
        for (id, source) in jobs {
            let output_dir = self.settings.output_dir_for(&source);
            converter.enqueue(id, ConvertRequest { source, output_dir });
        }
    }

    fn render_top_bar(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let convertible = self.files.convertible_count();
        let fixed_dir = self.settings.fixed_output_dir.as_deref();
        let folder_label: SharedString = fixed_dir
            .map(last_component)
            .unwrap_or_else(|| "Choose folder…".into())
            .into();

        h_flex()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("add")
                    .icon(IconName::Plus)
                    .label("Add files")
                    .on_click(cx.listener(|this, _, window, cx| this.pick_files(window, cx))),
            )
            .child(
                Button::new("clear")
                    .label("Clear")
                    .on_click(cx.listener(|this, _, window, cx| this.clear_finished(window, cx))),
            )
            .child(div().flex_1())
            .child(
                Checkbox::new("fixed-output")
                    .label("Using fixed output location")
                    .checked(self.settings.fixed_output)
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        this.set_fixed_output(*checked, window, cx)
                    })),
            )
            .child(
                Button::new("output-dir")
                    .icon(IconName::FolderOpen)
                    .label(folder_label)
                    .when_some(fixed_dir, |button, dir| {
                        button.tooltip(dir.display().to_string())
                    })
                    .disabled(!self.settings.fixed_output)
                    .on_click(cx.listener(|this, _, window, cx| this.pick_output_dir(window, cx))),
            )
            .child(
                Button::new("convert")
                    .primary()
                    .label(format!("Convert {convertible}"))
                    .disabled(convertible == 0 || self.converter.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.convert(cx))),
            )
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = file_list(&self.files, self.selected, cx);
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            // Dropping anywhere in the window adds the files, not only on the list.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.add_paths(paths.paths().to_vec(), window, cx)
            }))
            .child(self.render_top_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("main")
                        .child(resizable_panel().size(px(380.)).child(list))
                        .child(resizable_panel().child(self.preview.clone())),
                ),
            )
    }
}

fn show_dialog_error(e: anyhow::Error, window: &mut Window, cx: &mut Context<AppView>) {
    window.push_notification(
        Notification::error(format!("Couldn't open the file dialog: {e}")),
        cx,
    );
}

/// The last path component, or the whole path for a root like `/`.
fn last_component(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

fn skipped_message(unsupported: &[PathBuf]) -> String {
    let mut names: Vec<String> = unsupported
        .iter()
        .take(MAX_SKIPPED_NAMES)
        .map(|p| last_component(p))
        .collect();
    if unsupported.len() > MAX_SKIPPED_NAMES {
        names.push("…".into());
    }
    format!(
        "Skipped {} unsupported file(s): {}",
        unsupported.len(),
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_message_lists_up_to_five_names() {
        let one = [PathBuf::from("/fx/local.png")];
        assert_eq!(
            skipped_message(&one),
            "Skipped 1 unsupported file(s): local.png"
        );

        let seven: Vec<PathBuf> = (1..=7)
            .map(|i| PathBuf::from(format!("/fx/{i}.txt")))
            .collect();
        assert_eq!(
            skipped_message(&seven),
            "Skipped 7 unsupported file(s): 1.txt, 2.txt, 3.txt, 4.txt, 5.txt, …"
        );
    }

    #[test]
    fn last_component_of_paths() {
        assert_eq!(last_component(Path::new("/Users/me/Out")), "Out");
        assert_eq!(last_component(Path::new("/")), "/");
    }
}
