//! The jq page (app.md, custom pages): a filter, the input, and what the
//! filter gives, run live in the jq worker (jq_worker.rs). The last good
//! output stays, dimmed, while a run is on its way or has failed. The filter
//! is shown as typed: no ligatures, so `!=` is not drawn as `≠`.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, FontFeatures, FontWeight, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use toolwing_core::Tool as _;

use crate::jq_worker::{Failure, MEMORY_LIMIT, Reply, Request, TIME_LIMIT, Worker};

/// Typing pauses this long before a run (app.md).
const DEBOUNCE: Duration = Duration::from_millis(60);

/// The formats the input may be in, and the output.
const FROM: &[(&str, &str)] = &[
    ("json", "JSON"),
    ("yaml", "YAML"),
    ("toml", "TOML"),
    ("csv", "CSV"),
    ("xml", "XML"),
];
const TO: &[(&str, &str)] = &[("json", "JSON"), ("yaml", "YAML")];

/// The worker the app keeps warm for jq (architecture.md).
pub struct JqWorker(pub Arc<Worker>);

impl gpui::Global for JqWorker {}

/// Starts the worker, as the app's binary at `exe`.
pub fn start_worker(exe: std::path::PathBuf, cx: &mut gpui::App) {
    cx.set_global(JqWorker(Arc::new(Worker::new(exe))));
}

pub struct JqPage {
    filter: Entity<InputState>,
    input: Entity<TextareaState>,
    output: Entity<TextareaState>,
    options: Request,
    /// The last run's reply; its outputs stay on screen through a failure.
    reply: Reply,
    /// The outputs on screen, from the last run that gave any.
    shown: usize,
    stale: bool,
    generation: u64,
    worker: Arc<Worker>,
    _subscriptions: Vec<Subscription>,
}

impl JqPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        if cx.try_global::<JqWorker>().is_none() {
            start_worker(Worker::current_exe(), cx);
        }
        let worker = cx.global::<JqWorker>().0.clone();
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(".")
                .placeholder("A jq filter, such as .items[] | .name")
        });
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("The input: JSON, or the format chosen under From")
        });
        let output = cx.new(|cx| TextareaState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::Change = e {
                    this.schedule(window, cx);
                }
            }),
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::Change = e {
                    this.schedule(window, cx);
                }
            }),
        ];
        let mut page = JqPage {
            filter,
            input,
            output,
            options: Request {
                from: "json".into(),
                to: "json".into(),
                ..Request::default()
            },
            reply: Reply::default(),
            shown: 0,
            stale: false,
            generation: 0,
            worker,
            _subscriptions: subscriptions,
        };
        page.schedule(window, cx);
        page
    }

    /// Replaces the filter and the input, as typing or pasting them would.
    pub fn set_texts(
        &mut self,
        filter: &str,
        input: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter
            .update(cx, |s, cx| s.set_value(filter.to_owned(), window, cx));
        self.input
            .update(cx, |s, cx| s.set_value(input.to_owned(), window, cx));
        self.schedule(window, cx);
    }

    /// Sets the options (the filter and input in it are not used).
    pub fn set_options(&mut self, options: Request, window: &mut Window, cx: &mut Context<Self>) {
        self.options = options;
        self.schedule(window, cx);
    }

    pub fn settled(&self) -> bool {
        !self.stale
    }

    pub fn reply(&self) -> &Reply {
        &self.reply
    }

    pub fn output(&self, cx: &gpui::App) -> String {
        self.output.read(cx).value().to_string()
    }

    fn request(&self, cx: &gpui::App) -> Request {
        Request {
            filter: self.filter.read(cx).value().to_string(),
            input: self.input.read(cx).value().to_string(),
            ..self.options.clone()
        }
    }

    fn schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        // The run on its way is for an older filter or input.
        self.worker.cancel();
        let request = self.request(cx);
        let worker = self.worker.clone();
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
            let reply = cx
                .background_spawn(async move { worker.run(&request) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || reply.failure == Some(Failure::Cancelled) {
                    return;
                }
                this.stale = false;
                // A failed run keeps the last good output on screen.
                if reply.failure.is_none() {
                    let text = reply.outputs.concat();
                    let text = text.strip_suffix('\n').unwrap_or(&text).to_owned();
                    this.output
                        .update(cx, |s, cx| s.set_value(text, window, cx));
                    this.shown = reply.outputs.len();
                }
                this.reply = reply;
                cx.notify();
            });
        })
        .detach();
    }

    /// The `tw jq` command that gives what the page shows.
    fn command_line(&self, cx: &gpui::App) -> String {
        let o = &self.options;
        let mut line = String::from("tw jq");
        for (on, flag) in [
            (o.raw, " -r"),
            (o.compact, " -c"),
            (o.slurp, " -s"),
            (o.sort_keys, " -S"),
        ] {
            if on {
                line.push_str(flag);
            }
        }
        if o.from != "json" {
            line.push_str(&format!(" --from {}", o.from));
        }
        if o.to != "json" {
            line.push_str(&format!(" --to {}", o.to));
        }
        let filter = self.filter.read(cx).value().to_string();
        line.push_str(&format!(" {} < input.{}", shell_quote(&filter), o.from));
        line
    }
}

/// `s` as one shell word: in single quotes, a quote in it as `'\''`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A size as people read it: 512 B, 4.1 KB, 3.0 MB.
fn size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

