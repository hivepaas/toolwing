//! Draws each page with no display, as a person would use it, checks what it
//! shows, and saves a picture of it to `target/shots/`.
//!
//! `cargo test -p toolwing --no-default-features --features wayland,headless --test screen`

#![cfg(all(feature = "headless", target_os = "linux"))]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Entity, HeadlessAppContext, WindowHandle, px, size,
};
use gpui_component::Root;
use toolwing::diff_page::DiffPage;
use toolwing::hashes::HashesPage;
use toolwing::jq_page::JqPage;
use toolwing::jq_worker::Request;
use toolwing::page::ToolPage;
use toolwing::workspace::{Page, Workspace};
use toolwing_core::Value;
use toolwing_tools::hash::Algorithm;

fn save(cx: &mut HeadlessAppContext, window: AnyWindowHandle, name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&dir).expect("cannot make target/shots");
    let image = cx
        .capture_screenshot(window)
        .expect("cannot draw the window");
    image
        .save(dir.join(format!("{name}.png")))
        .expect("cannot save the picture");
}

/// Lets the debounce and the background run finish. The headless context runs
/// on a test dispatcher, whose clock moves only when told.
fn settle(cx: &mut HeadlessAppContext, settled: impl Fn(&App) -> bool) {
    for _ in 0..100 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if cx.update(|cx| settled(cx)) {
            return;
        }
    }
    panic!("the run did not finish");
}

struct Ui {
    cx: HeadlessAppContext,
    window: WindowHandle<Root>,
    workspace: Entity<Workspace>,
}

