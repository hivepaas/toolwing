//! The Diff page (app.md, custom pages): the text before and after, and the
//! unified diff between them, its lines coloured. One page for `tw diff`.

use std::time::Duration;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use toolwing_tools::diff::{self, Options};

/// Typing pauses this long before the diff is made (app.md).
const DEBOUNCE: Duration = Duration::from_millis(60);

pub struct DiffPage {
    before: Entity<TextareaState>,
    after: Entity<TextareaState>,
    context: Entity<InputState>,
    options: Options,
    /// The unified diff's lines, its two header lines left out.
    lines: Vec<String>,
    added: usize,
    removed: usize,
    stale: bool,
    generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl DiffPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let before = cx.new(|cx| TextareaState::new(window, cx).placeholder("The text before"));
        let after = cx.new(|cx| TextareaState::new(window, cx).placeholder("The text after"));
        let context = cx.new(|cx| InputState::new(window, cx).default_value("3"));
        let on_change =
            |this: &mut Self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
                if let InputEvent::Change = event {
                    this.schedule(window, cx);
                }
            };
        let subscriptions = vec![
            cx.subscribe_in(&before, window, move |this, _, e: &InputEvent, w, cx| {
                on_change(this, e, w, cx)
            }),
            cx.subscribe_in(&after, window, move |this, _, e: &InputEvent, w, cx| {
                on_change(this, e, w, cx)
            }),
            cx.subscribe_in(&context, window, move |this, _, e: &InputEvent, w, cx| {
                on_change(this, e, w, cx)
            }),
        ];
        let mut page = DiffPage {
            before,
            after,
            context,
            options: Options::default(),
            lines: Vec::new(),
            added: 0,
            removed: 0,
            stale: false,
            generation: 0,
            _subscriptions: subscriptions,
        };
        page.schedule(window, cx);
        page
    }

    /// Replaces both texts, as typing or pasting them would.
    pub fn set_texts(
        &mut self,
        before: &str,
        after: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.before
            .update(cx, |s, cx| s.set_value(before.to_owned(), window, cx));
        self.after
            .update(cx, |s, cx| s.set_value(after.to_owned(), window, cx));
        self.schedule(window, cx);
    }

    pub fn set_options(&mut self, options: Options, window: &mut Window, cx: &mut Context<Self>) {
        self.options = options;
        self.schedule(window, cx);
    }

    pub fn settled(&self) -> bool {
        !self.stale
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.added, self.removed)
    }

    fn context_lines(&self, cx: &gpui::App) -> usize {
        self.context
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(3)
            .min(1_000)
    }

    fn schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let before = self.before.read(cx).value().to_string();
        let after = self.after.read(cx).value().to_string();
        let options = self.options;
        let context = self.context_lines(cx);
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
                    let (a, b) = (
                        diff::lines(before.as_bytes()),
                        diff::lines(after.as_bytes()),
                    );
                    let text = diff::unified(&a, &b, options, context, ("before", "after"))
                        .unwrap_or_default();
                    let lines: Vec<String> = String::from_utf8_lossy(&text)
                        .lines()
                        .skip(2)
                        .map(str::to_owned)
                        .collect();
                    (lines, diff::counts(&a, &b, options))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.stale = false;
                (this.lines, (this.added, this.removed)) = result;
                cx.notify();
            });
        })
        .detach();
    }

    fn command_line(&self, cx: &gpui::App) -> String {
        let mut line = String::from("tw diff");
        let context = self.context_lines(cx);
        if context == 3 {
            line.push_str(" -u");
        } else {
            line.push_str(&format!(" -U {context}"));
        }
        for (on, flag) in [
            (self.options.ignore_case, " -i"),
            (self.options.ignore_all_space, " -w"),
            (self.options.ignore_space_change, " -b"),
        ] {
            if on {
                line.push_str(flag);
            }
        }
        line.push_str(" before.txt after.txt");
        line
    }
}

fn muted(text: impl Into<SharedString>, cx: &Context<DiffPage>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

impl Render for DiffPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (added_bg, removed_bg) = (theme.success.opacity(0.14), theme.danger.opacity(0.12));
        let (added_fg, removed_fg, muted_fg) =
            (theme.success, theme.danger, theme.muted_foreground);
        let border = theme.border;
        let rows = self
            .lines
            .iter()
            .map(|line| {
                let row = div()
                    .px_3()
                    .font_family(crate::MONO_FONT)
                    .text_sm()
                    .child(line.clone());
                match line.as_bytes().first() {
                    Some(b'+') => row.bg(added_bg).text_color(added_fg),
                    Some(b'-') => row.bg(removed_bg).text_color(removed_fg),
                    Some(b'@') => row.mt_1().text_color(muted_fg),
                    Some(b'\\') => row.text_color(muted_fg),
                    _ => row,
                }
            })
            .collect::<Vec<_>>();
        let same = self.lines.is_empty() && !self.stale;
        let command_line = self.command_line(cx);
        let flag = |id: &'static str,
                    label: &'static str,
                    on: bool,
                    set: fn(&mut Options, bool),
                    cx: &mut Context<Self>| {
            Checkbox::new(id)
                .label(label)
                .checked(on)
                .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                    let mut o = this.options;
                    set(&mut o, *checked);
                    this.set_options(o, window, cx);
                }))
        };
        let o = self.options;
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
                            .child("Diff"),
                    )
                    .child(muted("Compare two texts line by line, as diff -u does", cx)),
            )
            .child(
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(muted("Context lines", cx))
                            .child(div().w_16().child(Input::new(&self.context))),
                    )
                    .child(flag(
                        "ignore-case",
                        "Ignore case",
                        o.ignore_case,
                        |o, v| o.ignore_case = v,
                        cx,
                    ))
                    .child(flag(
                        "ignore-all-space",
                        "Ignore all white space",
                        o.ignore_all_space,
                        |o, v| o.ignore_all_space = v,
                        cx,
                    ))
                    .child(flag(
                        "ignore-space-change",
                        "Ignore changes in white space",
                        o.ignore_space_change,
                        |o, v| o.ignore_space_change = v,
                        cx,
                    ))
                    .child(
                        div()
                            .ml_auto()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_color(added_fg)
                                            .child(format!("+{}", self.added)),
                                    )
                                    .child(
                                        div()
                                            .text_color(removed_fg)
                                            .child(format!("−{}", self.removed)),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .h_40()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .gap_1()
                            .child(muted("Before", cx))
                            .child(Textarea::new(&self.before).h_full()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .gap_1()
                            .child(muted("After", cx))
                            .child(Textarea::new(&self.after).h_full()),
                    ),
            )
            .child(
                div()
                    .id("diff")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .rounded_lg()
                    .border_1()
                    .border_color(border)
                    .py_2()
                    .when(self.stale, |d| d.opacity(0.5))
                    .when(same, |d| {
                        d.child(
                            div()
                                .px_3()
                                .text_sm()
                                .text_color(muted_fg)
                                .child("The texts are the same"),
                        )
                    })
                    .children(rows),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_family(crate::MONO_FONT)
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
