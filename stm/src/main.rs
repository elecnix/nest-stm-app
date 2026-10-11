//! The STM bus app: the departures Home Assistant pushes for one stop.
//!
//! The app fetches nothing. Home Assistant polls the transit API and pushes a payload shaped like
//! `{"stop": "Gare Centrale", "departures": [{"route": "55", "headsign": "…", "minutes": 3,
//! "time": "14:32"}]}`; this app shows the stop as a header and three departures a page, and the
//! wheel turns the pages.
//!
//! Four screens, and no way to reach a fifth:
//!
//! * **Waiting** — nothing pushed yet: a bus icon and "Waiting for the bus...".
//! * **No departures** — a payload arrived and the list is empty, under the stop's name.
//! * **A board** — up to three rows of bus icon, bold route number, headsign and the minutes
//!   right-aligned, paged by the wheel.
//! * **Whatever else** — a payload that is not a board at all reads as an empty board, because a
//!   transit feed having a bad minute is not a reason for a black screen.
//!
//! It does not tick and it does not animate: there is nothing on a departure board to age, and the
//! Nest draws the same scene all afternoon for free. The loop blocks on the socket and ends on
//! `stop`, or on the end of the connection.

mod departures;
mod screen;

use nest_app_sdk::{App, Handler};
use serde_json::Value;

use crate::departures::Board;

/// What the app remembers between messages.
#[derive(Default)]
struct Stm {
    /// The last board Home Assistant pushed, if any.
    board: Option<Board>,
    /// Which page of it the wheel has turned to.
    page: usize,
}

impl Stm {
    /// Turn to another page. False when the wheel moves nothing — one page, or a whole lap.
    fn turn(&mut self, steps: i32) -> bool {
        let pages = self.board.as_ref().map_or(1, Board::pages) as i64;
        let turned = (self.page as i64 + i64::from(steps)).rem_euclid(pages) as usize;
        if turned == self.page {
            return false;
        }
        self.page = turned;
        true
    }

    /// Take a payload, and land on a page that exists.
    fn update(&mut self, data: &Value) {
        let board = Board::parse(data);
        self.page = self.page.min(board.pages() - 1);
        self.board = Some(board);
    }
}

impl Handler for Stm {
    fn on_open(&mut self, app: &mut App) {
        app.send_scene(&screen::screen(self.board.as_ref(), self.page));
    }

    fn on_wheel(&mut self, app: &mut App, steps: i32) {
        if !self.turn(steps) {
            return;
        }
        app.tick();
        app.send_scene(&screen::screen(self.board.as_ref(), self.page));
    }

    fn on_data(&mut self, app: &mut App, data: &Value) {
        self.update(data);
        app.send_scene(&screen::screen(self.board.as_ref(), self.page));
    }

    fn on_stop(&mut self, _app: &mut App) {
        // Nothing to clean up: the Nest is closing the app, and `main` returns.
    }
}

fn main() {
    let mut app = App::connect();
    let mut stm = Stm::default();
    if let Err(e) = app.run(&mut stm) {
        eprintln!("stm: the Nest is gone: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(count: usize) -> Value {
        json!({
            "stop": "Gare Centrale",
            "departures": (0..count)
                .map(|i| json!({"route": format!("5{i}"), "headsign": "Somewhere", "minutes": i}))
                .collect::<Vec<_>>(),
        })
    }

    #[test]
    fn the_page_turns_with_the_wheel_and_stops_at_the_ends() {
        let mut stm = Stm::default();
        // Nothing pushed yet: one page, and the wheel cannot leave it.
        assert!(!stm.turn(3));
        assert_eq!(stm.page, 0);
        assert_eq!(stm.board, None);

        stm.update(&payload(7));
        assert_eq!(stm.board.as_ref().expect("board").pages(), 3);
        assert_eq!(stm.page, 0);
        assert!(stm.turn(1) && stm.page == 1);
        assert!(stm.turn(1) && stm.page == 2);
        assert!(stm.turn(1) && stm.page == 0, "the pages are a ring");
        assert!(stm.turn(-1) && stm.page == 2, "and so is the other way");
        assert!(stm.turn(4) && stm.page == 0);
        assert!(stm.turn(-7) && stm.page == 2);
        // A whole lap changes nothing, so nothing is redrawn and nothing clicks.
        assert!(!stm.turn(3));
        assert!(!stm.turn(0));
    }

    #[test]
    fn a_new_payload_keeps_the_reader_on_a_page_that_exists() {
        let mut stm = Stm::default();
        stm.update(&payload(9));
        assert_eq!(stm.page, 0);
        assert!(stm.turn(2) && stm.page == 2);

        // The feed shrinks to one page: the reader lands on it rather than on page 3 of 1.
        stm.update(&payload(2));
        assert_eq!(stm.page, 0);
        assert_eq!(stm.board.as_ref().expect("board").pages(), 1);
        assert!(!stm.turn(1));

        // And a feed that is not a board at all is an empty one, not an error.
        stm.update(&json!("not a board"));
        assert_eq!(stm.board.as_ref().expect("board").pages(), 1);
        assert!(stm.board.as_ref().expect("board").departures.is_empty());
    }

    #[test]
    fn the_loop_answers_open_data_wheel_and_stop() {
        // The one test that needs a socket: it drives the real loop over a socket pair, so what
        // passes here is the app the Nest will run.
        use std::io::{BufRead, BufReader, ErrorKind, Write as _};
        use std::net::Shutdown;
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let (mut mine, theirs) = UnixStream::pair().expect("pair");
        mine.set_read_timeout(Some(Duration::from_millis(200)))
            .expect("timeout");
        let mut app = App::from_stream(theirs);
        let thread = std::thread::spawn(move || {
            let mut stm = Stm::default();
            app.run(&mut stm).expect("the loop");
        });
        mine.write_all(b"{\"type\":\"open\"}\n").expect("write");
        mine.write_all(format!("{{\"type\":\"data\",\"data\":{}}}\n", payload(4)).as_bytes())
            .expect("write");
        mine.write_all(b"{\"type\":\"wheel\",\"steps\":1}\n").expect("write");
        mine.write_all(b"{\"type\":\"stop\"}\n").expect("write");
        mine.shutdown(Shutdown::Write).expect("shutdown");
        thread.join().expect("the app thread");

        let mut reader = BufReader::new(mine);
        let mut sent = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => sent.push(serde_json::from_str::<Value>(&line).expect("a JSON line")),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
                Err(e) => panic!("{e}"),
            }
        }
        // open: the waiting screen; data: page 1 of 2; wheel: a click, then page 2 of 2.
        assert_eq!(sent.len(), 4, "{sent:#?}");
        assert_eq!(sent[0]["type"], "scene");
        assert_eq!(
            sent[0]["items"].as_array().expect("items").len(),
            3,
            "the waiting screen"
        );
        assert_eq!(sent[1]["type"], "scene");
        assert_eq!(sent[2], json!({"type": "tick"}));
        let texts: Vec<&str> = sent[3]["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter(|i| i["k"] == "text")
            .filter_map(|i| i["s"].as_str())
            .collect();
        assert!(!texts.contains(&"1/2"), "{texts:?}");
        assert!(texts.contains(&"2/2"), "{texts:?}");
    }
}