impl Ui {
    fn open() -> Ui {
        let text = Arc::new(gpui_wgpu::CosmicTextSystem::new(toolwing::FONT));
        let mut cx =
            HeadlessAppContext::with_platform(text, Arc::new(gpui_kit_assets::Assets), || {
                gpui_platform::current_headless_renderer()
            });
        cx.update(toolwing::init);
        // The jq worker is the app's binary, not this test's (jq_worker.rs);
        // started before the window, which opens on the jq page.
        cx.update(|cx| toolwing::jq_page::start_worker(env!("CARGO_BIN_EXE_toolwing").into(), cx));
        let workspace: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::default();
        let slot = workspace.clone();
        let window = cx
            .open_window(size(px(1200.), px(760.)), move |window, cx| {
                let view = cx.new(|cx| Workspace::new(window, cx));
                *slot.borrow_mut() = Some(view.clone());
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("cannot open a window");
        let workspace = workspace.borrow().clone().expect("the workspace");
        Ui {
            cx,
            window,
            workspace,
        }
    }

    fn go(&mut self, title: &str) -> Page {
        let workspace = self.workspace.clone();
        self.window
            .update(&mut self.cx, |_, window, cx| {
                workspace.update(cx, |w, cx| w.select_title(title, window, cx))
            })
            .expect("window");
        self.cx.update(|cx| self.workspace.read(cx).page().clone())
    }

    fn tool(&mut self, title: &str) -> Entity<ToolPage> {
        match self.go(title) {
            Page::Tool(p) => p,
            _ => panic!("{title} is not a tool page"),
        }
    }

    fn hashes(&mut self) -> Entity<HashesPage> {
        match self.go("Hashes") {
            Page::Hashes(p) => p,
            _ => panic!("Hashes is not the hashes page"),
        }
    }

    fn diff(&mut self) -> Entity<DiffPage> {
        match self.go("Diff") {
            Page::Diff(p) => p,
            _ => panic!("Diff is not the diff page"),
        }
    }

    fn jq(&mut self) -> Entity<JqPage> {
        match self.go("jq") {
            Page::Jq(p) => p,
            _ => panic!("jq is not the jq page"),
        }
    }

    fn on<V: 'static>(
        &mut self,
        page: &Entity<V>,
        f: impl FnOnce(&mut V, &mut gpui::Window, &mut gpui::Context<V>),
    ) {
        self.window
            .update(&mut self.cx, |_, window, cx| {
                page.update(cx, |p, cx| f(p, window, cx))
            })
            .expect("window");
    }

    fn save(&mut self, name: &str) {
        let any: AnyWindowHandle = self.window.into();
        save(&mut self.cx, any, name);
    }
}

#[test]
fn every_page_draws_and_runs() {
    let mut ui = Ui::open();

    // Base64: encode, then decode with an error.
    let p = ui.tool("Base64");
    ui.on(&p, |p, window, cx| {
        p.set_input("hello, toolwing ☕", window, cx)
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "aGVsbG8sIHRvb2x3aW5nIOKYlQ==\n"
    );
    ui.save("1-base64-encode");
    ui.on(&p, |p, window, cx| {
        p.select_action(1, window, cx);
        p.set_input("aGVsbG8sIHRvb2x3aW5nIOKYlQ== *", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert!(ui.cx.update(|cx| p.read(cx).error().is_some()));
    ui.save("2-base64-decode-error");

    // URL encoding.
    let p = ui.tool("URL");
    ui.on(&p, |p, window, cx| {
        p.set_value("form", Value::Bool(true), window, cx);
        p.set_input("q=tiếng việt & more", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "q%3Dti%E1%BA%BFng+vi%E1%BB%87t+%26+more"
    );
    ui.save("3-url-encode");

    // UUID: no input; v7, five of them.
    let p = ui.tool("UUID");
    ui.on(&p, |p, window, cx| {
        p.set_value("version", Value::Text("v7".into()), window, cx);
        p.set_value("count", Value::Int(5), window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    let ids = ui.cx.update(|cx| p.read(cx).output(cx));
    assert_eq!(ids.lines().count(), 5);
    assert!(
        ids.lines().all(|id| id.as_bytes().get(14) == Some(&b'7')),
        "{ids}"
    );
    ui.save("4-uuid");

    // Hashes: text, every algorithm, a pasted SHA-256 that matches.
    let h = ui.hashes();
    ui.on(&h, |h, window, cx| {
        h.set_text("hello world", window, cx);
        h.set_compare(
            "B94D27B9934D3E08A52E52D7DA7DABFAC484EFE37A5380EE9088F7ACE2EFCDE9  a.txt",
            window,
            cx,
        );
    });
    let q = h.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    let shown = ui.cx.update(|cx| h.read(cx).shown());
    assert_eq!(shown.len(), 6);
    assert_eq!(
        shown[2],
        (
            "SHA-256".into(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9".into()
        )
    );
    assert_eq!(
        ui.cx.update(|cx| h.read(cx).matched(cx)),
        Some(Algorithm::Sha256)
    );
    ui.save("5-hashes-text");

    // Hashes: a file, read in one pass.
    let dir = std::env::temp_dir().join(format!("toolwing-screen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("release.tar");
    std::fs::write(&file, vec![7u8; 3 * 1024 * 1024]).expect("temp file");
    ui.on(&h, |h, window, cx| {
        h.set_compare("", window, cx);
        h.set_file(file.clone(), window, cx);
    });
    let q = h.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert!(ui.cx.update(|cx| h.read(cx).error().is_none()));
    let shown = ui.cx.update(|cx| h.read(cx).shown());
    assert_eq!(shown.len(), 6);
    ui.save("6-hashes-file");
    let _ = std::fs::remove_dir_all(&dir);

    // Time: a Unix timestamp in every form, in two more zones.
    let p = ui.tool("Time");
    ui.on(&p, |p, window, cx| {
        p.set_value("tz", Value::Text("Asia/Ho_Chi_Minh".into()), window, cx);
        p.set_value(
            "zones",
            Value::Text("UTC, America/New_York".into()),
            window,
            cx,
        );
        p.set_value(
            "relative_to",
            Value::Text("2026-11-03T06:26:00Z".into()),
            window,
            cx,
        );
        p.set_input("1793692800", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    let field = |ui: &mut Ui, p: &Entity<ToolPage>, key: &str| {
        ui.cx.update(|cx| {
            p.read(cx)
                .fields()
                .and_then(|f| f.iter().find(|f| f.key == key).map(|f| f.value.clone()))
        })
    };
    assert_eq!(
        field(&mut ui, &p, "iso").as_deref(),
        Some("2026-11-03T08:00:00Z")
    );
    assert_eq!(
        field(&mut ui, &p, "relative").as_deref(),
        Some("in 1 hour 34 minutes")
    );
    assert_eq!(
        field(&mut ui, &p, "America/New_York").as_deref(),
        Some("Tue 2026-11-03 03:00:00 -05:00")
    );
    ui.save("7-time");

    // Case: every case of a name.
    let p = ui.tool("Case");
    ui.on(&p, |p, window, cx| {
        p.set_input("user account ID", window, cx)
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "camel").as_deref(),
        Some("userAccountId")
    );
    assert_eq!(
        field(&mut ui, &p, "constant").as_deref(),
        Some("USER_ACCOUNT_ID")
    );
    ui.save("8-case");

    // Number base: 255, grouped, as an 8-bit integer too.
    let p = ui.tool("Number base");
    ui.on(&p, |p, window, cx| {
        p.set_value("group", Value::Int(4), window, cx);
        p.set_value("bits", Value::Text("8".into()), window, cx);
        p.set_input("0xff", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(field(&mut ui, &p, "binary").as_deref(), Some("1111 1111"));
    assert_eq!(field(&mut ui, &p, "signed").as_deref(), Some("-1"));
    ui.save("9-number-base");

    // JWT: decode the RFC 7515 example, then verify it with its key.
    let token = "eyJ0eXAiOiJKV1QiLA0KICJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let p = ui.tool("JWT");
    ui.on(&p, |p, window, cx| {
        p.set_value("tz", Value::Text("Asia/Ho_Chi_Minh".into()), window, cx);
        p.set_value(
            "relative_to",
            Value::Text("2011-03-22T17:43:00Z".into()),
            window,
            cx,
        );
        p.set_input(token, window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(field(&mut ui, &p, "payload.iss").as_deref(), Some("joe"));
    ui.save("10-jwt-decode");
    ui.on(&p, |p, window, cx| {
        p.select_action(1, window, cx);
        p.set_value(
            "key",
            Value::Text("AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow".into()),
            window,
            cx,
        );
        p.set_value("base64_secret", Value::Bool(true), window, cx);
        p.set_value("tz", Value::Text("Asia/Ho_Chi_Minh".into()), window, cx);
        p.set_value("relative_to", Value::Text("2011-03-22T17:43:00Z".into()), window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(field(&mut ui, &p, "result").as_deref(), Some("Valid"));
    ui.save("11-jwt-verify");
    // A day later: expired.
    ui.on(&p, |p, window, cx| {
        p.set_value(
            "relative_to",
            Value::Text("2011-03-23T18:43:00Z".into()),
            window,
            cx,
        );
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    let error = ui
        .cx
        .update(|cx| p.read(cx).error().map(|e| e.message.clone()));
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.starts_with("expired 1 day ago")),
        "{error:?}"
    );
    ui.save("12-jwt-expired");

    // Regex: dates and their named groups in a log.
    let p = ui.tool("Regex");
    ui.on(&p, |p, window, cx| {
        p.set_value(
            "pattern",
            Value::Text(r"(?<year>\d{4})-(?<month>\d{2})-(\d{2})".into()),
            window,
            cx,
        );
        p.set_input(
            "INFO  2026-11-03 deploy started\nWARN  disk at 91%\nERROR 2026-11-04 rollback\n",
            window,
            cx,
        );
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(field(&mut ui, &p, "2.year").as_deref(), Some("2026"));
    assert_eq!(field(&mut ui, &p, "2.3").as_deref(), Some("04"));
    ui.save("13-regex");

    // Diff: a config before and after.
    let d = ui.diff();
    ui.on(&d, |d, window, cx| {
        d.set_texts(
            "{\n  \"service\": \"billing\",\n  \"replicas\": 6,\n  \"log\": \"warn\",\n  \"region\": \"ap-southeast-1\"\n}\n",
            "{\n  \"service\": \"billing\",\n  \"replicas\": 2,\n  \"log\": \"debug\",\n  \"debug_sql\": true,\n  \"region\": \"ap-southeast-1\"\n}\n",
            window,
            cx,
        )
    });
    let q = d.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(ui.cx.update(|cx| d.read(cx).counts()), (3, 2));
    assert_eq!(
        ui.cx
            .update(|cx| d.read(cx).lines().first().cloned())
            .as_deref(),
        Some("@@ -1,6 +1,7 @@")
    );
    ui.save("14-diff");

    // Cron: working hours, from Tuesday 13:26 in Ho Chi Minh City.
    let p = ui.tool("Cron");
    ui.on(&p, |p, window, cx| {
        p.set_value("tz", Value::Text("Asia/Ho_Chi_Minh".into()), window, cx);
        p.set_value(
            "relative_to",
            Value::Text("2026-11-03T06:26:00Z".into()),
            window,
            cx,
        );
        p.set_input("*/15 9-17 * * 1-5", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "description").as_deref(),
        Some("Every 15 minutes, from 09:00 through 17:45, Monday through Friday")
    );
    assert_eq!(
        field(&mut ui, &p, "next_1").as_deref(),
        Some("Tue 2026-11-03 13:30:00 +07:00 · in 4 minutes")
    );
    ui.save("15-cron");

    // jq: the pods that are not running, as the sketch has it.
    let pods = r#"{"kind": "List", "items": [
  {"metadata": {"name": "web-7d9f"}, "status": {"phase": "Running"}},
  {"metadata": {"name": "api-55c1"}, "status": {"phase": "CrashLoopBackOff"}},
  {"metadata": {"name": "job-x"}, "status": {"phase": "Succeeded"}}
]}"#;
    let j = ui.jq();
    ui.on(&j, |j, window, cx| {
        j.set_options(
            Request {
                raw: true,
                from: "json".into(),
                to: "json".into(),
                ..Request::default()
            },
            window,
            cx,
        );
        j.set_texts(
            r#".items[] | select(.status.phase != "Running") | .metadata.name"#,
            pods,
            window,
            cx,
        );
    });
    let q = j.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(ui.cx.update(|cx| j.read(cx).output(cx)), "api-55c1\njob-x");
    ui.save("16-jq");

    // A filter half typed: the error, and the last output dimmed under it.
    ui.on(&j, |j, window, cx| {
        j.set_texts(".items[] | select(", pods, window, cx)
    });
    let q = j.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert!(ui.cx.update(|cx| j.read(cx).reply().failure.is_some()));
    assert_eq!(ui.cx.update(|cx| j.read(cx).output(cx)), "api-55c1\njob-x");
    ui.save("17-jq-error");

    // JSON: formatted as jq . writes it, and where it stops being JSON.
    let p = ui.tool("JSON");
    ui.on(&p, |p, window, cx| {
        p.set_input(
            r#"{"service": "billing", "replicas": 3, "ports": [8080, 9090], "ratio": 1.50}"#,
            window,
            cx,
        )
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "{\n  \"service\": \"billing\",\n  \"replicas\": 3,\n  \"ports\": [\n    8080,\n    9090\n  ],\n  \"ratio\": 1.50\n}"
    );
    ui.save("18-json");
    ui.on(&p, |p, window, cx| {
        p.set_input(
            "{\"service\": \"billing\",\n \"replicas\": 3,\n}",
            window,
            cx,
        )
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    let span = ui.cx.update(|cx| p.read(cx).error().and_then(|e| e.span));
    assert_eq!(span.map(|s| s.line), Some(3));
    ui.save("19-json-error");

    // Convert: the sketch's ConfigMap, YAML to JSON.
    let p = ui.tool("Convert");
    ui.on(&p, |p, window, cx| {
        p.set_input(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: billing\ndata:\n  LOG_LEVEL: info\n  ENABLE_V2: \"no\"\n  VERSION: \"1.10\"\n",
            window,
            cx,
        )
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    let json = ui.cx.update(|cx| p.read(cx).output(cx));
    assert!(json.contains("\"VERSION\": \"1.10\""), "{json}");
    ui.save("20-convert");

    // URL parse: the sketch's URL, its query decoded.
    let p = ui.tool("URL");
    ui.on(&p, |p, window, cx| {
        p.select_action(2, window, cx);
        p.set_input(
            "https://admin@api.example.com:8443/v1/search?q=ti%E1%BA%BFng%20vi%E1%BB%87t&page=2&sort=-created&tag=a&tag=b#results",
            window,
            cx,
        );
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "host").as_deref(),
        Some("api.example.com")
    );
    assert_eq!(field(&mut ui, &p, "param.0").as_deref(), Some("tiếng việt"));
    ui.save("21-url-parse");

    // Hex and Base32.
    let p = ui.tool("Hex");
    ui.on(&p, |p, window, cx| {
        p.set_input("hello, toolwing ☕", window, cx)
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "68656c6c6f2c20746f6f6c77696e6720e29895"
    );
    ui.save("22-hex");
    let p = ui.tool("Base32");
    ui.on(&p, |p, window, cx| p.set_input("foobar", window, cx));
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "MZXW6YTBOI======\n"
    );
    ui.save("23-base32");

    // UUID inspect: the sketch's v7 UUID.
    let p = ui.tool("UUID");
    ui.on(&p, |p, window, cx| {
        p.select_action(1, window, cx);
        p.set_input("01a19fdf-ca6b-7b3a-9f21-3c5d2e1a0b44", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "version").as_deref(),
        Some("7 · time-ordered")
    );
    assert_eq!(
        field(&mut ui, &p, "created").as_deref(),
        Some("2026-11-03T05:26:46.123Z")
    );
    ui.save("24-uuid-inspect");

    // Random: five passwords of 20, look-alikes left out, as the sketch.
    let p = ui.tool("Random");
    ui.on(&p, |p, window, cx| {
        p.set_value("count", Value::Int(5), window, cx);
        p.set_value("no_lookalike", Value::Bool(true), window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    let passwords = ui.cx.update(|cx| p.read(cx).output(cx));
    assert_eq!(passwords.lines().count(), 5);
    let clear = |p: &str| p.chars().count() == 20 && !p.contains(['I', 'l', '1', 'O', '0']);
    assert!(passwords.lines().all(clear), "{passwords}");
    ui.save("25-random");

    // HMAC: RFC 4231's case 2, the key masked.
    let p = ui.tool("HMAC");
    ui.on(&p, |p, window, cx| {
        p.set_value("key", Value::Text("Jefe".into()), window, cx);
        p.set_input("what do ya want for nothing?", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    ui.save("26-hmac");

    // ULID inspect: the spec's example.
    let p = ui.tool("ULID");
    ui.on(&p, |p, window, cx| {
        p.select_action(1, window, cx);
        p.set_input("01ARZ3NDEKTSV4RRFFQ69G5FAV", window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "uuid").as_deref(),
        Some("01563e3a-b5d3-d676-4c61-efb99302bd5b")
    );
    ui.save("27-ulid");

    // Certificate: the test chain, read at a fixed time.
    let chain = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tools/testdata/cert/chain.pem"),
    )
    .expect("the test chain");
    let p = ui.tool("Certificate");
    ui.on(&p, |p, window, cx| {
        p.set_value("at", Value::Text("2026-11-04T00:00:00Z".into()), window, cx);
        p.set_input(&chain, window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "chain").as_deref(),
        Some("in order, leaf first, ending at a self-signed root")
    );
    assert_eq!(
        field(&mut ui, &p, "1.status").as_deref(),
        Some("valid, expires in 3 months")
    );
    ui.save("28-cert");

    // bcrypt: htpasswd's hash of "secret" checked. A password hash waits
    // for Ctrl+Enter, the password masked.
    let p = ui.tool("bcrypt");
    ui.on(&p, |p, window, cx| {
        p.select_action(1, window, cx);
        p.set_value(
            "hash",
            Value::Text("$2y$05$CHEzABv8He3HA.aYEGJj/eZYsINOZh41wgAP3kRVOa1znFxhNCtWC".into()),
            window,
            cx,
        );
        p.set_input("secret", window, cx);
    });
    assert!(ui.cx.update(|cx| p.read(cx).waiting()));
    ui.on(&p, |p, window, cx| p.run_now(window, cx));
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(field(&mut ui, &p, "matches").as_deref(), Some("yes"));
    assert_eq!(field(&mut ui, &p, "cost").as_deref(), Some("5"));
    ui.save("29-bcrypt");

    // Argon2: the reference command's own example, as Argon2id.
    let p = ui.tool("Argon2");
    ui.on(&p, |p, window, cx| {
        p.set_value("salt", Value::Text("somesalt".into()), window, cx);
        p.set_input("password", window, cx);
        p.run_now(window, cx);
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "encoded").as_deref(),
        Some(
            "$argon2id$v=19$m=4096,t=3,p=1$c29tZXNhbHQ$qLml5cbqFAO6YxVHhrSBHP0UWdxrIxkNcM8aMX3blzU"
        )
    );
    ui.save("30-argon2");

    // IP network: a /26 worked out, as ipcalc --all-info does.
    let p = ui.tool("IP network");
    ui.on(&p, |p, window, cx| {
        p.set_input("192.168.1.77/26", window, cx)
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        field(&mut ui, &p, "network").as_deref(),
        Some("192.168.1.64/26")
    );
    assert_eq!(field(&mut ui, &p, "addresses").as_deref(), Some("62"));
    ui.save("31-ipcalc");

    // HTML: references read as a browser reads them, legacy ones too.
    let p = ui.tool("HTML");
    ui.on(&p, |p, window, cx| {
        p.select_action(1, window, cx);
        p.set_input(
            "Caf&eacute; &amp; cr&egrave;me &lt;3 &copy 2026 &#x1F600;",
            window,
            cx,
        );
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "Café & crème <3 © 2026 😀"
    );
    ui.save("32-html");

    // XML: a document indented as xmllint --format indents it.
    let p = ui.tool("XML");
    ui.on(&p, |p, window, cx| {
        p.set_input(
            "<catalog><book id=\"1\"><title>Rust &amp; XML</title><price>29.99</price></book><!-- more --><book id=\"2\"/></catalog>",
            window,
            cx,
        )
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(
        ui.cx.update(|cx| p.read(cx).output(cx)),
        "<?xml version=\"1.0\"?>\n<catalog>\n  <book id=\"1\">\n    <title>Rust &amp; XML</title>\n    <price>29.99</price>\n  </book>\n  <!-- more -->\n  <book id=\"2\"/>\n</catalog>"
    );
    ui.save("33-xml");

    // QR code: a URL drawn as qrencode draws it, as a picture.
    let p = ui.tool("QR code");
    ui.on(&p, |p, window, cx| {
        p.set_input("https://github.com/hivepaas/toolwing", window, cx)
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert!(ui.cx.update(|cx| p.read(cx).error().is_none()));
    ui.save("34-qrencode");

    // Color: a wide-gamut OKLCH color in every form, mapped into sRGB.
    let p = ui.tool("Color");
    ui.on(&p, |p, window, cx| {
        p.set_input("oklch(70% 0.2 150)", window, cx)
    });
    let q = p.clone();
    settle(&mut ui.cx, move |cx| q.read(cx).settled());
    assert_eq!(field(&mut ui, &p, "hex").as_deref(), Some("#00bf54"));
    ui.save("35-color");
}
