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
    let ids = tw(&["uuid", "generate", "--json", "-n", "2"], "").stdout;
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
    assert_eq!(tw(&["uuid", "generate", "-n", "0"], "").exit, 2);
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

const RFC_TOKEN: &str = "eyJ0eXAiOiJKV1QiLA0KICJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const RFC_KEY: &str =
    "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow";

fn tw_env(args: &[&str], env: &[(&str, &str)]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_tw"))
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("tw: {e}"));
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

#[test]
fn jwt_decode_in_sections_or_as_json() {
    let run = tw(
        &[
            "jwt",
            "decode",
            "--tz",
            "UTC",
            "--relative-to",
            "2011-03-22T17:43:00Z",
            RFC_TOKEN,
        ],
        "",
    );
    assert_eq!(run.exit, 0);
    assert!(run.stdout.starts_with("Header\n  typ"), "{}", run.stdout);
    assert!(run.stdout.contains("\nPayload\n  iss "), "{}", run.stdout);
    assert!(
        run.stdout
            .contains("1300819380 · Tue 2011-03-22 18:43:00 +00:00 · in 1 hour"),
        "{}",
        run.stdout
    );
    let json = tw(&["jwt", "decode", "--json", RFC_TOKEN], "").stdout;
    assert!(
        json.starts_with(
            r#"{"header":{"typ":"JWT","alg":"HS256"},"payload":{"iss":"joe","exp":1300819380,"#
        ),
        "{json}"
    );
}

#[test]
fn jwt_verify_reads_the_key_from_where_it_is_kept() {
    let base = [
        "jwt",
        "verify",
        "--base64-secret",
        "--relative-to",
        "2011-03-22T17:43:00Z",
    ];
    let with = |extra: &[&str], env: &[(&str, &str)]| {
        tw_env(&[&base[..], extra, &[RFC_TOKEN]].concat(), env)
    };
    assert_eq!(with(&["--key", RFC_KEY], &[]).exit, 0);
    assert_eq!(
        with(&["--key-env", "TW_SECRET"], &[("TW_SECRET", RFC_KEY)]).exit,
        0
    );
    let dir = std::env::temp_dir().join(format!("tw-jwt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let file = dir.join("key");
    std::fs::write(&file, format!("{RFC_KEY}\n")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        with(&["--key-file", &file.display().to_string()], &[]).exit,
        0
    );
    // A failed check is exit 1; a key not given or of the wrong kind, 2.
    let wrong = with(&["--key", "AAAA"], &[]);
    assert_eq!(wrong.exit, 1);
    assert!(
        wrong
            .stderr
            .starts_with("tw: jwt verify: the signature does not match"),
        "{}",
        wrong.stderr
    );
    assert_eq!(with(&["--key-env", "TW_NOT_SET"], &[]).exit, 2);
    assert_eq!(with(&["--key", "a", "--key-env", "TW_SECRET"], &[]).exit, 2);
    let expired = tw(
        &[
            "jwt",
            "verify",
            "--base64-secret",
            "--key",
            RFC_KEY,
            RFC_TOKEN,
        ],
        "",
    );
    assert_eq!(expired.exit, 1);
    assert!(expired.stderr.contains("expired"), "{}", expired.stderr);
}

#[test]
fn jwt_encode_then_verify() {
    let token = tw(
        &[
            "jwt",
            "encode",
            "--key",
            "s3cret",
            "--kid",
            "k1",
            r#"{"sub":"42"}"#,
        ],
        "",
    )
    .stdout;
    let token = token.trim();
    assert_eq!(token.split('.').count(), 3);
    assert_eq!(tw(&["jwt", "verify", "--key", "s3cret", token], "").exit, 0);
    assert_eq!(tw(&["jwt", "encode", "--key", "s3cret", "[1]"], "").exit, 3);
}

#[test]
fn regex_matches_exit_as_grep_does() {
    let run = tw(
        &["regex", "match", "-e", r"(?<n>\d+)%", "--json"],
        "disk at 91%\n",
    );
    assert_eq!(run.exit, 0);
    assert_eq!(
        run.stdout,
        "[{\"match\":\"91%\",\"start\":8,\"end\":11,\"line\":1,\"column\":9,\"groups\":{\"n\":\"91\"}}]\n"
    );
    assert_eq!(tw(&["regex", "match", "-e", "nope"], "text").exit, 1);
    assert_eq!(tw(&["regex", "match", "-e", "(unclosed"], "text").exit, 2);
    assert_eq!(tw(&["regex", "match"], "text").exit, 2);
    let replaced = tw(
        &[
            "regex",
            "replace",
            "-e",
            r"(\w+)@(\w+)",
            "--with",
            "$2 at $1",
        ],
        "me@host\n",
    );
    assert_eq!(replaced.stdout, "host at me\n\n");
}

#[test]
fn diff_headers_carry_the_files_times() {
    let dir = std::env::temp_dir().join(format!("tw-diff-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::write(&a, "x\n").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&b, "y\n").unwrap_or_else(|e| panic!("{e}"));
    let (a, b) = (a.display().to_string(), b.display().to_string());
    let run = tw(&["diff", "-u", &a, &b], "");
    assert_eq!(run.exit, 1);
    let first = run.stdout.lines().next().unwrap_or_default().to_owned();
    // "--- NAME\t2026-11-03 15:52:43.986476647 +0700", as GNU writes it.
    let stamp = first
        .strip_prefix(&format!("--- {a}\t"))
        .unwrap_or_else(|| panic!("{first}"));
    let shape: String = stamp
        .chars()
        .map(|c| if c.is_ascii_digit() { '9' } else { c })
        .collect();
    assert!(
        shape == "9999-99-99 99:99:99.999999999 +9999"
            || shape == "9999-99-99 99:99:99.999999999 -9999",
        "{stamp}"
    );
}

#[test]
fn cron_from_a_moment() {
    let run = tw(
        &[
            "cron",
            "--tz",
            "UTC",
            "--relative-to",
            "2026-11-28T00:00:00Z",
            "-n",
            "2",
            "--json",
            "0 0 1 * MON",
        ],
        "",
    );
    assert_eq!(run.exit, 0);
    assert!(
        run.stdout
            .contains("\"next_1\":\"Mon 2026-11-30 00:00:00 +00:00 · in 2 days\""),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("\"next_2\":\"Tue 2026-12-01 00:00:00 +00:00 · in 3 days\""),
        "{}",
        run.stdout
    );
    let bad = tw(&["cron", "*/15 25 * * *"], "");
    assert_eq!(bad.exit, 3);
    assert!(bad.stderr.contains("at line 1, column 6"), "{}", bad.stderr);
}

#[test]
fn help_is_stable() {
    for args in [
        &["--help"][..],
        &["base64", "--help"],
        &["url", "--help"],
        &["uuid", "--help"],
        &["uuid", "generate", "--help"],
        &["random", "password", "--help"],
        &["sha256sum", "--help"],
        &["time", "--help"],
        &["case", "--help"],
        &["base", "--help"],
        &["jwt", "verify", "--help"],
        &["regex", "match", "--help"],
        &["diff", "--help"],
        &["cron", "--help"],
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
        "uuid.generate",
        "uuid.inspect",
        "random.password",
        "hmac",
    ] {
        assert!(run.stdout.contains(&format!("\"id\":\"{id}\"")), "{id}");
    }
}
