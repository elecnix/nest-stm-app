//! The loop, driven over a real socket pair.
//!
//! These tests are the contract of the SDK's wire side: what the loop does with each message, what
//! it sends back, where it stops, and what it refuses to choke on. The app side runs in a thread
//! and the Nest side is scripted, exactly as `apps/build.sh --host` plus a fake Nest would drive
//! it — so what passes here is what the app does on the device.

use std::io::{BufRead, BufReader, ErrorKind, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use nest_app_sdk::{App, Handler, MAX_ITEMS, MAX_MESSAGE};
use serde_json::{Value, json};

/// A handler that writes down what it was called with, and answers each message with a scene that
/// names it — so the test can check the two directions line up.
#[derive(Default)]
struct Script {
    calls: Vec<String>,
    /// Messages the app sent, parsed.
    sent: Vec<Value>,
    /// The raw lines the app sent, with their framing.
    raw: Vec<String>,
    ticks: usize,
    wheels: usize,
    /// Ticks seen when `open` arrived, to prove a wait does not lose a message.
    ticks_at_open: Option<usize>,
    /// Close from the handler once it has seen `open` and this many ticks.
    close_after_ticks: Option<usize>,
    /// Close from the handler after this many wheel messages.
    close_after_wheels: Option<usize>,
}

impl Handler for Script {
    fn on_open(&mut self, app: &mut App) {
        self.ticks_at_open = Some(self.ticks);
        self.calls.push("open".to_string());
        app.send_scene(&[json!({"k": "text", "c": "#ffffff", "s": "open"})]);
    }

    fn on_wheel(&mut self, app: &mut App, steps: i32) {
        self.calls.push(format!("wheel {steps}"));
        self.wheels += 1;
        app.tick();
        app.event(&json!({"wheel": steps}));
        app.send_scene(&[json!({"k": "text", "c": "#ffffff", "s": format!("wheel{steps}")})]);
        if let Some(n) = self.close_after_wheels
            && self.wheels >= n
        {
            app.close();
        }
    }

    fn on_data(&mut self, app: &mut App, data: &Value) {
        self.calls
            .push(format!("data {}", data["stop"].as_str().unwrap_or("?")));
        app.send_scene(&[json!({"k": "text", "c": "#ffffff", "s": "data"})]);
    }

    fn on_tick(&mut self, app: &mut App) {
        self.ticks += 1;
        if let Some(n) = self.close_after_ticks
            && self.ticks >= n
            && self.ticks_at_open.is_some()
        {
            app.close();
        }
    }

    fn on_stop(&mut self, _app: &mut App) {
        self.calls.push("stop".to_string());
    }
}

/// Everything the app sent before it went quiet. `stream` must carry a read deadline.
fn drain(stream: &UnixStream) -> Vec<String> {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut lines = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => lines.push(line.clone()),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
            Err(e) => panic!("read: {e}"),
        }
    }
    lines
}

/// Run an app on one end of a socket pair against `input` on the other, and hand back what it did.
///
/// The Nest side stops writing once `input` is in, so a loop that never gets a `stop` still ends
/// on the end of the connection.
fn drive(input: &str, timeout: Option<Duration>, mut handler: Script) -> Script {
    let (mut mine, theirs) = UnixStream::pair().expect("socketpair");
    mine.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("read timeout");
    let mut app = App::from_stream(theirs);
    let app_thread = std::thread::spawn(move || {
        let run = app.run_with_timeout(timeout, &mut handler);
        let ignored = app.ignored();
        (handler, run.map_err(|e| e.to_string()), ignored)
    });
    mine.write_all(input.as_bytes()).expect("write the script");
    mine.shutdown(Shutdown::Write).expect("end the script");

    let (runner, run, ignored) = app_thread.join().expect("the app thread");
    assert_eq!(run, Ok(()), "the loop failed");
    assert_eq!(ignored, 0, "nothing the Nest sent here should have been ignored");

    let mut handler = runner;
    for line in drain(&mine) {
        handler
            .sent
            .push(serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line:?} is not JSON: {e}")));
        handler.raw.push(line);
    }
    handler
}

