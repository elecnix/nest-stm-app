//! The Nest's half of the conversation: newline-delimited UTF-8 JSON on fd 3.
//!
//! The Nest starts the app with one end of a socket pair as file descriptor 3 and sends `open`.
//! From then on it forwards the ring's `wheel` steps and Home Assistant's `data` payloads, and
//! draws whatever scenes come back. A message is a JSON object with a `type` and is at most 64 KiB
//! including its newline.
//!
//! | From the Nest | Meaning |
//! |---|---|
//! | `{"type": "open"}` | the process was started; answer with the first scene |
//! | `{"type": "wheel", "steps": 2}` | the ring turned this many detents; positive is clockwise |
//! | `{"type": "data", "data": …}` | a payload Home Assistant pushed, once right after `open` if one is cached, and live while the app runs |
//! | `{"type": "stop"}` | the Nest is closing the app; exit with status 0 |
//!
//! | From the app | Meaning |
//! |---|---|
//! | `send_scene` | replace the whole screen |
//! | `tick` | play the ring click, rate-limited to one per 50 ms |
//! | `event` | tell Home Assistant something, forwarded to it as an `app_event` |
//! | `close` | ask the Nest to close the app |
//!
//! Anything else — not JSON, not an object, no known `type`, a `wheel` with no `steps` — is
//! ignored, silently and without ending the loop: the protocol is additive, and an app must
//! survive a message a newer Nest sends it. (The Nest counts those ignores, and kills an app that
//! provokes more than 32 of them in one run; the SDK gives an app no way to send one.) [`App`]
//! holds the connection, [`Handler`] is the app; [`App::run_with_timeout`] lets an app animate or
//! do periodic work between messages.

use std::io::{self, ErrorKind, Read, Write};
use std::mem::ManuallyDrop;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{Value, json};

use crate::{MAX_ITEMS, MAX_MESSAGE};

/// The file descriptor the Nest hands the app: its end of a socket pair.
pub const APP_FD: i32 = 3;

/// The fixed part of a scene message: `{"type":"scene","items":[]}`, newline not included.
const SCENE_SKELETON: usize = 27;

/// What the Nest said.
enum Incoming {
    /// A parsed message object.
    Message(Value),
    /// Nothing complete arrived before the read deadline.
    Timeout,
    /// The socket was closed: the Nest is gone (or the app asked to close).
    Eof,
}

/// An app's connection to the Nest, and its half of the loop.
///
/// The struct owns the socket but never closes it — fd 3 belongs to the process, and the app may
/// outlive the loop — so an [`App`] dropped at the end of `main` leaves fd 3 exactly as it found it.
pub struct App {
    stream: ManuallyDrop<UnixStream>,
    /// Bytes read but not yet consumed.
    buf: Vec<u8>,
    /// True while discarding the tail of a message that was over the 64 KiB cap.
    skip: bool,
    /// The deadline currently on the socket, so it is set only when it changes.
    read_timeout: Option<Duration>,
    /// Messages queued by the app, written out by the loop.
    outbox: Vec<String>,
    /// True once the app asked to close, or the Nest said `stop`.
    done: bool,
    ignored: usize,
}

impl App {
    /// Connect to the Nest on file descriptor 3.
    ///
    /// Never fails: if fd 3 is not the socket the Nest passed, the first read or write reports it
    /// (`ENOTSOCK`) and [`App::run`] returns that error. Run the host build under a fake Nest that
    /// sets fd 3 up the same way (`apps/README.md` has the recipe).
    pub fn connect() -> App {
        // SAFETY: fd 3 is the socket end the Nest inherited into this process; wrapping it in a
        // `UnixStream` (which App never closes) is how that end is used.
        let stream = unsafe { UnixStream::from_raw_fd(APP_FD) };
        App::from_stream(stream)
    }

    /// An app on a socket of your own — `UnixStream::pair()` in a test, or an embedding.
    ///
    /// The socket is not closed when the `App` goes away: fd 3 is the process's, not the SDK's.
    pub fn from_stream(stream: UnixStream) -> App {
        App {
            stream: ManuallyDrop::new(stream),
            buf: Vec::with_capacity(1024),
            skip: false,
            read_timeout: None,
            outbox: Vec::new(),
            done: false,
            ignored: 0,
        }
    }

    /// Read until the Nest closes the socket, handling every message as it arrives.
    pub fn run<H: Handler>(&mut self, handler: &mut H) -> io::Result<()> {
        self.run_with_timeout(None, handler)
    }

    /// The same loop, with a read deadline.
    ///
    /// `timeout` is how long the loop waits for input before calling [`Handler::on_tick`] and
    /// going back to waiting; `None` blocks until a message arrives. This is what lets an app
    /// animate, or refresh "3 min ago", without a thread and without polling. A `Timeout` with a
    /// half-read message in flight keeps the bytes: nothing that arrives is lost to a tick.
    pub fn run_with_timeout<H: Handler>(&mut self, timeout: Option<Duration>, handler: &mut H) -> io::Result<()> {
        self.set_read_timeout(timeout)?;
        while !self.done {
            match self.next_message()? {
                Incoming::Message(msg) => self.dispatch(handler, &msg),
                Incoming::Timeout => handler.on_tick(self),
                Incoming::Eof => break,
            }
            self.flush()?;
        }
        Ok(())
    }

