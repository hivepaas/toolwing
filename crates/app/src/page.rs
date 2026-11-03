//! A group's page: its actions as tabs, input, options and output, run live
//! (docs/design/app.md). The page is generic, built from the tools' params.

use std::time::Duration;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::IndexPath;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::select::{Select, SelectEvent, SelectState};
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use toolwing_core::{Ctx, Error, Field, Group, InputKind, Output, ParamKind, Tool, Value, Values};
use toolwing_tools::{command, group};

/// Typing pauses this long before a run (app.md).
const DEBOUNCE: Duration = Duration::from_millis(60);
/// A choice of more options than this is a dropdown, not a row of tabs.
const TABS_MAX: usize = 6;

type Dropdown = Entity<SelectState<Vec<&'static str>>>;

/// The choices a param offers, what leaving it out means first.
fn choices(kind: ParamKind) -> Vec<&'static str> {
    match kind {
        ParamKind::Choice { options, unset, .. } => unset
            .into_iter()
            .chain(options.iter().map(|(o, _)| *o))
            .collect(),
        _ => Vec::new(),
    }
}

/// The value a choice stands for: empty for what leaving it out means.
fn choice_value(kind: ParamKind, choice: &str) -> String {
    match kind {
        ParamKind::Choice { unset: Some(u), .. } if u == choice => String::new(),
        _ => choice.to_owned(),
    }
}

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
    /// Dropdowns of long choices, by (action, key).
    dropdowns: Vec<(usize, &'static str, Dropdown)>,
    error: Option<Error>,
    /// The last result's named values, shown as rows instead of the editor.
    result_fields: Option<Vec<Field>>,
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
                if !matches!(
                    p.kind,
                    ParamKind::Int { .. } | ParamKind::Text | ParamKind::Secret
                ) {
                    continue;
                }
                // A secret is masked, and kept nowhere but here (app.md).
                let masked = matches!(p.kind, ParamKind::Secret);
                let state = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(initial)
                        .masked(masked)
                });
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
        let mut dropdowns = Vec::new();
        for (action, tool) in tools.iter().enumerate() {
            for p in tool.params() {
                let items = choices(p.kind);
                if items.len() <= TABS_MAX {
                    continue;
                }
                let key = p.key;
                let kind = p.kind;
                let state =
                    cx.new(|cx| SelectState::new(items, Some(IndexPath::new(0)), window, cx));
                subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, _, event: &SelectEvent<Vec<&'static str>>, window, cx| {
                        if let SelectEvent::Confirm(Some(choice)) = event {
                            this.values[action].set(key, Value::Text(choice_value(kind, choice)));
                            this.schedule(window, cx);
                        }
                    },
                ));
                dropdowns.push((action, key, state));
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
            dropdowns,
            error: None,
            result_fields: None,
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
        if let (Some(text), Some((_, _, state))) = (text.clone(), field) {
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        }
        let kind = self
            .tool()
            .params()
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.kind);
        let dropdown = self
            .dropdowns
            .iter()
            .find(|(a, k, _)| *a == self.action && *k == key);
        if let (Some(text), Some(kind), Some((_, _, state))) = (text, kind, dropdown) {
            let index = choices(kind)
                .iter()
                .position(|c| choice_value(kind, c) == text)
                .map(IndexPath::new);
            state.update(cx, |state, cx| state.set_selected_index(index, window, cx));
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

    /// The last result's named values, when it had them.
    pub fn fields(&self) -> Option<&[Field]> {
        self.result_fields.as_deref()
    }

    pub fn select_action(&mut self, action: usize, window: &mut Window, cx: &mut Context<Self>) {
        if action != self.action {
            self.action = action;
            // The other action's result would read as this one's.
            self.error = None;
            self.result_fields = None;
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
                        this.result_fields = match &output {
                            Output::Fields(f) | Output::Record { fields: f, .. } => Some(f.clone()),
                            _ => None,
                        };
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
                ParamKind::Choice { .. } if choices(p.kind).len() > TABS_MAX => {
                    let state = self
                        .dropdowns
                        .iter()
                        .find(|(a, k, _)| *a == action && *k == key)
                        .map(|(_, _, s)| s.clone());
                    h_flex()
                        .gap_2()
                        .child(label(p.label, cx))
                        .when_some(state, |row, state| {
                            row.child(div().w_40().child(Select::new(&state)))
                        })
                        .into_any_element()
                }
                ParamKind::Choice { options, unset, .. } => {
                    // What leaving it out means comes first, as a choice of its own.
                    let choices: Vec<&'static str> = unset
                        .into_iter()
                        .chain(options.iter().map(|(o, _)| *o))
                        .collect();
                    let current = values.text(self.tool().params(), key);
                    let offset = usize::from(unset.is_some());
                    let selected = options
                        .iter()
                        .position(|(o, _)| *o == current)
                        .map_or(0, |i| i + offset);
                    h_flex()
                        .gap_2()
                        .child(label(p.label, cx))
                        .child(
                            TabBar::new(SharedString::from(format!("{action}-{key}")))
                                .segmented()
                                .selected_index(selected)
                                .children(choices.into_iter().map(|c| Tab::new().label(c)))
                                .on_click(cx.listener(move |this, ix: &usize, window, cx| {
                                    let value = match ix.checked_sub(offset) {
                                        None => String::new(),
                                        Some(i) => options
                                            .get(i)
                                            .map_or_else(String::new, |(o, _)| (*o).to_owned()),
                                    };
                                    this.set_value(key, Value::Text(value), window, cx)
                                })),
                        )
                        .into_any_element()
                }
                ParamKind::Int { .. } | ParamKind::Text | ParamKind::Secret => {
                    let field = self
                        .fields
                        .iter()
                        .find(|(a, k, _)| *a == action && *k == key)
                        .map(|(_, _, state)| state.clone());
                    h_flex()
                        .gap_2()
                        .child(label(p.label, cx))
                        .when_some(field, |row, state| {
                            let text = matches!(p.kind, ParamKind::Text | ParamKind::Secret);
                            row.child(
                                div()
                                    .when(text, |d| d.w_56())
                                    .when(!text, |d| d.w_20())
                                    .child(Input::new(&state)),
                            )
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
        Output::Fields(fields) | Output::Record { fields, .. } => fields
            .iter()
            .map(|f| format!("{}: {}", f.label, f.value))
            .collect::<Vec<_>>()
            .join("\n"),
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

/// Named values as rows, each with its own copy button (app.md).
fn field_rows(fields: &[Field], cx: &Context<ToolPage>) -> impl IntoElement {
    let border = cx.theme().border;
    let muted = cx.theme().muted_foreground;
    let mut list = v_flex().rounded_lg().border_1().border_color(border);
    let mut section: Option<&str> = None;
    for (i, f) in fields.iter().enumerate() {
        if f.section.as_deref() != section {
            section = f.section.as_deref();
            if let Some(name) = section {
                list = list.child(
                    div()
                        .px_4()
                        .pt_3()
                        .pb_1()
                        .when(i > 0, |d| d.border_t_1().border_color(border))
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(muted)
                        .child(name.to_uppercase()),
                );
            }
        }
        let copy = f.value.clone();
        let first_of_section = i == 0 || fields[i - 1].section != f.section;
        list = list.child(
            h_flex()
                .gap_3()
                .px_4()
                .py_2()
                .when(i > 0 && !first_of_section, |row| {
                    row.border_t_1().border_color(border)
                })
                .child(
                    div()
                        .w_40()
                        .flex_none()
                        .text_sm()
                        .text_color(muted)
                        .child(f.label.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(crate::MONO_FONT)
                        .text_sm()
                        .child(f.value.clone()),
                )
                .child(
                    Button::new(("copy-field", i))
                        .ghost()
                        .label("Copy")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                ),
        );
    }
    list
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
                        // Named values need the room more than the input does.
                        let narrow = self.result_fields.is_some();
                        row.child(
                            v_flex()
                                .when(narrow, |d| d.w_1_3().flex_none())
                                .when(!narrow, |d| d.flex_1())
                                .min_w_0()
                                .h_full()
                                .gap_1()
                                .child(label("Input", cx))
                                .child(Textarea::new(&self.input).h_full()),
                        )
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
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
                                    .map(|d| match &self.result_fields {
                                        Some(fields) => d.child(
                                            div()
                                                .id("fields")
                                                .size_full()
                                                .overflow_y_scroll()
                                                .child(field_rows(fields, cx)),
                                        ),
                                        None => d.child(
                                            Textarea::new(&self.output).h_full().readonly(true),
                                        ),
                                    }),
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
                            .min_w_0()
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
