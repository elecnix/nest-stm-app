//! # nest-app-sdk
//!
//! What a [Nest Assistant](https://github.com/elecnix/nest-assistant) app builds on. An app is a
//! single program the Nest starts on demand, running chrooted as its own unprivileged user. It
//! never touches the framebuffer: it reads input and Home Assistant's payloads from a Unix socket
//! on file descriptor 3 and answers with scenes.
//!
//! ```no_run
//! use nest_app_sdk::{path, scene, App, Handler};
//!
//! struct Counter(i64);
//!
//! impl Handler for Counter {
//!     fn on_open(&mut self, app: &mut App) {
//!         app.send_scene(&self.scene());
//!     }
//!     fn on_wheel(&mut self, app: &mut App, steps: i32) {
//!         self.0 += steps as i64;
//!         app.tick();
//!         app.send_scene(&self.scene());
//!     }
//! }
//!
//! impl Counter {
//!     fn scene(&self) -> Vec<serde_json::Value> {
//!         use scene::{fill, text, rgb, Anchor, TextStyle};
//!         vec![
//!             fill(path::circle(160.0, 160.0, 120.0), rgb(0x10, 0x2b, 0x29)),
//!             text(
//!                 &self.0.to_string(),
//!                 160.0,
//!                 175.0,
//!                 72.0,
//!                 "#ffffff",
//!                 TextStyle::new().bold().anchor(Anchor::Middle),
//!             ),
//!         ]
//!     }
//! }
//!
//! // In the real thing this is `main`;
//! let mut app = App::connect();
//! let _ = app.run(&mut Counter(0));
//! ```
//!
//! The protocol is newline-delimited UTF-8 JSON on fd 3; one message is at most 64 KiB including
//! its newline. [`App`] owns the socket, [`Handler`] is the app's half of the loop, [`scene`] and
//! [`path`] build what the Nest draws and [`Store`] is the app's one writable directory.
//!
//! Two rules the SDK cannot enforce for you:
//!
//! * **Relative paths only.** The app is chrooted into its own directory: `data/state.json` is
//!   right, `/data/state.json` is not (it is neither absolute nor inside the filesystem the app
//!   can see).
//! * **No network.** Everything an app receives arrives through `data` messages. Opening a socket
//!   is outside the API.
//!
//! The SDK is std-only and behaves identically on the Nest and on a developer's host, so an app's
//! tests run with `cargo test` and the same binary runs against a fake Nest.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod path;
pub mod protocol;
pub mod scene;
pub mod store;

pub use protocol::{APP_FD, App, Handler};
pub use store::Store;

/// The display canvas, in pixels: (0, 0) is the top left, (320, 320) the bottom right.
///
/// The screen itself is a disc of radius ~163 px centred on (160, 160), so an app that wants its
/// content to stay on the glass keeps within ~150 px of the centre.
pub const CANVAS: f32 = 320.0;

/// The largest message on the wire, its newline included.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// The largest scene: a scene with more items is truncated to this many.
pub const MAX_ITEMS: usize = 256;

/// The largest number of path segments in one item.
pub const MAX_SEGS: usize = 4096;

/// The longest text item, in characters.
pub const MAX_TEXT: usize = 200;

/// The smallest font size, for text and icons.
pub const MIN_SIZE: f32 = 4.0;

/// The largest font size, for text and icons.
pub const MAX_SIZE: f32 = 120.0;

/// The largest stroke width.
pub const MAX_STROKE: f32 = 64.0;