    /// Replace the whole screen with `items`.
    ///
    /// The scene is capped: the first [`MAX_ITEMS`] items are kept, and then only as many as fit in
    /// one 64 KiB message. An item that cannot fit is dropped rather than the message, since a
    /// message the Nest cannot parse would cost the app one of its 32 chances. A scene whose first
    /// item is itself over the cap is sent empty.
    pub fn send_scene(&mut self, items: &[Value]) {
        let msg = json!({"type": "scene", "items": fit_items(items)});
        self.queue(&msg);
    }

    /// Play the standard ring click.
    pub fn tick(&mut self) {
        self.queue(&json!({"type": "tick"}));
    }

    /// Tell Home Assistant something — anything that serialises to JSON.
    pub fn event(&mut self, data: &Value) {
        self.queue(&json!({"type": "event", "data": data}));
    }

    /// Ask the Nest to close the app. The loop returns after this message is written.
    pub fn close(&mut self) {
        self.queue(&json!({"type": "close"}));
        self.done = true;
    }

    /// True once [`App::close`] was called or the Nest said `stop`.
    pub fn is_closed(&self) -> bool {
        self.done
    }

    /// How many messages from the Nest were dropped — unparsable, or of an unknown `type`.
    ///
    /// Diagnostics only: the protocol asks an app to ignore them, and this is what it ignored.
    pub fn ignored(&self) -> usize {
        self.ignored
    }

    /// Write queued messages out, in order.
    ///
    /// Called by the loop after every message and every tick; an app that drives [`App`] itself
    /// instead of calling [`App::run`] calls this to make its `tick`, `event`, `send_scene` and
    /// `close` visible to the Nest.
    pub fn flush(&mut self) -> io::Result<()> {
        for msg in std::mem::take(&mut self.outbox) {
            self.stream.write_all(msg.as_bytes())?;
        }
        Ok(())
    }

    /// Frame `msg` and queue it, unless it does not fit one message.
    fn queue(&mut self, msg: &Value) {
        if let Ok(mut line) = serde_json::to_string(msg) {
            line.push('\n');
            if line.len() <= MAX_MESSAGE {
                self.outbox.push(line);
            }
        }
    }

    /// Handle one message from the Nest.
    fn dispatch<H: Handler>(&mut self, handler: &mut H, msg: &Value) {
        match msg.get("type").and_then(Value::as_str) {
            Some("open") => handler.on_open(self),
            Some("wheel") => match steps_of(msg) {
                Some(steps) => handler.on_wheel(self, steps),
                None => self.ignored += 1,
            },
            Some("data") => match msg.get("data") {
                Some(data) => handler.on_data(self, data),
                None => self.ignored += 1,
            },
            Some("stop") => {
                self.done = true;
                handler.on_stop(self);
            }
            _ => self.ignored += 1,
        }
    }

    /// The next message, a timeout, or the end of the connection.
    ///
    /// A line that is not a JSON object is skipped, silently and without ending the loop; a line
    /// over the cap is discarded up to the next newline, so an oversized message costs one message
    /// rather than desynchronising the stream.
    fn next_message(&mut self) -> io::Result<Incoming> {
        loop {
            if self.skip {
                match self.buf.iter().position(|&b| b == b'\n') {
                    Some(nl) => {
                        self.buf.drain(..=nl);
                        self.skip = false;
                    }
                    None => self.buf.clear(),
                }
            }
            if !self.skip {
                if let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = self.buf.drain(..=nl).collect();
                    match serde_json::from_slice::<Value>(&line[..line.len() - 1]) {
                        Ok(msg) => return Ok(Incoming::Message(msg)),
                        Err(_) => {
                            self.ignored += 1;
                            continue;
                        }
                    }
                }
                // No newline yet, and already a whole message's worth of bytes: it never will be
                // one, so drop it and resynchronise on the next newline.
                if self.buf.len() >= MAX_MESSAGE {
                    self.ignored += 1;
                    self.skip = true;
                    self.buf.clear();
                    continue;
                }
            }
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(Incoming::Eof),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return Ok(Incoming::Timeout);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        if self.read_timeout != timeout {
            self.stream.set_read_timeout(timeout)?;
            self.read_timeout = timeout;
        }
        Ok(())
    }
}

