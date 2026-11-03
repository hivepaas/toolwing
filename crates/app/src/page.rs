//! A group's page: its actions as tabs, input, options and output, run live
//! (docs/design/app.md). The page is generic, built from the tools' params.

use std::time::Duration;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, FontWeight, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use toolwing_core::{Ctx, Error, Group, InputKind, Output, ParamKind, Tool, Value, Values};
use toolwing_tools::{command, group};

/// Typing pauses this long before a run (app.md).
const DEBOUNCE: Duration = Duration::from_millis(60);
/// Output longer than this shows in part.
const SHOWN_MAX: usize = 1 << 20;

pub struct ToolPage {
    group: &'static Group,
    tools: Vec<&'static dyn Tool>,
    action: usize,
    /// Each action's values, kept while switching between them.
    values: Vec<Values>,
    input: Entity<TextareaState>,
    output: Entity<TextareaState>,
    /// Text fields of Int and Text params, by (action, key).
    fields: Vec<(usize, &'static str, Entity<InputState>)>,
    error: Option<Error>,
    /// The output shown is from an earlier input: dimmed while a run is due.
    stale: bool,
    generation: u64,
    ctx: Ctx,
    _subscriptions: Vec<Subscription>,
}

impl ToolPage {
    pub fn new(group_info: &'static Group, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tools = group(group_info.name);
        let input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Type or paste the input here"));
        let output = cx.new(|cx| TextareaState::new(window, cx));
        let mut subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::Change = event {
                        this.schedule(window, cx);
                    }
                }),
            ];
        let mut fields = Vec::new();
        for (action, tool) in tools.iter().enumerate() {
            for p in tool.params() {
                let initial = match &p.default {
                    Value::Int(n) => n.to_string(),
                    Value::Text(s) => s.clone(),
                    Value::Bool(_) => continue,
                };
                if !matches!(p.kind, ParamKind::Int { .. } | ParamKind::Text) {
                    continue;
                }
                let state = cx.new(|cx| InputState::new(window, cx).default_value(initial));
                let key = p.key;
                subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, state, event: &InputEvent, window, cx| {
                        if let InputEvent::Change = event {
                            let text = state.read(cx).value().to_string();
                            this.set_field(action, key, &text, window, cx);
                        }
                    },
                ));
                fields.push((action, key, state));
            }
        }
        let mut page = ToolPage {
            group: group_info,
            values: vec![Values::new(); tools.len()],
            tools,
            action: 0,
            input,
            output,
            fields,
            error: None,
            stale: false,
            generation: 0,
            ctx: Ctx::new(),
            _subscriptions: subscriptions,
        };
        page.schedule(window, cx);
        page
    }

    fn tool(&self) -> &'static dyn Tool {
        self.tools[self.action]
    }

    pub fn set_value(
        &mut self,
        key: &str,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A text field shows what was set from elsewhere too.
        let text = match &value {
            Value::Int(n) => Some(n.to_string()),
            Value::Text(s) => Some(s.clone()),
            Value::Bool(_) => None,
        };
        let field = self
            .fields
            .iter()
            .find(|(a, k, _)| *a == self.action && *k == key);
        if let (Some(text), Some((_, _, state))) = (text, field) {
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        }
        self.values[self.action].set(key, value);
        self.schedule(window, cx);
    }

    /// A text field changed: an Int is applied when it parses and is in range.
    fn set_field(
        &mut self,
        action: usize,
        key: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.tools[action].params().iter().find(|p| p.key == key) else {
            return;
        };
        let value = match p.kind {
            ParamKind::Int { min, max } => match text.trim().parse::<i64>() {
                Ok(n) if (min..=max).contains(&n) => Value::Int(n),
                _ => {
                    self.error = Some(Error::usage(format!(
                        "{} must be a whole number from {min} to {max}",
                        p.label
                    )));
                    cx.notify();
                    return;
                }
            },
            _ => Value::Text(text.to_owned()),
        };
        self.values[action].set(key, value);
        self.schedule(window, cx);
    }

    /// Replaces the input, as typing or pasting it would.
    pub fn set_input(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.schedule(window, cx);
    }

    /// The last run has finished and its result is shown.
    pub fn settled(&self) -> bool {
        !self.stale
    }

    pub fn output(&self, cx: &gpui::App) -> String {
        self.output.read(cx).value().to_string()
    }

    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    pub fn select_action(&mut self, action: usize, window: &mut Window, cx: &mut Context<Self>) {
        if action != self.action {
            self.action = action;
            // The other action's result would read as this one's.
            self.error = None;
            self.output
                .update(cx, |o, cx| o.set_value(String::new(), window, cx));
            self.schedule(window, cx);
        }
    }

    /// Runs the tool after a pause, in the background; a newer run cancels
    /// this one, and a result that is not the latest is dropped.
    fn schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.ctx.cancel();
        self.ctx = Ctx::new();
        let ctx = self.ctx.clone();
        let tool = self.tool();
        let values = self.values[self.action].clone();
        let input = self.input.read(cx).value().to_string();
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
            let bytes = prepare(tool.meta().input, input);
            let result = cx
                .background_spawn(async move { tool.run(&ctx, &bytes, &values) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.stale = false;
                match result {
                    Ok(output) => {
                        this.error = None;
                        let text = shown(&output);
                        this.output
                            .update(cx, |o, cx| o.set_value(text, window, cx));
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn command_line(&self, cx: &Context<Self>) -> String {
        let input = self.input.read(cx).value();
        command::shell_line(self.tool(), &self.values[self.action], &input)
    }

    fn options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let action = self.action;
        let values = &self.values[action];
        let mut row = h_flex().gap_4().flex_wrap().items_center();
        for p in self.tool().params() {
            let key = p.key;
            row = row.child(match p.kind {
                ParamKind::Flag => Checkbox::new(SharedString::from(format!("{action}-{key}")))
                    .label(p.label)
                    .checked(values.flag(self.tool().params(), key))
                    .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                        this.set_value(key, Value::Bool(*checked), window, cx)
                    }))
                    .into_any_element(),
                ParamKind::Choice { options, .. } => {
                    let current = values.text(self.tool().params(), key);
                    let selected = options.iter().position(|(o, _)| *o == current).unwrap_or(0);
                    h_flex()
                        .gap_2()
                        .child(label(p.label, cx))
                        .child(
                            TabBar::new(SharedString::from(format!("{action}-{key}")))
                                .segmented()
                                .selected_index(selected)
                                .children(options.iter().map(|(o, _)| Tab::new().label(*o)))
                                .on_click(cx.listener(move |this, ix: &usize, window, cx| {
                                    if let Some((o, _)) = options.get(*ix) {
                                        this.set_value(key, Value::Text((*o).into()), window, cx)
                                    }
                                })),
                        )
                        .into_any_element()
                }
                ParamKind::Int { .. } | ParamKind::Text => {
                    let field = self
                        .fields
                        .iter()
                        .find(|(a, k, _)| *a == action && *k == key)
                        .map(|(_, _, state)| state.clone());
                    h_flex()
                        .gap_2()
                        .child(label(p.label, cx))
                        .when_some(field, |row, state| {
                            row.child(div().w_24().child(Input::new(&state)))
                        })
                        .into_any_element()
                }
            });
        }
        row
    }
}

