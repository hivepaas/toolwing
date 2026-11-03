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
    let p = ui.tool("URL encoding");
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
}
