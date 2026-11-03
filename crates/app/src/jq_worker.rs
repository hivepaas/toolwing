//! jq in a worker process (architecture.md): a jq program is the one thing
//! the app runs that the user writes, and it need not end, may recurse past
//! the stack and may take all memory. The worker is the app's own binary,
//! started as `toolwing --jq-worker`; one is kept warm, and a run that goes
//! on too long, or that a newer run replaces, is stopped by killing it.
//!
//! Requests and replies are JSON messages, each after its length as four
//! bytes (little-endian), on the worker's stdin and stdout.

use std::io::{self, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};
use toolwing_jq::doc::{self, Style};
use toolwing_jq::json::write::Pp;
use toolwing_jq::write::{Writer, write};
use toolwing_jq::{Format, Runner, Stop, Val, Vars};

/// The argument that makes the app's binary a worker.
pub const WORKER_ARG: &str = "--jq-worker";

/// A run is stopped after this long (architecture.md).
pub const TIME_LIMIT: Duration = Duration::from_secs(5);
/// The worker may take this much memory.
pub const MEMORY_LIMIT: u64 = 2 << 30;
/// Output past this is left out.
pub const OUTPUT_LIMIT: usize = 64 << 20;
/// The stack a program runs on in the worker.
const STACK: usize = 256 << 20;

/// What a run asks for: `tw jq`'s options that the page offers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub filter: String,
    pub input: String,
    pub raw: bool,
    pub compact: bool,
    pub slurp: bool,
    pub sort_keys: bool,
    pub null_input: bool,
    /// `json`, `yaml`, `toml`, `csv` or `xml`.
    pub from: String,
    /// `json` or `yaml`.
    pub to: String,
}

/// What a run gave.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    /// The outputs, each as `tw jq` writes it, newline and all.
    pub outputs: Vec<String>,
    /// Left out past [`OUTPUT_LIMIT`].
    pub truncated: bool,
    /// Errors the program raised on an input, as jq reports them and goes on.
    pub errors: Vec<String>,
    /// Why there is no result: the filter does not compile, the input does
    /// not parse, the run was stopped.
    pub failure: Option<Failure>,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    Compile(String),
    Input(String),
    /// `halt` or `halt_error`, with its exit code.
    Halt(i32),
    /// Stopped after [`TIME_LIMIT`].
    TimedOut,
    OutOfMemory,
    /// Too deeply nested for the stack.
    StackOverflow,
    /// The worker ended for another reason.
    Crashed(String),
    /// A newer run took its place.
    Cancelled,
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::Compile(m) | Failure::Input(m) | Failure::Crashed(m) => m.clone(),
            Failure::Halt(code) => format!("Halted with exit code {code}"),
            Failure::TimedOut => format!("Stopped after {} s", TIME_LIMIT.as_secs()),
            Failure::OutOfMemory => format!("Ran out of memory ({} GB)", MEMORY_LIMIT >> 30),
            Failure::StackOverflow => "Too deeply nested: the program ran out of stack".into(),
            Failure::Cancelled => "Stopped for a newer run".into(),
        }
    }
}

// The protocol.