/// The `steps` of a `wheel` message: an integer, and a float too, so that a Nest that sends
/// `2.0` is understood rather than ignored.
fn steps_of(msg: &Value) -> Option<i32> {
    let steps = match msg.get("steps")? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))?,
        _ => return None,
    };
    Some(steps.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

/// The items of a scene that fit one message.
fn fit_items(items: &[Value]) -> Vec<Value> {
    let mut kept: Vec<Value> = Vec::new();
    // The newline counts towards the cap, so start at the skeleton plus one.
    let mut len = SCENE_SKELETON + 1;
    for item in items.iter().take(MAX_ITEMS) {
        let Ok(rendered) = serde_json::to_string(item) else {
            continue;
        };
        let extra = rendered.len() + usize::from(!kept.is_empty());
        if len + extra > MAX_MESSAGE {
            break;
        }
        len += extra;
        kept.push(item.clone());
    }
    kept
}

/// The app: the callbacks the loop calls as the Nest talks to it.
///
/// Every method but [`Handler::on_open`] does nothing by default. Each one gets the [`App`] to
/// send with, so a handler is one `impl` block and no state lives in the SDK.
///
/// ```no_run
/// use nest_app_sdk::{scene, App, Handler};
///
/// struct Clock;
///
/// impl Handler for Clock {
///     fn on_open(&mut self, app: &mut App) {
///         app.send_scene(&[scene::text("12:00", 160.0, 175.0, 64.0, "#ffffff", scene::TextStyle::new())]);
///     }
///     fn on_tick(&mut self, app: &mut App) {
///         app.send_scene(&[]); // once a minute, on the loop's own clock
///     }
///     fn on_stop(&mut self, _app: &mut App) {}
/// }
/// ```
pub trait Handler {
    /// The process was started. Send the first scene.
    ///
    /// Home Assistant's cached payload, if there is one, arrives as [`Handler::on_data`] right
    /// after this, so a first scene can reasonably be drawn from the persisted state and replaced
    /// a moment later.
    fn on_open(&mut self, app: &mut App);

    /// The ring turned `steps` detents; positive is clockwise.
    ///
    /// The Nest coalesces the steps of one turn, so `2` or `5` in one message is normal, and an app
    /// that moves a selection by one per step should loop rather than treat it as a single nudge.
    fn on_wheel(&mut self, _app: &mut App, _steps: i32) {}

    /// Home Assistant pushed `data` for this app.
    fn on_data(&mut self, _app: &mut App, _data: &Value) {}

    /// The read deadline passed with nothing to read; only with [`App::run_with_timeout`].
    fn on_tick(&mut self, _app: &mut App) {}

    /// The Nest is closing the app: the loop returns as soon as this returns, and `main` should
    /// return so that the process exits with status 0.
    fn on_stop(&mut self, _app: &mut App) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_skeleton_is_counted_right() {
        let skeleton = json!({"type": "scene", "items": []}).to_string();
        assert_eq!(skeleton.len(), SCENE_SKELETON, "{skeleton}");
    }

    #[test]
    fn a_scene_keeps_what_fits() {
        let small = vec![json!({"k": "fill", "c": "#ffffff", "d": "M0 0L1 1Z"}); 10];
        assert_eq!(fit_items(&small).len(), 10);

        // At most MAX_ITEMS items, whatever the caller built.
        let many = vec![json!({"k": "tick"}); MAX_ITEMS + 40];
        assert_eq!(fit_items(&many).len(), MAX_ITEMS);

        // A scene over the cap loses its tail, not the message.
        let fat = json!({"k": "fill", "c": "#ffffff", "d": "M0 0".repeat(4096)});
        let items = vec![
            json!({"k": "fill", "c": "#ffffff", "d": "M0 0"}),
            fat.clone(),
            fat.clone(),
            fat.clone(),
            fat,
        ];
        let kept = fit_items(&items);
        assert!(kept.len() < items.len() && !kept.is_empty());
        for item in &kept {
            assert!(serde_json::to_string(item).expect("json").len() < MAX_MESSAGE);
        }
        let message = json!({"type": "scene", "items": kept}).to_string();
        assert!(message.len() < MAX_MESSAGE, "the newline has to fit too");

        // One item that is over the cap on its own: an empty scene, not an oversized message.
        let huge = json!({"k": "fill", "c": "#ffffff", "d": "M0 0".repeat(20000)});
        assert!("M0 0".repeat(20000).len() > MAX_MESSAGE);
        assert!(fit_items(&[huge]).is_empty());
    }

    #[test]
    fn wheel_steps_are_read_leniently() {
        assert_eq!(steps_of(&json!({"steps": 2})), Some(2));
        assert_eq!(steps_of(&json!({"steps": -3})), Some(-3));
        assert_eq!(steps_of(&json!({"steps": 2.0})), Some(2));
        assert_eq!(steps_of(&json!({"steps": 1e12})), Some(i32::MAX));
        assert_eq!(steps_of(&json!({"steps": -1e12})), Some(i32::MIN));
        assert_eq!(steps_of(&json!({})), None);
        assert_eq!(steps_of(&json!({"steps": "2"})), None);
    }
}