/// The input as the tool reads it.
fn prepare(kind: InputKind, input: String) -> Vec<u8> {
    match kind {
        InputKind::None => Vec::new(),
        InputKind::Text { trim_newline: true } => input
            .strip_suffix('\n')
            .unwrap_or(&input)
            .as_bytes()
            .to_vec(),
        _ => input.into_bytes(),
    }
}

/// The output as text for the output editor.
fn shown(output: &Output) -> String {
    let text = match output {
        Output::Text(s) => s.clone(),
        Output::Lines(lines) => lines.join("\n"),
        Output::Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) => s.to_owned(),
            Err(_) => format!("{} bytes of binary data", b.len()),
        },
    };
    if text.len() > SHOWN_MAX {
        let mut end = SHOWN_MAX;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n… ({} bytes in all)", &text[..end], text.len())
    } else {
        text
    }
}

/// "decode" → "Decode": the tab under the group's title says only the action.
fn action_label(action: &str) -> String {
    let mut chars = action.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn label(text: &'static str, cx: &Context<ToolPage>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

impl Render for ToolPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let meta = self.tool().meta();
        let takes_input = meta.input != InputKind::None;
        let command_line = self.command_line(cx);
        let error = self.error.as_ref().map(|e| match e.span {
            Some(s) => format!("{} (line {}, column {})", e.message, s.line, s.column),
            None => e.message.clone(),
        });
        let actions = (self.tools.len() > 1).then(|| {
            TabBar::new("actions")
                .underline()
                .selected_index(self.action)
                .children(
                    self.tools
                        .iter()
                        .map(|t| Tab::new().label(action_label(t.meta().action))),
                )
                .on_click(
                    cx.listener(|this, ix: &usize, window, cx| this.select_action(*ix, window, cx)),
                )
        });
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
                            .child(self.group.title),
                    )
                    .child(label(self.group.summary, cx)),
            )
            .children(actions)
            .child(self.options(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .when(takes_input, |row| {
                        row.child(
                            v_flex()
                                .flex_1()
                                .h_full()
                                .gap_1()
                                .child(label("Input", cx))
                                .child(Textarea::new(&self.input).h_full()),
                        )
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .gap_1()
                            .child(h_flex().justify_between().child(label("Output", cx)).when(
                                !takes_input,
                                |row| {
                                    row.child(
                                        Button::new("again")
                                            .ghost()
                                            .label("Generate again")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.schedule(window, cx)
                                            })),
                                    )
                                },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .when(self.stale || self.error.is_some(), |d| d.opacity(0.5))
                                    .child(Textarea::new(&self.output).h_full().readonly(true)),
                            )
                            .when_some(error, |col, message| {
                                col.child(
                                    div().text_sm().text_color(cx.theme().danger).child(message),
                                )
                            }),
                    ),
            )
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
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    command_line.clone(),
                                ))
                            }),
                    ),
            )
    }
}
