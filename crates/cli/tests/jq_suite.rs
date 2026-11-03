//! jq's own test suite (`jq-suite/`, from jq 1.8.2, MIT: `LICENSE-jq`), run
//! through `tw jq` and judged as jq's runner (`src/jq_test.c`) judges: the
//! outputs compared as values, in order; an error after the expected outputs
//! is fine; a `%%FAIL` case passes when its program does not compile.
//!
//! Not every case passes yet (docs/design/jq.md). Each file's count is a
//! ratchet: fewer passing fails the test, and so do more, until `PASSING`
//! is raised to the new count.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use toolwing_jq::{Val, read};

/// The cases that pass in each file, of how many.
const PASSING: &[(&str, usize, usize)] = &[
    ("jq.test", 471, 550),
    ("man.test", 231, 231),
    ("onig.test", 47, 47),
    ("manonig.test", 19, 19),
    ("base64.test", 10, 10),
    ("uri.test", 20, 20),
    ("optional.test", 2, 2),
];

/// A case that runs longer is stopped and fails.
const TIMEOUT: Duration = Duration::from_secs(10);

struct Case {
    line: usize,
    program: String,
    /// `%%FAIL`: the program must not compile.
    must_fail: bool,
    input: String,
    expected: Vec<String>,
}

/// jq's `skipline`: blank lines and comments separate cases.
fn skip(line: &str) -> bool {
    let s = line.trim_start_matches([' ', '\t']);
    s.is_empty() || s.starts_with('#')
}

fn cases(text: &str) -> Vec<Case> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let (mut i, mut must_fail) = (0, false);
    while i < lines.len() {
        let (line, number) = (lines[i], i + 1);
        i += 1;
        if skip(line) {
            continue;
        }
        if line == "%%FAIL" || line == "%%FAIL IGNORE MSG" {
            must_fail = true;
            continue;
        }
        let mut block = Vec::new();
        while i < lines.len() && !skip(lines[i]) {
            block.push(lines[i].to_owned());
            i += 1;
        }
        let input = if must_fail || block.is_empty() {
            "null".to_owned()
        } else {
            block.remove(0)
        };
        out.push(Case {
            line: number,
            program: line.to_owned(),
            must_fail,
            input,
            expected: if must_fail { Vec::new() } else { block },
        });
        must_fail = false;
    }
    out
}

fn value(text: &str) -> Option<Val> {
    read::json::parse_single(text.trim().as_bytes()).ok()
}

/// Runs one case; why it fails, if it does.
/// `program` is where the case's program is written for `-f`.
fn run(case: &Case, modules: &Path, program: &Path) -> Result<(), String> {
    fs::write(program, &case.program).map_err(|e| e.to_string())?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_tw"))
        .arg("jq")
        .arg("-c")
        .arg("-L")
        .arg(modules)
        .arg("-f")
        .arg(program)
        // As jq's `mantest` runs the manual's examples.
        .env("PAGER", "less")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(case.input.as_bytes());
    }
    let start = Instant::now();
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let compiled = out.status.code() != Some(3);
    if case.must_fail {
        return if compiled {
            Err("compiled".into())
        } else {
            Ok(())
        };
    }
    if !compiled {
        return Err("does not compile".into());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got: Vec<&str> = stdout.split('\n').filter(|l| !l.is_empty()).collect();
    for (n, want) in case.expected.iter().enumerate() {
        let Some(got) = got.get(n) else {
            return Err(format!("{n} outputs, not {}", case.expected.len()));
        };
        let same = match (value(want), value(got)) {
            (Some(w), Some(g)) => w == g,
            _ => false,
        };
        if !same {
            return Err(format!("{} instead of {}", got, want.trim()));
        }
    }
    if got.len() > case.expected.len() {
        return Err(format!("more outputs than {}", case.expected.len()));
    }
    Ok(())
}

#[test]
fn jq_suite_passes_as_far_as_it_did() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/jq-suite");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("jq-suite");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let modules = root.join("modules");
    let verbose = std::env::var_os("TW_JQ_SUITE_VERBOSE").is_some();
    let mut all = Vec::new();
    for &(file, _, total) in PASSING {
        let text = fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let cases = cases(&text);
        assert_eq!(cases.len(), total, "{file}: the count of cases");
        all.extend(cases.into_iter().map(|c| (file, c)));
    }

    // Workers take the next case. They have a big stack: some cases output
    // values nested 10,000 deep, which take more than a test thread's to
    // parse for comparing.
    let next = AtomicUsize::new(0);
    let passed: Vec<&str> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..std::thread::available_parallelism().map_or(4, |n| n.get()))
            .map(|_| {
                std::thread::Builder::new()
                    .stack_size(256 << 20)
                    .spawn_scoped(scope, || {
                        let mut passed = Vec::new();
                        while let Some((file, case)) = all.get(next.fetch_add(1, Ordering::Relaxed))
                        {
                            let program = dir.join(format!("{file}-{}.jq", case.line));
                            match run(case, &modules, &program) {
                                Ok(()) => passed.push(*file),
                                Err(why) if verbose => {
                                    eprintln!("{file}:{}: {}: {why}", case.line, case.program)
                                }
                                Err(_) => {}
                            }
                        }
                        passed
                    })
                    .unwrap_or_else(|e| panic!("{e}"))
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_default())
            .collect()
    });

    let mut wrong = Vec::new();
    for &(file, passing, total) in PASSING {
        let count = passed.iter().filter(|f| **f == file).count();
        eprintln!("{file}: {count} of {total} pass");
        if count != passing {
            wrong.push(format!(
                "{file}: {count} pass, PASSING says {passing} of {total}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{}\n(TW_JQ_SUITE_VERBOSE=1 lists the failing cases)",
        wrong.join("\n")
    );
}
