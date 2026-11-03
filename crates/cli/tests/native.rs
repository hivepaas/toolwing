//! Native commands' conventions (docs/design/cli.md): input, output, exit codes.

use std::io::Write;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn tw(args: &[&str], stdin: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tw"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("tw: {e}"));
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(stdin.as_bytes());
    }
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("tw: {e}"));
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

#[test]
fn the_value_stdin_and_a_file_give_the_same() {
    let dir = std::env::temp_dir().join(format!("tw-native-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let file = dir.join("in.txt");
    std::fs::write(&file, "a b\n").unwrap_or_else(|e| panic!("{e}"));
    let path = file.display().to_string();
    for run in [
        tw(&["url", "encode", "a b"], ""),
        tw(&["url", "encode"], "a b\n"),
        tw(&["url", "encode", "-"], "a b"),
        tw(&["url", "encode", "-i", &path], ""),
    ] {
        assert_eq!((run.stdout.as_str(), run.exit), ("a%20b\n", 0));
    }
}

#[test]
fn json_and_no_newline() {
    assert_eq!(
        tw(&["url", "decode", "--json", "a%22b"], "").stdout,
        "\"a\\\"b\"\n"
    );
    assert_eq!(
        tw(&["url", "encode", "--no-newline", "a b"], "").stdout,
        "a%20b"
    );
    let ids = tw(&["uuid", "--json", "-n", "2"], "").stdout;
    assert!(
        ids.starts_with("[\"") && ids.matches(',').count() == 1,
        "{ids}"
    );
}

#[test]
fn exit_codes_say_what_went_wrong() {
    let bad = tw(&["url", "decode", "ab%g1"], "");
    assert_eq!(bad.exit, 3);
    assert_eq!(
        bad.stderr,
        "tw: url decode: % not followed by two hex digits at line 1, column 3\n"
    );
    assert_eq!(tw(&["url", "nope"], "").exit, 2);
    assert_eq!(tw(&["uuid", "-n", "0"], "").exit, 2);
    let missing = tw(&["url", "encode", "-i", "/nonexistent/toolwing-test"], "");
    assert_eq!(missing.exit, 4);
}

#[test]
fn named_values_line_up_or_come_as_one_json_object() {
    let run = tw(&["base", "255"], "");
    assert_eq!(run.exit, 0);
    assert!(
        run.stdout.starts_with("Binary   11111111\nOctal    377\n"),
        "{}",
        run.stdout
    );
    let run = tw(&["base", "--json", "-g", "4", "0xff"], "");
    assert_eq!(
        run.stdout,
        "{\"binary\":\"1111 1111\",\"octal\":\"377\",\"decimal\":\"255\",\"hex\":\"ff\",\"base36\":\"73\"}\n"
    );
    assert_eq!(
        tw(&["base", "--to", "2", "--bits", "8", "-1"], "").stdout,
        "-1\n"
    );
    assert_eq!(tw(&["base", "12z"], "").exit, 3);
}

#[test]
fn time_case_and_base_one_value_at_a_time() {
    let at = [
        "--relative-to",
        "2026-11-03T06:26:00Z",
        "--tz",
        "Asia/Ho_Chi_Minh",
    ];
    let time = |extra: &[&str]| tw(&[&["time"][..], &at, extra].concat(), "").stdout;
    assert_eq!(
        time(&["--to", "iso", "1793692800"]),
        "2026-11-03T08:00:00Z\n"
    );
    assert_eq!(time(&["--to", "unix", "2026-11-03 15:00"]), "1793692800\n");
    assert_eq!(
        time(&["--to", "relative", "1793692800"]),
        "in 1 hour 34 minutes\n"
    );
    assert_eq!(time(&["--format", "%d/%m/%Y", "in 3 days"]), "06/11/2026\n");
    assert_eq!(tw(&["time", "--tz", "Mars/Olympus", "0"], "").exit, 2);
    assert_eq!(tw(&["time", "soon"], "").exit, 3);
    assert_eq!(
        tw(&["case", "--snake", "userAccountID"], "").stdout,
        "user_account_id\n"
    );
    assert_eq!(
        tw(&["case", "--kebab"], "One Two\nthree four\n").stdout,
        "one-two\nthree-four\n"
    );
    assert_eq!(tw(&["case", "--snake", "--kebab", "x"], "").exit, 2);
    assert_eq!(tw(&["base", "--to", "36", "1295"], "").stdout, "zz\n");
}

#[test]
fn help_is_stable() {
    for args in [
        &["--help"][..],
        &["base64", "--help"],
        &["url", "--help"],
        &["uuid", "--help"],
        &["sha256sum", "--help"],
        &["time", "--help"],
        &["case", "--help"],
        &["base", "--help"],
    ] {
        let run = tw(args, "");
        assert_eq!(run.exit, 0);
        insta::assert_snapshot!(args.join("-").trim_start_matches('-'), run.stdout);
    }
}

#[test]
fn list_names_every_tool() {
    let run = tw(&["list", "--json"], "");
    for id in [
        "base64.encode",
        "base64.decode",
        "url.encode",
        "url.decode",
        "uuid",
    ] {
        assert!(run.stdout.contains(&format!("\"id\":\"{id}\"")), "{id}");
    }
}
