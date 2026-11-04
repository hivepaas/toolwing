//! toolwing's tool model (docs/design/architecture.md).
//!
//! A tool is one operation, such as `base64.encode`. Tools that belong together
//! share a group (`base64`), which is one command in the CLI and one page in
//! the app. Neither the CLI nor the app is known here: a tool reads the input
//! it is handed and returns or writes its output.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Marks English shown to people, so `scripts/i18n.py` finds it; the app
/// translates it where it shows it. Expands to the string itself.
#[macro_export]
macro_rules! en {
    ($text:literal) => {
        $text
    };
}

/// Where a tool is listed: the app's sidebar, `tw list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Data,
    Encoding,
    Crypto,
    Time,
    Text,
    Web,
    Network,
}

impl Category {
    pub const ALL: &[Category] = &[
        Category::Data,
        Category::Encoding,
        Category::Crypto,
        Category::Time,
        Category::Text,
        Category::Web,
        Category::Network,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Data => en!("Data"),
            Category::Encoding => en!("Encoding"),
            Category::Crypto => en!("Crypto"),
            Category::Time => en!("Time"),
            Category::Text => en!("Text"),
            Category::Web => en!("Web"),
            Category::Network => en!("Network"),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Category::Data => "data",
            Category::Encoding => "encoding",
            Category::Crypto => "crypto",
            Category::Time => "time",
            Category::Text => "text",
            Category::Web => "web",
            Category::Network => "network",
        }
    }
}

/// What a tool's main input is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// Nothing: the tool generates (a UUID, a password).
    None,
    /// UTF-8 text. With `trim_newline`, one newline at the end is dropped, so
    /// `echo` and `printf` give the same result (cli.md).
    Text { trim_newline: bool },
    /// Any bytes.
    Bytes,
}

/// How closely a tool follows its reference (tools.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compat {
    /// The reference's flags, output bytes and exit codes; its inputs are files.
    DropIn,
    /// The reference's results, our own flags.
    Output,
    /// A specification, no single tool.
    Standard,
}

/// The tool or specification a tool matches and is tested against.
#[derive(Clone, Copy, Debug)]
pub struct Reference {
    pub name: &'static str,
    pub version: &'static str,
    pub compat: Compat,
}

/// A group of tools: one command in the CLI, one page in the app.
#[derive(Debug)]
pub struct Group {
    pub name: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
}

#[derive(Debug)]
pub struct Meta {
    /// `group.action`, or the group alone for a group of one: `base64.encode`, `uuid`.
    pub id: &'static str,
    pub group: &'static str,
    /// The action's name in the CLI (`tw url decode`); empty for a group of one.
    pub action: &'static str,
    /// For drop-in groups, the flag that selects this action instead of the
    /// group's first (`tw base64 -d`).
    pub action_flag: Option<(char, &'static str)>,
    pub title: &'static str,
    pub summary: &'static str,
    pub category: Category,
    pub keywords: &'static [&'static str],
    pub input: InputKind,
    /// Can run in constant memory through [`Tool::run_stream`].
    pub streams: bool,
    pub reference: Option<Reference>,
}

impl Meta {
    pub fn drop_in(&self) -> bool {
        matches!(
            self.reference,
            Some(Reference {
                compat: Compat::DropIn,
                ..
            })
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ParamKind {
    Flag,
    /// One of these values, each with its help. With `flags`, the CLI offers
    /// each as its own flag (`--v4`, `--v7`) rather than `--key value`.
    /// `unset` names what leaving it out means ("All", "Guess"), for the
    /// app to offer as a choice of its own; `None` when the first option is
    /// the default.
    Choice {
        options: &'static [(&'static str, &'static str)],
        flags: bool,
        unset: Option<&'static str>,
    },
    Int {
        min: i64,
        max: i64,
    },
    Text,
    /// A secret: a password, a signing key. The CLI also takes it from a file
    /// or an environment variable (`--key-file`, `--key-env`), "copy as
    /// command" writes it as `--key-env TW_SECRET`, never in clear, and the
    /// app masks it and keeps it nowhere (cli.md, app.md).
    Secret,
}

/// The names a param has on the command line, spelled as the reference spells them.
#[derive(Clone, Copy, Debug)]
pub struct Cli {
    pub long: &'static str,
    pub short: Option<char>,
}

#[derive(Debug)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: ParamKind,
    pub default: Value,
    pub cli: Cli,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// A tool's param values, by key; missing keys take the param's default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Values(BTreeMap<String, Value>);

impl Values {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.0.insert(key.to_owned(), value);
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn flag(&self, params: &[Param], key: &str) -> bool {
        match self.resolve(params, key) {
            Value::Bool(b) => b,
            _ => false,
        }
    }

    pub fn int(&self, params: &[Param], key: &str) -> i64 {
        match self.resolve(params, key) {
            Value::Int(n) => n,
            _ => 0,
        }
    }

    pub fn text(&self, params: &[Param], key: &str) -> String {
        match self.resolve(params, key) {
            Value::Text(s) => s,
            _ => String::new(),
        }
    }

    fn resolve(&self, params: &[Param], key: &str) -> Value {
        self.0.get(key).cloned().unwrap_or_else(|| {
            params
                .iter()
                .find(|p| p.key == key)
                .map(|p| p.default.clone())
                .unwrap_or(Value::Bool(false))
        })
    }
}

/// One named value of a result: a JWT's claim, a time in one format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The key in `--json` output: snake_case, stable (cli.md).
    pub key: String,
    /// What people read: the CLI's aligned lines, the app's rows.
    pub label: String,
    pub value: String,
    /// The heading the field shows under (a JWT's Header, Payload), if any.
    pub section: Option<String>,
}

impl Field {
    pub fn new(key: impl Into<String>, label: impl Into<String>, value: impl Into<String>) -> Self {
        Field {
            key: key.into(),
            label: label.into(),
            value: value.into(),
            section: None,
        }
    }

    pub fn in_section(mut self, section: impl Into<String>) -> Self {
        self.section = Some(section.into());
        self
    }
}

/// What a run gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Text(String),
    /// Several values, one per line; `--json` makes them an array.
    Lines(Vec<String>),
    /// Named values: aligned lines for people, a JSON object with `--json`.
    Fields(Vec<Field>),
    /// Named values for people, and the JSON `--json` writes as it is: for
    /// results whose values are not all strings (a JWT's claims).
    Record {
        fields: Vec<Field>,
        json: String,
    },
    Bytes(Vec<u8>),
    /// A picture, as PNG (a QR code): the app shows it, the CLI writes the
    /// bytes.
    Image(Vec<u8>),
}

/// Where in the input an error is, for the CLI to print and the app to underline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Byte offset in the input.
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

impl Span {
    /// The line and column (both from 1) of a byte offset in `input`.
    pub fn at(input: &[u8], offset: usize) -> Span {
        let before = &input[..offset.min(input.len())];
        let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
        let start = before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let column = String::from_utf8_lossy(&before[start..]).chars().count() + 1;
        Span {
            offset,
            line,
            column,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The options do not make sense together.
    Usage,
    /// The input is not what the tool reads.
    Input,
    /// A check ran and failed: a hash, a signature, a password.
    Check,
    /// Reading or writing failed.
    Io,
    /// The run was cancelled.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub span: Option<Span>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind,
            message: message.into(),
            span: None,
        }
    }

