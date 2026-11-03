# The desktop app

One window, built with GPUI and gpui-component, themed with the -wing apps'
palette so it sits with shellwing and tracewing.

## Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ ◧  toolwing          [ Search tools…                        Ctrl K ] │
├────────────────┬─────────────────────────────────────────────────────┤
│ ★ Favorites    │ JWT                          Decode │ Verify │ Encode│
│   jq           │ ┌ Input ──────────────────┐ ┌ Output ──────────────┐│
│   JWT          │ │ eyJhbGciOiJIUzI1NiIsInR5│ │ alg   HS256       ⧉  ││
│ ⟲ Recent       │ │ cCI6IkpXVCJ9.eyJzdWIiOi │ │ sub   1234567890  ⧉  ││
│                │ │ …                       │ │ exp   2026-11-04 …⧉  ││
│ Data           │ │                         │ │       (in 23 hours)  ││
│ Encoding       │ └─────────────────────────┘ └──────────────────────┘│
│ Crypto         │ Options: [✓] Show times in local zone               │
│ Time           │                                                     │
│ Text           │ $ tw jwt decode --json "$TOKEN"                  ⧉  │
│ Web · Network  │                                                     │
├────────────────┴─────────────────────────────────────────────────────┤
│ 📋 Clipboard looks like a JWT: Decode it  ·  Dismiss                 │
└──────────────────────────────────────────────────────────────────────┘
```

- **Sidebar:** favorites, recent tools, then categories. Resizable, can be
  hidden.
- **A tool's page:** the group's actions as tabs, input on the left, output on
  the right (stacked when the window is narrow), options in one row under them.
- **Command line:** the `tw` command that gives the result on screen, with a
  copy button. Secret values appear as `"$TW_SECRET"`, never in clear text.

## Live results

- Every change of input or option runs the tool again, after 60 ms without
  typing. The run goes to the background, the previous one is cancelled, and a
  stale result is never shown (architecture.md).
- Over 1 MB of input, or for a tool marked costly (bcrypt, argon2), results
  come on Ctrl+Enter instead, with a note saying why.
- An error replaces the output with its message, and its span is underlined in
  the input or the filter. The last good output stays visible, dimmed, so the
  layout does not jump while typing.
- jq and the other tools that run user-written code or parse deeply nested
  input run in the worker process; a crash or a timeout shows as an error.

## Editors

- gpui-component's editor for input and output: line numbers, syntax highlight
  for JSON, YAML, TOML, XML and SQL (its tree-sitter features), folding, find.
- Output editors are read-only; output above 10 MB is shown in part with "Save
  to file" and "Copy all".
- Open a file into the input by drag and drop or Ctrl+O; save the output with
  Ctrl+S. Binary output is offered as a file, never pasted as text.

## The palette

Ctrl+K (Cmd+K on macOS) finds tools by title, group, keyword and id, and runs
app actions (theme, favorites, settings). The same fuzzy matching as
shellwing's palette.

## Clipboard suggestions

- When the window gains focus, the app reads the clipboard (up to 64 KB of
  text), asks every tool's `detect`, and offers the best match above a
  threshold in the bottom bar: "Clipboard looks like a JWT: Decode it".
- It reads only on focus, never in the background, never keeps or sends the
  text. Settings has a switch to turn it off; the first time it fires, the bar
  says how to.

## What is kept

- In `state/`, as the other -wing apps do: window size, sidebar width,
  favorites, recent tool ids, each tool's last options.
- **Inputs are not kept.** They are often tokens and keys. "Restore inputs on
  start" exists in Settings, off by default, and never applies to `Secret`
  params.
- Settings in `config.toml`: theme, font size, clipboard suggestions, live-run
  delay, the 1 MB threshold.

## Custom pages

Groups whose generic page would be poor register their own, calling the same
tools: `color` (picker and contrast), `qr` (preview, save PNG/SVG), `diff`
(side by side and inline), `cron` (the next runs on a calendar), `regex`
(matches highlighted in the text, groups in a table), `time` (several zones at
once, a live clock).

## Keyboard

Everything works without a mouse: Ctrl+K, Ctrl+1–9 for the group's actions,
Ctrl+Enter to run, Ctrl+Shift+C to copy the output, Ctrl+Shift+L to copy the
command line, F6 to move between input, options and output.

**Open:**

- Translations: gpui-component brings `rust-i18n`, shellwing has its own
  `shellwing-i18n`. One of them for both the components and our text, or
  English only in 1.0.
- Accessibility: what GPUI exposes to screen readers as of `gpui-pre` 0.3.11,
  and what that leaves out.
- Several tools open at once (tabs), or one tool at a time as now.
