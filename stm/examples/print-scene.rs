//! Print the scene one payload draws, as the app sends it to the Nest.
//!
//! The capture pipeline for the documentation and the pull requests: the real
//! payload on stdin, the real [`screen`] code, the real scene on stdout —
//! which `nest-app`'s `capture-scene` example then renders with the Nest's own
//! decoder and rasteriser.
//!
//! ```sh
//! cat payload.json | cargo run -p nest-app-stm --example print-scene -- 0 \
//!   | NEST_FONTS=… cargo run -p nestui --example capture-scene -- out.png
//! ```
//!
//! The argument is the page the wheel would have turned to.

use std::io::Read;

fn main() {
    let page: usize = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let mut payload = String::new();
    std::io::stdin().read_to_string(&mut payload).expect("read the payload");
    let value: serde_json::Value = serde_json::from_str(&payload).expect("a JSON payload");
    let board = nest_app_stm::departures::Board::parse(&value);
    let items = nest_app_stm::screen::screen(Some(&board), page);
    println!("{}", serde_json::Value::Array(items).to_string());
}