fn muted(text: impl Into<SharedString>, cx: &Context<JqPage>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

impl Render for JqPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (danger, muted_fg, border) = (theme.danger, theme.muted_foreground, theme.border);
        let meta = toolwing_tools::jq::Jq.meta();
        let command_line = self.command_line(cx);
        let input_size = self.input.read(cx).value().len();
        let o = self.options.clone();
        let flag = |id: &'static str,
                    label: &'static str,
                    on: bool,
                    set: fn(&mut Request, bool),
                    cx: &mut Context<Self>| {
            Checkbox::new(id)
                .label(label)
                .checked(on)
                .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                    let mut o = this.options.clone();
                    set(&mut o, *checked);
                    this.set_options(o, window, cx);
                }))
        };
        let formats = |id: &'static str,
                       label: &'static str,
                       list: &'static [(&str, &str)],
                       current: &str,
                       set: fn(&mut Request, String),
                       cx: &mut Context<Self>| {
            let selected = list.iter().position(|(k, _)| *k == current).unwrap_or(0);
            h_flex().gap_2().child(muted(label, cx)).child(
                TabBar::new(id)
                    .segmented()
                    .selected_index(selected)
                    .children(list.iter().map(|(_, l)| Tab::new().label(*l)))
                    .on_click(cx.listener(move |this, ix: &usize, window, cx| {
                        let mut o = this.options.clone();
                        set(&mut o, list.get(*ix).map_or("json", |(k, _)| *k).to_owned());
                        this.set_options(o, window, cx);
                    })),
            )
        };
        let reply = &self.reply;
        let status = if self.stale {
            "Running…".to_owned()
        } else if reply.failure.is_some() {
            String::new()
        } else {
            let n = reply.outputs.len();
            let ms = reply.elapsed.as_millis();
            let results = if n == 1 {
                "1 result".to_owned()
            } else {
                format!("{n} results")
            };
            let cut = if reply.truncated {
                ", the rest left out"
            } else {
                ""
            };
            format!("{results} · {ms} ms{cut}")
        };
        let failure = reply.failure.as_ref().map(Failure::message);
        let errors = if self.stale {
            Vec::new()
        } else {
            reply.errors.clone()
        };
        let dimmed = self.stale || failure.is_some();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(meta.title),
                    )
                    .child(muted(meta.summary, cx)),
            )
            .child(
                v_flex().gap_1().child(muted("Filter", cx)).child(
                    div()
                        .font_family(crate::MONO_FONT)
                        .font_features(FontFeatures::disable_ligatures())
                        .child(Input::new(&self.filter)),
                ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .items_center()
                    .child(flag("jq-raw", "Raw output -r", o.raw, |o, v| o.raw = v, cx))
                    .child(flag(
                        "jq-compact",
                        "Compact -c",
                        o.compact,
                        |o, v| o.compact = v,
                        cx,
                    ))
                    .child(flag(
                        "jq-slurp",
                        "Slurp -s",
                        o.slurp,
                        |o, v| o.slurp = v,
                        cx,
                    ))
                    .child(flag(
                        "jq-sort",
                        "Sort keys -S",
                        o.sort_keys,
                        |o, v| o.sort_keys = v,
                        cx,
                    ))
                    .child(formats(
                        "jq-from",
                        "From",
                        FROM,
                        &o.from,
                        |o, v| o.from = v,
                        cx,
                    ))
                    .child(formats("jq-to", "To", TO, &o.to, |o, v| o.to = v, cx)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .gap_1()
                            .child(muted(format!("Input · {}", size(input_size)), cx))
                            .child(Textarea::new(&self.input).h_full()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .gap_1()
                            .child(muted(
                                if status.is_empty() {
                                    "Output".to_owned()
                                } else {
                                    format!("Output · {status}")
                                },
                                cx,
                            ))
                            .when_some(failure, |col, message| {
                                col.child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(danger)
                                        .text_color(danger)
                                        .font_family(crate::MONO_FONT)
                                        .text_sm()
                                        .child(message),
                                )
                            })
                            .children(errors.into_iter().take(3).map(|e| {
                                div()
                                    .text_sm()
                                    .text_color(danger)
                                    .font_family(crate::MONO_FONT)
                                    .child(format!("error: {e}"))
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .when(dimmed, |d| d.opacity(0.5))
                                    .child(Textarea::new(&self.output).h_full().readonly(true)),
                            ),
                    ),
            )
            .child(div().text_xs().text_color(muted_fg).child(format!(
                "Runs in its own process: stopped after {} s or {} GB",
                TIME_LIMIT.as_secs(),
                MEMORY_LIMIT >> 30
            )))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(border)
                    .pt_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_family(crate::MONO_FONT)
                            .font_features(FontFeatures::disable_ligatures())
                            .text_color(muted_fg)
                            .child(format!("$ {command_line}")),
                    )
                    .child(
                        Button::new("copy-command")
                            .ghost()
                            .label("Copy command")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    command_line.clone(),
                                ))
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_are_quoted_for_the_shell() {
        assert_eq!(shell_quote(".a"), "'.a'");
        assert_eq!(
            shell_quote(r#"select(.n == "it's")"#),
            r#"'select(.n == "it'\''s")'"#
        );
        assert_eq!(size(4200), "4.1 KB");
    }
}
