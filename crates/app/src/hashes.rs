//! The Hashes page (app.md, custom pages): every algorithm at once, of text
//! or a file, with a hash to compare against and the `tw` command that gives
//! the line. One page for the five `*sum` groups, and BLAKE3.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, ExternalPaths, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement, PathPromptOptions, Render, SharedString,
    Styled, Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use toolwing_core::Ctx;
use toolwing_tools::command::quote;
use toolwing_tools::hash::{self, Algorithm};

/// Typing pauses this long before the digests are computed (app.md).
const DEBOUNCE: Duration = Duration::from_millis(60);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Text,
    File,
}

pub struct HashesPage {
    source: Source,
    input: Entity<TextareaState>,
    /// Hash the text with a newline at the end, as `echo` gives it.
    add_newline: bool,
    file: Option<PathBuf>,
    compare: Entity<InputState>,
    base64: bool,
    upper: bool,
    digests: Vec<(Algorithm, Vec<u8>)>,
    /// Bytes hashed, for the line under the input.
    hashed: u64,
    error: Option<String>,
    stale: bool,
    generation: u64,
    ctx: Ctx,
    _subscriptions: Vec<Subscription>,
}

impl HashesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Type or paste the text"));
        let compare =
            cx.new(|cx| InputState::new(window, cx).placeholder("Paste a hash to compare with"));
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    this.schedule(window, cx);
                }
            }),
            cx.subscribe_in(&compare, window, |_, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
        ];
        let mut page = HashesPage {
            source: Source::Text,
            input,
            add_newline: false,
            file: None,
            compare,
            base64: false,
            upper: false,
            digests: Vec::new(),
            hashed: 0,
            error: None,
            stale: false,
            generation: 0,
            ctx: Ctx::new(),
            _subscriptions: subscriptions,
        };
        page.schedule(window, cx);
        page
    }

    /// Replaces the text, as typing or pasting it would.
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.source = Source::Text;
        self.input
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.schedule(window, cx);
    }

    /// Hashes a file, as dropping or choosing it would.
    pub fn set_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.source = Source::File;
        self.file = Some(path);
        self.schedule(window, cx);
    }

    pub fn set_compare(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.compare
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        cx.notify();
    }

    pub fn settled(&self) -> bool {
        !self.stale
    }

    /// Each algorithm's digest as the page shows it.
    pub fn shown(&self) -> Vec<(String, String)> {
        self.digests
            .iter()
            .map(|(a, d)| (a.title(), self.format(d)))
            .collect()
    }

    pub fn matched(&self, cx: &gpui::App) -> Option<Algorithm> {
        hash::find_match(&self.compare.read(cx).value(), &self.digests)
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn format(&self, digest: &[u8]) -> String {
        if self.base64 {
            hash::base64(digest)
        } else if self.upper {
            hash::hex(digest).to_uppercase()
        } else {
            hash::hex(digest)
        }
    }

    /// Computes every digest after a pause, in the background, in one pass;
    /// a newer run cancels this one.
    fn schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.ctx.cancel();
        self.ctx = Ctx::new();
        let ctx = self.ctx.clone();
        let source = match self.source {
            Source::Text => {
                let mut text = self.input.read(cx).value().to_string().into_bytes();
                if self.add_newline {
                    text.push(b'\n');
                }
                Ok(text)
            }
            Source::File => match &self.file {
                Some(path) => Err(path.clone()),
                None => {
                    self.digests.clear();
                    self.stale = false;
                    cx.notify();
                    return;
                }
            },
        };
        self.stale = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let due = this
                .update(cx, |this, _| this.generation == generation)
                .unwrap_or(false);
            if !due {
                return;
            }
            let result = cx
                .background_spawn(async move {
                    let (mut reader, size): (Box<dyn std::io::Read>, u64) = match source {
                        Ok(bytes) => {
                            let n = bytes.len() as u64;
                            (Box::new(std::io::Cursor::new(bytes)), n)
                        }
                        Err(path) => {
                            let file = std::fs::File::open(&path)
                                .map_err(|e| format!("{}: {e}", path.display()))?;
                            let n = file.metadata().map(|m| m.len()).unwrap_or(0);
                            (Box::new(file), n)
                        }
                    };
                    hash::digest_all(&ctx, &Algorithm::ALL, &mut reader)
                        .map(|d| (d, size))
                        .map_err(|e| e.message)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.stale = false;
                match result {
                    Ok((digests, size)) => {
                        this.error = None;
                        this.hashed = size;
                        this.digests = Algorithm::ALL.iter().copied().zip(digests).collect();
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Hash".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| this.set_file(path, window, cx));
            }
        })
        .detach();
    }

    /// The `tw` command for the algorithm that matched, else SHA-256.
    fn command_line(&self, cx: &Context<Self>) -> String {
        let command = self
            .matched(cx)
            .and_then(hash::command_of)
            .unwrap_or("sha256sum");
        match (self.source, &self.file) {
            (Source::File, Some(path)) => {
                format!("tw {command} {}", quote(&path.to_string_lossy()))
            }
            (Source::File, None) => format!("tw {command} FILE"),
            (Source::Text, _) => {
                let text = self.input.read(cx).value();
                if text.is_empty() || text.contains('\n') || text.len() > 60 {
                    format!("tw {command} < input.txt")
                } else if self.add_newline {
                    format!("echo {} | tw {command}", quote(&text))
                } else {
                    format!("printf %s {} | tw {command}", quote(&text))
                }
            }
        }
    }
}