#[test]
fn every_message_is_dispatched_and_answered_in_order() {
    let script = concat!(
        "{\"type\":\"open\"}\n",
        "{\"type\":\"wheel\",\"steps\":2}\n",
        "{\"type\":\"wheel\",\"steps\":-1}\n",
        "{\"type\":\"wheel\",\"steps\":2.0}\n",
        "{\"type\":\"data\",\"data\":{\"stop\":\"Gare Centrale\"}}\n",
        "{\"type\":\"stop\"}\n",
    );
    let ran = drive(script, None, Script::default());

    assert_eq!(
        ran.calls,
        ["open", "wheel 2", "wheel -1", "wheel 2", "data Gare Centrale", "stop"]
    );
    assert_eq!(
        ran.sent,
        vec![
            json!({"type": "scene", "items": [{"k": "text", "c": "#ffffff", "s": "open"}]}),
            json!({"type": "tick"}),
            json!({"type": "event", "data": {"wheel": 2}}),
            json!({"type": "scene", "items": [{"k": "text", "c": "#ffffff", "s": "wheel2"}]}),
            json!({"type": "tick"}),
            json!({"type": "event", "data": {"wheel": -1}}),
            json!({"type": "scene", "items": [{"k": "text", "c": "#ffffff", "s": "wheel-1"}]}),
            json!({"type": "tick"}),
            json!({"type": "event", "data": {"wheel": 2}}),
            json!({"type": "scene", "items": [{"k": "text", "c": "#ffffff", "s": "wheel2"}]}),
            json!({"type": "scene", "items": [{"k": "text", "c": "#ffffff", "s": "data"}]}),
        ]
    );
    // One message per line, one newline each, and nothing after `stop`.
    assert_eq!(ran.raw.len(), 11);
    // serde_json writes an object's keys in sorted order (the SDK adds no `preserve_order`
    // dependency); the Nest parses JSON and does not care.
    assert_eq!(
        ran.raw[0],
        "{\"items\":[{\"c\":\"#ffffff\",\"k\":\"text\",\"s\":\"open\"}],\"type\":\"scene\"}\n"
    );
    assert!(
        ran.raw
            .iter()
            .all(|l| l.ends_with('\n') && l.matches('\n').count() == 1)
    );
}

#[test]
fn unknown_and_malformed_messages_are_ignored_silently() {
    let script = concat!(
        "{\"type\":\"hello world\"}\n",
        "{not json at all\n",
        "[1,2,3]\n",
        "\n",
        "{\"type\":\"wheel\"}\n",
        "{\"type\":\"data\"}\n",
        "{\"type\":\"wheel\",\"steps\":\"two\"}\n",
        "null\n",
        "{\"type\":\"open\"}\n",
    );
    let (mut mine, theirs) = UnixStream::pair().expect("socketpair");
    mine.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("read timeout");
    let mut app = App::from_stream(theirs);
    let app_thread = std::thread::spawn(move || {
        let mut handler = Script::default();
        let run = app.run_with_timeout(None, &mut handler);
        (handler, run.map_err(|e| e.to_string()), app.ignored())
    });
    mine.write_all(script.as_bytes()).expect("write the script");
    mine.shutdown(Shutdown::Write).expect("end the script");
    let (ran, run, ignored) = app_thread.join().expect("the app thread");

    assert_eq!(run, Ok(()), "nothing malformed may end the loop");
    assert_eq!(ran.calls, ["open"], "only the good message got through");
    assert_eq!(ignored, 8, "one per message that could not be used");
    assert_eq!(drain(&mine).len(), 1, "and it answered the one it understood");
}

#[test]
fn close_ends_the_loop_and_is_the_last_word() {
    let script = Script {
        close_after_wheels: Some(1),
        ..Script::default()
    };
    let ran = drive(
        "{\"type\":\"open\"}\n{\"type\":\"wheel\",\"steps\":1}\n{\"type\":\"wheel\",\"steps\":1}\n",
        None,
        script,
    );
    assert_eq!(
        ran.calls,
        ["open", "wheel 1"],
        "nothing after the close, buffered or not"
    );
    assert_eq!(ran.sent.last(), Some(&json!({"type": "close"})));
}

