//! Remote files dialog: browse a host folder (`ls`) and open, edit and save a text file
//! (`cat`, `cat > file`), all over ssh. Every call runs off the UI thread.

use agentdeck_core::hosts::{Host, RemoteEntry, join_path, parent_path};
use agentdeck_core::logic::theme::FONT_MONO;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{hsla, hsla_a, palette, ui};

/// Opens the dialog on `host`, in its default folder.
pub fn open_files(host: Host, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| RemoteFiles::new(host, window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        let viewport = window.viewport_size();
        dialog
            .w(px((f32::from(viewport.width) * 0.94).min(1100.0)))
            .h(px((f32::from(viewport.height) - 48.0).min(800.0)))
            .p_0()
            .overflow_hidden()
            .centered()
            .close_button(false)
            .child(view.clone())
    });
}

struct RemoteFiles {
    host: Host,
    /// Absolute folder shown (resolved by the host); the default folder until the first listing.
    dir: String,
    entries: Vec<RemoteEntry>,
    /// File in the editor.
    file: Option<String>,
    editor: Entity<TextareaState>,
    busy: bool,
    /// Last error, or "Saved".
    message: Option<(String, bool)>,
}

impl RemoteFiles {
    fn new(host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| TextareaState::new(window, cx));
        let dir = host.folder.clone();
        let mut this = Self { host, dir: dir.clone(), entries: Vec::new(), file: None, editor, busy: false, message: None };
        this.list(dir, window, cx);
        this
    }

    /// Runs a blocking ssh call off the UI thread, then `done` on the dialog.
    fn run<T: Send + 'static>(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        work: impl FnOnce(Host) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.message = None;
        let host = self.host.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { work(host) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(value) => done(this, value, window, cx),
                    Err(e) => this.message = Some((e, true)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn list(&mut self, dir: String, window: &Window, cx: &mut Context<Self>) {
        self.run(window, cx, move |host| host.list_dir(&dir), |this, (dir, entries), _, _| {
            this.dir = dir;
            this.entries = entries;
        });
    }

    fn open(&mut self, path: String, window: &Window, cx: &mut Context<Self>) {
        let read = path.clone();
        self.run(window, cx, move |host| host.read_file(&read), move |this, text, window, cx| {
            this.editor.update(cx, |e, cx| e.set_value(text, window, cx));
            this.file = Some(path);
        });
    }

    fn save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(path) = self.file.clone() else { return };
        let text = self.editor.read(cx).value().to_string();
        self.run(window, cx, move |host| host.write_file(&path, &text), |this, (), _, _| {
            this.message = Some(("Saved".into(), false));
        });
    }

    fn row(&self, i: usize, name: String, dir: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        let path = if name == ".." { parent_path(&self.dir) } else { join_path(&self.dir, &name) };
        let current = self.file.as_deref() == Some(path.as_str());
        div()
            .id(("remote-entry", i))
            .flex()
            .items_center()
            .gap_2()
            .h(ui(26.0))
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .text_color(hsla(if current { p.fg } else { p.muted }))
            .when(current, |d| d.bg(hsla(p.raised)))
            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if dir {
                    this.list(path.clone(), window, cx);
                } else {
                    this.open(path.clone(), window, cx);
                }
            }))
            .child(Icon::new(if dir { IconName::Folder } else { IconName::File }).size(ui(14.0)))
            .child(div().min_w_0().truncate().child(name))
    }
}

impl Render for RemoteFiles {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mut rows = vec![self.row(0, "..".into(), true, cx)];
        let entries = self.entries.clone();
        rows.extend(entries.into_iter().enumerate().map(|(i, e)| self.row(i + 1, e.name, e.dir, cx)));
        let title = format!("{} · {}", self.host.name, if self.dir.is_empty() { "~" } else { self.dir.as_str() });
        let can_save = self.file.is_some() && !self.busy;
        let header = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .h_12()
            .pl_5()
            .pr_3()
            .border_b_1()
            .border_color(hsla(p.line))
            .child(Icon::new(IconName::Server).size(ui(15.0)).text_color(hsla(p.muted)))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(title))
            .when(self.busy, |d| d.child(div().text_xs().text_color(hsla(p.subtle)).child("Working…")))
            .when_some(self.message.clone(), |d, (text, bad)| {
                d.child(div().text_xs().text_color(hsla(if bad { p.bad } else { p.ok })).child(text))
            })
            .child(
                div()
                    .id("remote-save")
                    .flex()
                    .items_center()
                    .gap(ui(6.0))
                    .h_8()
                    .px_3()
                    .rounded_md()
                    .border_1()
                    .border_color(hsla(p.line))
                    .text_color(hsla(p.muted))
                    .when(!can_save, |d| d.opacity(0.4))
                    .when(can_save, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
                    })
                    .child(Icon::new(IconName::Save).size(ui(14.0)))
                    .child("Save"),
            )
            .child(crate::panes::close_button("remote-close", "Close").on_click(|window, cx| window.close_dialog(cx)));
        let editor = match &self.file {
            Some(path) => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_2()
                .p_3()
                .child(div().text_xs().text_color(hsla(p.subtle)).truncate().child(path.clone()))
                .child(div().flex_1().min_h_0().font_family(FONT_MONO).text_xs().child(Textarea::new(&self.editor).h(relative(1.0)))),
            None => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(hsla(p.subtle))
                .child("Pick a file to open it."),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.panel))
            .text_color(hsla(p.fg))
            .text_size(ui(13.0))
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("remote-list")
                            .flex()
                            .flex_col()
                            .w(ui(280.0))
                            .flex_shrink_0()
                            .p_2()
                            .border_r_1()
                            .border_color(hsla(p.line))
                            .bg(hsla_a(p.base, 0.5))
                            .overflow_y_scroll()
                            .children(rows),
                    )
                    .child(editor),
            )
    }
}
