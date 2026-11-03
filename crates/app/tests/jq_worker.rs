//! The jq worker (jq_worker.rs, architecture.md), run as the app runs it:
//! the app's binary started with `--jq-worker`. Each way a run can end, and
//! a worker that ended is started again.

use std::sync::Arc;
use std::time::Duration;

use toolwing::jq_worker::{Failure, Reply, Request, TIME_LIMIT, Worker};

fn worker() -> Worker {
    Worker::new(env!("CARGO_BIN_EXE_toolwing").into())
}

fn request(filter: &str, input: &str) -> Request {
    Request {
        filter: filter.into(),
        input: input.into(),
        compact: true,
        ..Request::default()
    }
}

fn outputs(reply: &Reply) -> String {
    reply.outputs.concat()
}

#[test]
fn runs_as_tw_jq_does() {
    let w = worker();
    let r = w.run(&request(
        ".items[] | select(.ok) | .name",
        r#"{"items":[{"name":"a","ok":true},{"name":"b"}]}"#,
    ));
    assert_eq!((outputs(&r), r.failure), ("\"a\"\n".into(), None));
    let raw = Request {
        raw: true,
        ..request(".[]", r#"["x", 1]"#)
    };
    assert_eq!(outputs(&w.run(&raw)), "x\n1\n");
    let pretty = Request {
        compact: false,
        sort_keys: true,
        ..request(".", r#"{"b":1,"a":2.0}"#)
    };
    assert_eq!(
        outputs(&w.run(&pretty)),
        "{\n  \"a\": 2.0,\n  \"b\": 1\n}\n"
    );
    let yaml = Request {
        from: "yaml".into(),
        to: "yaml".into(),
        ..request(".a", "a: [1, 2]\n")
    };
    assert_eq!(outputs(&w.run(&yaml)), "- 1\n- 2\n");
    let slurp = Request {
        slurp: true,
        ..request("length", "1 2 3")
    };
    assert_eq!(outputs(&w.run(&slurp)), "3\n");
}

#[test]
fn says_what_went_wrong() {
    let w = worker();
    let r = w.run(&request(".a |", "{}"));
    assert!(matches!(r.failure, Some(Failure::Compile(_))), "{r:?}");
    let r = w.run(&request(".", "{\"a\":"));
    assert!(matches!(r.failure, Some(Failure::Input(_))), "{r:?}");
    // A runtime error on one input, and the next goes on, as in jq.
    let r = w.run(&request(".a", r#"{"a":1} 3 {"a":2}"#));
    assert_eq!(
        (outputs(&r), r.errors.len(), r.failure),
        ("1\n2\n".into(), 1, None)
    );
    let r = w.run(&request("\"bye\" | halt_error(3)", "null"));
    assert_eq!(r.failure, Some(Failure::Halt(3)));
}

#[test]
fn stops_what_does_not_end() {
    let w = worker();
    let r = w.run(&request("last(range(1e15))", "null"));
    assert_eq!(r.failure, Some(Failure::TimedOut));
    assert!(
        r.elapsed >= TIME_LIMIT && r.elapsed < TIME_LIMIT * 2,
        "{:?}",
        r.elapsed
    );
    // And the next run has a worker again.
    assert_eq!(outputs(&w.run(&request("1 + 1", "null"))), "2\n");
}

#[test]
fn survives_the_stack_and_memory() {
    let w = worker();
    let r = w.run(&request("def f: [f]; f", "null"));
    assert_eq!(r.failure, Some(Failure::StackOverflow), "{r:?}");
    let r = w.run(&request("[range(1e10)] | length", "null"));
    assert!(
        matches!(r.failure, Some(Failure::OutOfMemory | Failure::TimedOut)),
        "{r:?}"
    );
    assert_eq!(outputs(&w.run(&request("[1, 2] | add", "null"))), "3\n");
}

#[test]
fn a_newer_run_stops_the_running_one() {
    let w = Arc::new(worker());
    let running = {
        let w = w.clone();
        std::thread::spawn(move || w.run(&request("last(range(1e15))", "null")))
    };
    std::thread::sleep(Duration::from_millis(300));
    w.cancel();
    let r = running.join().unwrap_or_default();
    assert_eq!(r.failure, Some(Failure::Cancelled));
    assert!(r.elapsed < TIME_LIMIT, "{:?}", r.elapsed);
    assert_eq!(outputs(&w.run(&request("\"next\"", "null"))), "\"next\"\n");
}
