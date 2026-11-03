//! Values nested as deep as is allowed (10,000 levels, doc.rs) are read and
//! written on a small stack: 2 MB, a spawned thread's default, as the app's
//! background threads may have. Deeper input is refused, not crashed on.

use toolwing_core::{Ctx, Tool, Value, Values};
use toolwing_tools::find;

const SMALL_STACK: usize = 2 << 20;

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    let t = std::thread::Builder::new().stack_size(SMALL_STACK).spawn(f);
    assert!(
        t.and_then(|t| t.join().map_err(|_| std::io::Error::other("panicked")))
            .is_ok()
    );
}

fn run(id: &str, input: Vec<u8>, set: &[(&str, &str)]) -> Result<(), String> {
    let tool: &dyn Tool = find(id).ok_or("no tool")?;
    let mut v = Values::new();
    for (k, x) in set {
        v.set(k, Value::Text((*x).into()));
    }
    tool.run(&Ctx::new(), &input, &v)
        .map(|_| ())
        .map_err(|e| e.message)
}

fn nested(depth: usize, open: &str, close: &str) -> Vec<u8> {
    format!("{}{}", open.repeat(depth), close.repeat(depth)).into_bytes()
}

#[test]
fn json_at_the_limit_is_read_and_written() {
    on_small_stack(|| {
        let deep = nested(10_000, "[", "]");
        assert_eq!(run("json.format", deep.clone(), &[]), Ok(()));
        assert_eq!(run("json.minify", deep.clone(), &[]), Ok(()));
        for to in ["json", "yaml", "toml", "xml"] {
            // TOML and XML refuse a bare array; what matters is no crash.
            let _ = run("convert", deep.clone(), &[("from", "json"), ("to", to)]);
        }
        let objects = nested(10_000, "{\"a\":", "}");
        let objects = [
            &objects[..objects.len() - 10_000],
            b"1",
            &"}".repeat(10_000).into_bytes()[..],
        ]
        .concat();
        for to in ["json", "yaml", "toml", "xml"] {
            let _ = run("convert", objects.clone(), &[("from", "json"), ("to", to)]);
        }
    });
}

#[test]
fn other_formats_at_the_limit() {
    on_small_stack(|| {
        let _ = run("convert", nested(10_000, "[", "]"), &[("from", "yaml")]);
        let _ = run("convert", nested(10_000, "<a>", "</a>"), &[("from", "xml")]);
        let toml = [b"a = ".to_vec(), nested(9_999, "[", "]")].concat();
        let _ = run("convert", toml, &[("from", "toml")]);
    });
}

#[test]
fn deeper_is_refused() {
    on_small_stack(|| {
        for (from, open, close) in [
            ("json", "[", "]"),
            ("yaml", "[", "]"),
            ("xml", "<a>", "</a>"),
        ] {
            let e = run("convert", nested(100_000, open, close), &[("from", from)]);
            assert!(
                e.as_ref().is_err_and(|e| e.contains("deep")),
                "{from}: {e:?}"
            );
        }
    });
}
