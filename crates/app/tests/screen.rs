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

use gpui::{AnyWindowHandle, AppContext as _, Entity, HeadlessAppContext, px, size};
use gpui_component::Root;
use toolwing::page::ToolPage;
use toolwing::workspace::Workspace;
use toolwing_core::Value;
use toolwing_tools::GROUPS;

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
fn settle(cx: &mut HeadlessAppContext, page: &Entity<ToolPage>) {
    for _ in 0..100 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if cx.update(|cx| page.read(cx).settled()) {
            return;
        }
    }
    panic!("the run did not finish");
}

#[test]
fn every_page_draws_and_runs() {
    let text = Arc::new(gpui_wgpu::CosmicTextSystem::new(toolwing::FONT));
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(gpui_kit_assets::Assets), || {
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
    let any: AnyWindowHandle = window.into();
    let page = |cx: &mut HeadlessAppContext| cx.update(|cx| workspace.read(cx).page().clone());

    // Base64: encode, then decode with an error underlined.
    let p = page(&mut cx);
    window
        .update(&mut cx, |_, window, cx| {
            p.update(cx, |p, cx| p.set_input("hello, toolwing ☕", window, cx))
        })
        .expect("window");
    settle(&mut cx, &p);
    assert_eq!(
        cx.update(|cx| p.read(cx).output(cx)),
        "aGVsbG8sIHRvb2x3aW5nIOKYlQ==\n"
    );
    save(&mut cx, any, "1-base64-encode");
    window
        .update(&mut cx, |_, window, cx| {
            p.update(cx, |p, cx| {
                p.select_action(1, window, cx);
                p.set_input("aGVsbG8sIHRvb2x3aW5nIOKYlQ== *", window, cx);
            })
        })
        .expect("window");
    settle(&mut cx, &p);
    assert!(cx.update(|cx| p.read(cx).error().is_some()));
    save(&mut cx, any, "2-base64-decode-error");

    // URL encoding.
    let url = GROUPS.iter().position(|g| g.name == "url").expect("url");
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |w, cx| w.select(url, window, cx))
        })
        .expect("window");
    let p = page(&mut cx);
    window
        .update(&mut cx, |_, window, cx| {
            p.update(cx, |p, cx| {
                p.set_value("form", Value::Bool(true), window, cx);
                p.set_input("q=tiếng việt & more", window, cx);
            })
        })
        .expect("window");
    settle(&mut cx, &p);
    assert_eq!(
        cx.update(|cx| p.read(cx).output(cx)),
        "q%3Dti%E1%BA%BFng+vi%E1%BB%87t+%26+more"
    );
    save(&mut cx, any, "3-url-encode");

    // UUID: no input; v7, five of them.
    let uuid = GROUPS.iter().position(|g| g.name == "uuid").expect("uuid");
    window
        .update(&mut cx, |_, window, cx| {
            workspace.update(cx, |w, cx| w.select(uuid, window, cx))
        })
        .expect("window");
    let p = page(&mut cx);
    window
        .update(&mut cx, |_, window, cx| {
            p.update(cx, |p, cx| {
                p.set_value("version", Value::Text("v7".into()), window, cx);
                p.set_value("count", Value::Int(5), window, cx);
            })
        })
        .expect("window");
    settle(&mut cx, &p);
    let ids = cx.update(|cx| p.read(cx).output(cx));
    assert_eq!(ids.lines().count(), 5);
    assert!(
        ids.lines().all(|id| id.as_bytes().get(14) == Some(&b'7')),
        "{ids}"
    );
    save(&mut cx, any, "4-uuid");
}
