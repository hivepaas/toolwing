//! Compatibility cases (docs/design/quality.md): each case under `compat/<group>/`
//! was run against the reference tool by `scripts/compat_record.sh`, and `tw`
//! must give the same stdout, byte for byte, and the same exit code; and the
//! same stderr, when the case has a `stderr` file (the groups whose messages
//! are matched too). A case's `files/` folder, when it has one, is where the
//! command runs.
//!
//! A case with a `known` file is a difference not yet fixed, the file saying
//! why: it must still differ, so that a fix shows up as a failure here until
//! the file is removed.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn tw_matches_the_reference_tools() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat");
    let mut failures = Vec::new();
    let mut count = 0;
    for group in sorted_dirs(&root) {
        let name = group
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for case in sorted_dirs(&group) {
            count += 1;
            let label = format!(
                "{name}/{}",
                case.file_name().unwrap_or_default().to_string_lossy()
            );
            match (check(&name, &case), case.join("known").exists()) {
                (Err(why), false) => failures.push(format!("{label}: {why}")),
                (Ok(()), true) => failures.push(format!(
                    "{label}: same as the reference now; remove its `known`"
                )),
                _ => {}
            }
        }
    }
    assert!(count > 0, "no cases under {}", root.display());
    assert!(
        failures.is_empty(),
        "{} of {count} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn check(group: &str, case: &Path) -> Result<(), String> {
    let read = |file: &str| fs::read(case.join(file)).map_err(|e| format!("{file}: {e}"));
    let stdin_path = case.join("stdin");
    let args: Vec<String> = String::from_utf8_lossy(&read("args")?)
        .lines()
        .map(|a| {
            if a == "@stdin" {
                stdin_path.display().to_string()
            } else {
                a.to_owned()
            }
        })
        .collect();
    let stdin = read("stdin")?;
    let want_stdout = read("stdout")?;
    let want_stderr = fs::read(case.join("stderr")).ok();
    let want_exit: i32 = String::from_utf8_lossy(&read("exit")?)
        .trim()
        .parse()
        .map_err(|e| format!("exit: {e}"))?;

    let files = case.join("files");
    let mut child = Command::new(env!("CARGO_BIN_EXE_tw"))
        .current_dir(if files.is_dir() {
            files
        } else {
            case.to_owned()
        })
        // As the reference ran, in Docker as root: a case may read $HOME, and
        // a Windows runner has none.
        .env("HOME", "/root")
        .arg(group)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if want_stderr.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut input) = child.stdin.take() {
        // A command that fails early may not read its input.
        let _ = input.write_all(&stdin);
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let exit = out.status.code().unwrap_or(-1);
    if exit != want_exit {
        return Err(format!("exit {exit}, the reference {want_exit}"));
    }
    let stdout = unix_wording(out.stdout);
    let (stdout, want_stdout) = if group == "argon2" {
        (untimed(&stdout), untimed(&want_stdout))
    } else {
        (stdout, want_stdout)
    };
    if stdout != want_stdout {
        return Err(format!(
            "stdout differs: {} bytes, the reference {} ({:?}… vs {:?}…)",
            stdout.len(),
            want_stdout.len(),
            String::from_utf8_lossy(&stdout[..stdout.len().min(40)]),
            String::from_utf8_lossy(&want_stdout[..want_stdout.len().min(40)]),
        ));
    }
    if let Some(want) = want_stderr {
        let stderr = unix_wording(out.stderr);
        let stderr = if group == "zstd" {
            zstd_banner(&stderr)
        } else {
            stderr
        };
        if stderr != want {
            return Err(format!(
                "stderr differs:\n{}\nthe reference:\n{}",
                String::from_utf8_lossy(&stderr),
                String::from_utf8_lossy(&want),
            ));
        }
    }
    Ok(())
}

/// `tw zstd -v`'s banner as zstd's: tw names itself there, as with `-V`.
fn zstd_banner(stderr: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(stderr);
    let Some(rest) = text.strip_prefix("*** tw zstd ") else {
        return stderr.to_vec();
    };
    match rest.split_once(" ***\n") {
        Some((_, after)) => {
            format!("*** Zstandard CLI (64-bit) v1.5.7, by Yann Collet ***\n{after}").into_bytes()
        }
        None => stderr.to_vec(),
    }
}

/// The OS's own words for an error, as the reference (on Linux) printed
/// them: tw prints what the system says, as b3sum built for Windows would.
fn unix_wording(stdout: Vec<u8>) -> Vec<u8> {
    if !cfg!(windows) {
        return stdout;
    }
    const WORDS: [(&str, &str); 2] = [
        (
            "The system cannot find the file specified. (os error 2)",
            "No such file or directory (os error 2)",
        ),
        (
            "The system cannot find the path specified. (os error 3)",
            "No such file or directory (os error 2)",
        ),
    ];
    let mut text = String::from_utf8_lossy(&stdout).into_owned();
    if !WORDS.iter().any(|(w, _)| text.contains(w)) {
        return stdout;
    }
    for (windows, unix) in WORDS {
        text = text.replace(windows, unix);
    }
    text.into_bytes()
}

/// Every case's files can be checked out on Windows too: none is named with
/// a character it refuses, or as a device, or as another is but for case.
/// (A name b3sum escapes, with a backslash or a newline, is tested in
/// native.rs instead, on Unix.)
#[test]
fn case_paths_are_portable() {
    fn walk(dir: &Path, bad: &mut Vec<String>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
            let device = ["con", "prn", "aux", "nul"].contains(&stem.as_str())
                || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")));
            let refused = name.chars().any(|c| c < ' ' || "<>:\"|?*\\".contains(c))
                || name.ends_with([' ', '.'])
                || device;
            if refused {
                bad.push(entry.path().display().to_string());
            }
            if entry.path().is_dir() {
                walk(&entry.path(), bad);
            }
        }
        // Windows (and macOS) names ignore case: two would be one file.
        let mut names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_lowercase())
            .collect();
        names.sort();
        for pair in names.windows(2) {
            if pair[0] == pair[1] {
                bad.push(format!(
                    "{} (twice, ignoring case)",
                    dir.join(&pair[0]).display()
                ));
            }
        }
    }
    let mut bad = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat"),
        &mut bad,
    );
    assert!(bad.is_empty(), "names Windows refuses:\n{}", bad.join("\n"));
}

/// argon2's line saying how long it took ("0.006 seconds"), as one that
/// says no time: the only line of its output that differs run to run.
fn untimed(stdout: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(stdout.len());
    for line in stdout.split_inclusive(|&b| b == b'\n') {
        let text = line.strip_suffix(b"\n").unwrap_or(line);
        let timed = text.strip_suffix(b" seconds").is_some_and(|n| {
            let n = String::from_utf8_lossy(n);
            n.trim_start().split_once('.').is_some_and(|(whole, part)| {
                !whole.is_empty()
                    && whole.bytes().all(|b| b.is_ascii_digit())
                    && part.len() == 3
                    && part.bytes().all(|b| b.is_ascii_digit())
            })
        });
        if timed {
            out.extend_from_slice(b"N seconds\n");
        } else {
            out.extend_from_slice(line);
        }
    }
    out
}

fn sorted_dirs(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}