fn send(w: &mut impl Write, message: &Json) -> io::Result<()> {
    let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
    let len = u32::try_from(bytes.len()).map_err(io::Error::other)?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

/// The next message, or `None` when the other end has gone.
fn receive(r: &mut impl Read) -> io::Result<Option<Json>> {
    let mut len = [0; 4];
    match r.read_exact(&mut len) {
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        r => r?,
    }
    let mut bytes = vec![0; u32::from_le_bytes(len) as usize];
    r.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

impl Request {
    fn to_json(&self) -> Json {
        json!({
            "filter": self.filter, "input": self.input, "raw": self.raw,
            "compact": self.compact, "slurp": self.slurp, "sort_keys": self.sort_keys,
            "null_input": self.null_input, "from": self.from, "to": self.to,
        })
    }

    fn from_json(j: &Json) -> Request {
        let s = |k: &str| j[k].as_str().unwrap_or_default().to_owned();
        let b = |k: &str| j[k].as_bool().unwrap_or(false);
        Request {
            filter: s("filter"),
            input: s("input"),
            raw: b("raw"),
            compact: b("compact"),
            slurp: b("slurp"),
            sort_keys: b("sort_keys"),
            null_input: b("null_input"),
            from: s("from"),
            to: s("to"),
        }
    }
}

// The worker's side.

/// Runs as the worker: requests from stdin, replies to stdout, until stdin
/// ends. Each program runs on a thread with a large stack.
pub fn serve() -> io::Result<()> {
    limit_memory();
    toolwing_jq::log_to_stderr();
    let mut stdin = io::stdin().lock();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    while let Some(message) = receive(&mut stdin)? {
        let request = Request::from_json(&message);
        let (tx, rx) = std::sync::mpsc::channel();
        let runner = std::thread::Builder::new()
            .stack_size(STACK)
            .spawn(move || evaluate(&request, |m| tx.send(m).is_ok()))?;
        for m in rx {
            send(&mut stdout, &m)?;
        }
        runner
            .join()
            .map_err(|_| io::Error::other("the run panicked"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn limit_memory() {
    // Safe: the crate does the system call. macOS does not enforce it.
    let _ = rlimit::setrlimit(rlimit::Resource::AS, MEMORY_LIMIT, MEMORY_LIMIT);
}

#[cfg(not(unix))]
fn limit_memory() {}

/// Runs one request, handing each message to `send` (false when nobody
/// listens any more), ending with `{"done": …}`.
fn evaluate(r: &Request, send: impl FnMut(Json) -> bool) {
    // Outputs and errors both send.
    let send = std::cell::RefCell::new(send);
    let send = |m: Json| (send.borrow_mut())(m);
    let done = |extra: Json| {
        send(json!({ "done": extra }));
    };
    let named = ["ARGS".to_owned(), "ENV".to_owned()];
    let program = match toolwing_jq::compile("<filter>", &r.filter, &named, &[]) {
        Ok(p) => p,
        Err(report) => return done(json!({ "compile": report })),
    };
    let from = match r.from.as_str() {
        "yaml" => Format::Yaml,
        "toml" => Format::Toml,
        "csv" => Format::Csv,
        "xml" => Format::Xml,
        _ => Format::Json,
    };
    let mut inputs = Vec::new();
    if !r.null_input || r.filter.contains("input") {
        match doc::parse(from, r.input.as_bytes()) {
            Ok(vals) => inputs = vals,
            Err(e) => {
                let at = e
                    .offset
                    .map(|o| format!(" (at byte {o})"))
                    .unwrap_or_default();
                return done(json!({ "input": format!("{}{at}", e.message) }));
            }
        }
    }
    if r.slurp {
        inputs = vec![inputs.into_iter().collect()];
    }
    let to = if r.to == "yaml" {
        Format::Yaml
    } else {
        Format::Json
    };
    // YAML is always written in blocks, and without `---` and `...`.
    let yaml = matches!(to, Format::Yaml);
    let style = Style {
        indent: (!r.compact || yaml).then(|| "  ".to_owned()),
        ..Style::default()
    };
    let writer = Writer {
        format: if r.raw && matches!(to, Format::Json) {
            Format::Raw
        } else {
            to
        },
        pp: Pp {
            indent: style.indent.clone(),
            sort_keys: r.sort_keys,
            styles: Default::default(),
            sep_space: !r.compact || yaml,
        },
        join: yaml,
    };
    let runner = Runner {
        null_input: r.null_input,
        color_err: false,
        writer,
    };
    let args = Val::obj(
        [
            (
                Val::from("positional".to_owned()),
                Val::Arr(Default::default()),
            ),
            (Val::from("named".to_owned()), Val::obj(Default::default())),
        ]
        .into_iter()
        .collect(),
    );
    let env = Val::obj(
        std::env::vars()
            .map(|(k, v)| (Val::from(k), Val::from(v)))
            .collect(),
    );
    let mut values = vec![args, env];
    values.extend(program.imported.iter().cloned());
    let mut written = 0;
    let mut truncated = false;
    let mut listening = true;
    let mut errors = 0;
    let ran = toolwing_jq::run(
        &runner,
        &program,
        Vars::new(values),
        inputs.into_iter().map(Ok),
        || Val::Null,
        |v| {
            if truncated || !listening {
                return Ok(());
            }
            let mut buf = Vec::new();
            write(&mut buf, &runner.writer, &v)?;
            written += buf.len();
            if written > OUTPUT_LIMIT {
                truncated = true;
                return Ok(());
            }
            listening = send(json!({ "output": String::from_utf8_lossy(&buf) }));
            Ok(())
        },
        |e| {
            errors += 1;
            let message = match &e {
                Val::TStr(s) | Val::BStr(s) => String::from_utf8_lossy(s).into_owned(),
                e => format!("{e} (not a string)"),
            };
            send(json!({ "error": message }));
        },
    );
    let extra = match ran {
        Ok(_) => json!({ "truncated": truncated }),
        Err(Stop::Halt(code)) => json!({ "halt": code, "truncated": truncated }),
        Err(Stop::Input(e)) => json!({ "input": e }),
        Err(Stop::Io(e)) => json!({ "input": e.to_string() }),
    };
    done(extra);
}

// The app's side.

struct Process {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// What the worker wrote to stderr (`debug` and `stderr`, and why it
    /// ended), the last 64 KB.
    stderr: Arc<Mutex<Vec<u8>>>,
}

/// A worker kept warm, and the run in it.
pub struct Worker {
    exe: PathBuf,
    process: Mutex<Option<Process>>,
    /// The running request's process, to kill it from elsewhere.
    running: Mutex<Option<Arc<Mutex<Child>>>>,
    /// Bumped by [`Worker::cancel`]: a run that sees it changed was cancelled.
    cancels: AtomicU64,
}

impl Worker {
    /// A worker that runs `exe --jq-worker`, started now so the first run
    /// does not wait for it.
    pub fn new(exe: PathBuf) -> Worker {
        let worker = Worker {
            exe,
            process: Mutex::new(None),
            running: Mutex::new(None),
            cancels: AtomicU64::new(0),
        };
        if let Ok(p) = worker.spawn() {
            *lock(&worker.process) = Some(p);
        }
        worker
    }

    /// The app's own binary.
    pub fn current_exe() -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("toolwing"))
    }

    fn spawn(&self) -> io::Result<Process> {
        let mut child = Command::new(&self.exe)
            .arg(WORKER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let missing = || io::Error::other("the worker has no pipes");
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(missing)?);
        let mut stderr_pipe = child.stderr.take().ok_or_else(missing)?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let sink = stderr.clone();
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n @ 1..) = stderr_pipe.read(&mut buf) {
                let mut kept = lock(&sink);
                kept.extend_from_slice(&buf[..n]);
                let over = kept.len().saturating_sub(64 << 10);
                kept.drain(..over);
            }
        });
        Ok(Process {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
            stderr,
        })
    }

    /// Stops the running request, if there is one.
    pub fn cancel(&self) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        if let Some(child) = lock(&self.running).take() {
            let _ = lock(&child).kill();
        }
    }

    /// Runs `request`, blocking until it ends, is stopped after
    /// [`TIME_LIMIT`] or is cancelled. A worker that ended is started again
    /// for the next run.
    pub fn run(&self, request: &Request) -> Reply {
        let started = Instant::now();
        let cancels = self.cancels.load(Ordering::SeqCst);
        let mut slot = lock(&self.process);
        let mut process = match slot.take() {
            Some(p) => p,
            None => match self.spawn() {
                Ok(p) => p,
                Err(e) => {
                    let failure = Failure::Crashed(format!("cannot start the jq worker: {e}"));
                    return Reply {
                        failure: Some(failure),
                        ..Reply::default()
                    };
                }
            },
        };
        // A cancel that came while waiting for the process wins.
        if self.cancels.load(Ordering::SeqCst) != cancels {
            *slot = Some(process);
            return Reply {
                failure: Some(Failure::Cancelled),
                ..Reply::default()
            };
        }
        *lock(&self.running) = Some(process.child.clone());
        let finished = Arc::new(AtomicBool::new(false));
        let timed_out = Arc::new(AtomicBool::new(false));
        {
            let (child, finished, timed_out) =
                (process.child.clone(), finished.clone(), timed_out.clone());
            std::thread::spawn(move || {
                let deadline = Instant::now() + TIME_LIMIT;
                while Instant::now() < deadline {
                    if finished.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                if !finished.load(Ordering::SeqCst) {
                    timed_out.store(true, Ordering::SeqCst);
                    let _ = lock(&child).kill();
                }
            });
        }
        let mut reply = Reply::default();
        let ended = send(&mut process.stdin, &request.to_json()).and_then(|()| {
            loop {
                match receive(&mut process.stdout)? {
                    None => break Ok(false),
                    Some(m) => {
                        if let Some(out) = m["output"].as_str() {
                            reply.outputs.push(out.to_owned());
                        } else if let Some(e) = m["error"].as_str() {
                            reply.errors.push(e.to_owned());
                        } else if let Some(done) = m.get("done") {
                            reply.truncated = done["truncated"].as_bool().unwrap_or(false);
                            reply.failure = if let Some(c) = done["compile"].as_str() {
                                Some(Failure::Compile(c.trim_end().to_owned()))
                            } else if let Some(i) = done["input"].as_str() {
                                Some(Failure::Input(format!("The input does not parse: {i}")))
                            } else {
                                done["halt"].as_i64().map(|c| Failure::Halt(c as i32))
                            };
                            break Ok(true);
                        }
                    }
                }
            }
        });
        finished.store(true, Ordering::SeqCst);
        *lock(&self.running) = None;
        reply.elapsed = started.elapsed();
        if matches!(ended, Ok(true)) {
            *slot = Some(process);
            return reply;
        }
        // The worker ended: say why, and leave the slot empty so the next
        // run starts another.
        let status = lock(&process.child).wait().ok();
        let stderr = String::from_utf8_lossy(&lock(&process.stderr)).into_owned();
        reply.failure = Some(if timed_out.load(Ordering::SeqCst) {
            Failure::TimedOut
        } else if self.cancels.load(Ordering::SeqCst) != cancels {
            Failure::Cancelled
        } else if stderr.contains("overflowed its stack") {
            Failure::StackOverflow
        } else if stderr.contains("memory allocation") || stderr.contains("capacity overflow") {
            Failure::OutOfMemory
        } else {
            let how = status.map_or_else(|| "for no known reason".to_owned(), |s| s.to_string());
            Failure::Crashed(format!("The jq worker ended ({how})"))
        });
        reply
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(p) = lock(&self.process).take() {
            let _ = lock(&p.child).kill();
        }
    }
}

/// A lock, even one a panicking thread left: what it holds stays usable.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