/// "3.0 MB (3,145,728 bytes)"
fn size(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let units = [(1u64 << 30, "GB"), (1 << 20, "MB"), (1 << 10, "KB")];
    match units.iter().find(|(n, _)| bytes >= *n) {
        Some((n, unit)) => format!("{:.1} {unit} ({grouped} bytes)", bytes as f64 / *n as f64),
        None => format!("{grouped} bytes"),
    }
}

fn muted(text: impl Into<SharedString>, cx: &Context<HashesPage>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

fn segmented(
    id: &'static str,
    labels: [&'static str; 2],
    selected: usize,
    on_click: impl Fn(&mut HashesPage, usize, &mut Window, &mut Context<HashesPage>) + 'static,
    cx: &mut Context<HashesPage>,
) -> TabBar {
    TabBar::new(id)
        .segmented()
        .selected_index(selected)
        .children(labels.map(|l| Tab::new().label(l)))
        .on_click(cx.listener(move |this, ix: &usize, window, cx| on_click(this, *ix, window, cx)))
}

impl Render for HashesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let matched = self.matched(cx);
        let compare_text = self.compare.read(cx).value();
        let command_line = self.command_line(cx);
        let source = match self.source {
            Source::Text => 0,
            Source::File => 1,
        };
        let input_note = match self.source {
            Source::Text if self.add_newline => {
                format!("{} bytes, UTF-8, with a newline at the end", self.hashed)
            }
            Source::Text => format!("{} bytes, UTF-8, no newline at the end", self.hashed),
            Source::File if self.file.is_some() => size(self.hashed),
            Source::File => String::new(),
        };
        let compare_note = match (compare_text.trim().is_empty(), matched) {
            (true, _) => None,
            (false, Some(a)) => Some((format!("✓ Matches {}", a.title()), true)),
            (false, None) => Some(("No algorithm here gives this hash".to_owned(), false)),
        };
        let border = cx.theme().border;
        let match_bg = cx.theme().success.opacity(0.12);
        let rows = self
            .digests
            .iter()
            .enumerate()
            .map(|(i, (algorithm, digest))| {
                let shown = self.format(digest);
                let copy = shown.clone();
                let is_match = matched == Some(*algorithm);
                h_flex()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(border)
                    .when(is_match, |row| row.bg(match_bg))
                    .child(
                        div()
                            .w_24()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if is_match {
                                format!("{} ✓", algorithm.title())
                            } else {
                                algorithm.title()
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(crate::MONO_FONT)
                            .text_sm()
                            .child(shown),
                    )
                    .child(
                        Button::new(("copy-digest", i))
                            .ghost()
                            .label("Copy")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }),
                    )
            })
            .collect::<Vec<_>>();
        let (file_name, file_dir) = match &self.file {
            Some(p) => (
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                p.parent()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            None => (
                "No file yet".to_owned(),
                "Drop a file here, or choose one".to_owned(),
            ),
        };

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Hashes"))
                    .child(muted(
                        "MD5, SHA-1, SHA-2, BLAKE2 and BLAKE3 of text or a file, as sha256sum gives them",
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .flex_wrap()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_64()
                            .gap_1()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(muted("Input", cx))
                                    .child(segmented(
                                        "source",
                                        ["Text", "File"],
                                        source,
                                        |this, ix, window, cx| {
                                            this.source = if ix == 0 { Source::Text } else { Source::File };
                                            this.schedule(window, cx);
                                        },
                                        cx,
                                    )),
                            )
                            .when(self.source == Source::Text, |col| {
                                col.child(div().h_24().child(Textarea::new(&self.input).h_full()))
                                    .child(
                                        h_flex()
                                            .gap_3()
                                            .child(muted(input_note.clone(), cx))
                                            .child(
                                                Checkbox::new("add-newline")
                                                    .label("Add one, as echo does")
                                                    .checked(self.add_newline)
                                                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                                        this.add_newline = *checked;
                                                        this.schedule(window, cx);
                                                    })),
                                            ),
                                    )
                            })
                            .when(self.source == Source::File, |col| {
                                col.child(
                                    h_flex()
                                        .h_24()
                                        .gap_3()
                                        .px_4()
                                        .rounded_lg()
                                        .border_1()
                                        .border_dashed()
                                        .border_color(cx.theme().border)
                                        .bg(cx.theme().muted)
                                        .drag_over::<ExternalPaths>(|style, _, _, cx| {
                                            style.border_color(cx.theme().primary)
                                        })
                                        .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                            if let Some(path) = paths.paths().first() {
                                                this.set_file(path.clone(), window, cx);
                                            }
                                        }))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .child(div().font_weight(FontWeight::SEMIBOLD).child(file_name))
                                                .child(muted(file_dir, cx)),
                                        )
                                        .child(
                                            Button::new("choose-file")
                                                .label("Choose a file…")
                                                .on_click(cx.listener(|this, _, window, cx| this.choose_file(window, cx))),
                                        ),
                                )
                                .child(muted(input_note.clone(), cx))
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_64()
                            .gap_1()
                            .child(muted("Compare with", cx))
                            .child(Input::new(&self.compare))
                            .when_some(compare_note, |col, (note, ok)| {
                                col.child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(if ok { cx.theme().success } else { cx.theme().danger })
                                        .child(note),
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(muted("As", cx))
                    .child(segmented(
                        "encoding",
                        ["hex", "Base64"],
                        usize::from(self.base64),
                        |this, ix, _, cx| {
                            this.base64 = ix == 1;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(
                        Checkbox::new("upper")
                            .label("Uppercase")
                            .checked(self.upper)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.upper = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .when(self.stale, |t| t.opacity(0.5))
                    .child(
                        h_flex()
                            .px_4()
                            .py_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(div().w_24().child("Algorithm"))
                            .child(div().flex_1().child("Digest")),
                    )
                    .children(rows),
            )
            .when_some(self.error.clone(), |col, e| {
                col.child(div().text_sm().text_color(cx.theme().danger).child(e))
            })
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_family(crate::MONO_FONT)
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("$ {command_line}")),
                    )
                    .child(
                        Button::new("copy-command")
                            .ghost()
                            .label("Copy command")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(command_line.clone()))
                            }),
                    ),
            )
    }
}
