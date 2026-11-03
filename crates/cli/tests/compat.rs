//! Compatibility cases (docs/design/quality.md): each case under `compat/<group>/`
//! was run against the reference tool by `scripts/compat_record.sh`, and `tw`
//! must give the same stdout, byte for byte, and the same exit code. A case's
//! `files/` folder, when it has one, is where the command runs.
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
        .arg(group)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
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
    if out.stdout != want_stdout {
        return Err(format!(
            "stdout differs: {} bytes, the reference {} ({:?}… vs {:?}…)",
            out.stdout.len(),
            want_stdout.len(),
            String::from_utf8_lossy(&out.stdout[..out.stdout.len().min(40)]),
            String::from_utf8_lossy(&want_stdout[..want_stdout.len().min(40)]),
        ));
    }
    Ok(())
}

/// Every case's files can be checked out on Windows too: none is named with
/// a character it refuses, or as a device. (A name b3sum escapes, with a
/// backslash or a newline, is tested in native.rs instead, on Unix.)
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
    }
    let mut bad = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat"),
        &mut bad,
    );
    assert!(bad.is_empty(), "names Windows refuses:\n{}", bad.join("\n"));
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