#[test]
fn a_deadline_ticks_and_a_half_read_message_is_not_lost() {
    let (mut mine, theirs) = UnixStream::pair().expect("socketpair");
    mine.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("read timeout");
    let mut app = App::from_stream(theirs);
    let mut script = Script {
        close_after_ticks: Some(2),
        ..Script::default()
    };
    let app_thread = std::thread::spawn(move || {
        let run = app.run_with_timeout(Some(Duration::from_millis(10)), &mut script);
        (script, run.map_err(|e| e.to_string()))
    });
    // Half a message, then a wait long enough for several ticks, then the rest.
    mine.write_all(b"{\"type\":\"op").expect("write");
    std::thread::sleep(Duration::from_millis(250));
    mine.write_all(b"en\"}\n").expect("write");

    let (ran, run) = app_thread.join().expect("the app thread");
    let sent: Vec<Value> = drain(&mine)
        .iter()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(run, Ok(()));
    assert_eq!(ran.calls, ["open"], "the two halves made one message");
    let at = ran.ticks_at_open.expect("open arrived");
    assert!(at >= 2, "the loop waited: {at} ticks before the message completed");
    assert!(ran.ticks >= 4, "and kept ticking after it");
    assert_eq!(
        sent.first(),
        Some(&json!({"type": "scene", "items": [{"k": "text", "c": "#ffffff", "s": "open"}]}))
    );
    assert_eq!(sent.last(), Some(&json!({"type": "close"})));
}

#[test]
fn an_oversized_message_is_dropped_and_the_stream_resynchronises() {
    let (mut mine, theirs) = UnixStream::pair().expect("socketpair");
    mine.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("read timeout");
    let mut app = App::from_stream(theirs);
    let app_thread = std::thread::spawn(move || {
        let mut handler = Script::default();
        let run = app.run_with_timeout(None, &mut handler);
        (handler, run.map_err(|e| e.to_string()), app.ignored())
    });
    // A message far over the 64 KiB cap with no newline in sight, then two good ones.
    mine.write_all(&vec![b'x'; 200 * 1024]).expect("write");
    mine.write_all(b"\n{\"type\":\"open\"}\n{\"type\":\"stop\"}\n")
        .expect("write");
    mine.shutdown(Shutdown::Write).expect("end the script");
    let (ran, run, ignored) = app_thread.join().expect("the app thread");

    assert_eq!(run, Ok(()));
    assert_eq!(ran.calls, ["open", "stop"], "the stream recovered on the next newline");
    assert_eq!(ignored, 1);
    assert_eq!(drain(&mine).len(), 1);
}

#[test]
fn the_end_of_the_connection_ends_the_loop() {
    let ran = drive("", None, Script::default());
    assert!(ran.calls.is_empty());
    assert!(ran.sent.is_empty());
}

#[test]
fn a_scene_is_capped_on_the_wire() {
    struct Wide;
    impl Handler for Wide {
        fn on_open(&mut self, app: &mut App) {
            let items = vec![json!({"k": "fill", "c": "#ffffff", "d": "M0 0L1 1Z"}); MAX_ITEMS + 44];
            app.send_scene(&items);
            // An event payload that cannot fit one message is dropped, not truncated.
            app.event(&json!({"big": "y".repeat(MAX_MESSAGE)}));
            app.tick();
        }
    }

    let (mine, theirs) = UnixStream::pair().expect("socketpair");
    mine.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("read timeout");
    let mut app = App::from_stream(theirs);
    let app_thread = std::thread::spawn(move || {
        app.run(&mut Wide).expect("the loop");
    });
    // One message in, then the end of the connection: the app answers and stops.
    {
        let mut nest = mine.try_clone().expect("clone");
        nest.write_all(b"{\"type\":\"open\"}\n").expect("write");
        nest.shutdown(Shutdown::Write).expect("end");
    }
    app_thread.join().expect("the app thread");
    let lines = drain(&mine);

    assert_eq!(lines.len(), 2, "the scene and the tick, and no oversized event");
    let scene: Value = serde_json::from_str(&lines[0]).expect("a scene");
    assert_eq!(scene["type"], "scene");
    assert_eq!(scene["items"].as_array().expect("items").len(), MAX_ITEMS);
    assert!(
        lines[0].len() <= MAX_MESSAGE,
        "one message is at most 64 KiB with its newline"
    );
    assert_eq!(lines[1], "{\"type\":\"tick\"}\n");
}