    pub fn input(message: impl Into<String>) -> Self {
        Error::new(ErrorKind::Input, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Error::new(ErrorKind::Usage, message)
    }

    pub fn at(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(ErrorKind::Io, e.to_string())
    }
}

/// A run's context: cancellation, checked by long loops, and whether the
/// run is the app's. The CLI never cancels; the app cancels when the input
/// changes, and holds costly params to limits (architecture.md).
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    cancelled: Arc<AtomicBool>,
    interactive: bool,
}

impl Ctx {
    pub fn new() -> Self {
        Self::default()
    }

    /// The app's context: a costly param past its limit is refused, as a
    /// run that cannot be cancelled would hold a thread until it ends.
    pub fn interactive() -> Self {
        Ctx {
            interactive: true,
            ..Self::default()
        }
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn check(&self) -> Result<(), Error> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(Error::new(ErrorKind::Cancelled, "cancelled"))
        } else {
            Ok(())
        }
    }
}

/// How sure a tool is that a sample is its kind of input, from 0 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score(pub u8);

pub trait Tool: Sync {
    fn meta(&self) -> &'static Meta;

    fn params(&self) -> &'static [Param] {
        &[]
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error>;

    /// Runs over a stream. The default reads all input, runs, and writes the result.
    fn run_stream(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        values: &Values,
    ) -> Result<(), Error> {
        let mut all = Vec::new();
        input.read_to_end(&mut all)?;
        match self.run(ctx, &all, values)? {
            Output::Text(s) => out.write_all(s.as_bytes())?,
            Output::Lines(lines) => {
                for line in lines {
                    out.write_all(line.as_bytes())?;
                    out.write_all(b"\n")?;
                }
            }
            Output::Fields(fields) | Output::Record { fields, .. } => {
                for f in fields {
                    writeln!(out, "{}: {}", f.label, f.value)?;
                }
            }
            Output::Bytes(b) | Output::Image(b) => out.write_all(&b)?,
        }
        Ok(())
    }

    fn detect(&self, _sample: &[u8]) -> Option<Score> {
        None
    }

    /// A run takes long by design (a password hash): the app runs it on
    /// Ctrl+Enter, not as the input changes (app.md).
    fn costly(&self) -> bool {
        false
    }

    /// The input is a secret (a password): the app masks it, and "copy as
    /// command" reads it from TW_SECRET, never writing it (cli.md).
    fn secret_input(&self) -> bool {
        false
    }

    /// The arguments after `tw`, for a command whose line is not made of
    /// its params as flags (argon2's salt comes first); `None` for the rest.
    fn command_args(&self, _values: &Values) -> Option<Vec<String>> {
        None
    }

    /// The input is the command's last argument (`ipcalc 10.0.0.1/8`), not
    /// its stdin; with [`Tool::command_args`].
    fn input_in_args(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_counts_lines_and_characters() {
        let input = "ab\ncé\nxyz".as_bytes();
        assert_eq!(
            Span::at(input, 0),
            Span {
                offset: 0,
                line: 1,
                column: 1
            }
        );
        // "é" is two bytes but one column.
        let z = input.len() - 1;
        assert_eq!(Span::at(input, z).line, 3);
        assert_eq!(Span::at(input, z).column, 3);
        assert_eq!(Span::at(input, 6).column, 3);
    }

    #[test]
    fn values_fall_back_to_defaults() {
        static PARAMS: &[Param] = &[Param {
            key: "wrap",
            label: "Wrap",
            help: "",
            kind: ParamKind::Int { min: 0, max: 1000 },
            default: Value::Int(76),
            cli: Cli {
                long: "wrap",
                short: Some('w'),
            },
        }];
        let mut v = Values::new();
        assert_eq!(v.int(PARAMS, "wrap"), 76);
        v.set("wrap", Value::Int(0));
        assert_eq!(v.int(PARAMS, "wrap"), 0);
    }
}
