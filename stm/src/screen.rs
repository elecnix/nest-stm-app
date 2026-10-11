//! The board, as the Nest draws it.
//!
//! A stop name and a bus at the top, three rows under it, and a page number when there is more
//! than one page. Everything stays inside a disc of radius ~150 px around (160, 160), inside the
//! display's ~163 px rim, so the glass never cuts a row in half.

use nest_app_sdk::{
    path,
    scene::{self, Anchor, TextStyle},
};
use serde_json::Value;

use crate::departures::{Board, Departure, MAX_HEADSIGN, MAX_STOP, cut};

/// The canvas centre, and the centre of the disc the screen is.
const C: f32 = 160.0;
/// The teal the Nest's own entity discs are filled with.
const TEAL: &str = "#7cd6c9";
/// The stop name and the route numbers.
const WHITE: &str = "#ffffff";
/// Everything that is not the point: the bus glyphs, the headsigns, the rules, the page number.
const DIM: &str = "#2f5b56";
/// Where the three rows' baselines sit.
const ROWS: [f32; 3] = [108.0, 168.0, 228.0];

/// The whole scene for a board — `None` before Home Assistant has pushed anything.
pub fn screen(board: Option<&Board>, page: usize) -> Vec<Value> {
    let Some(board) = board else {
        return waiting();
    };
    let page = page % board.pages();
    let mut items = header(&board.stop);
    if board.departures.is_empty() {
        empty(&mut items);
        return items;
    }
    let rows = board.page(page);
    for (row, departure) in rows.iter().enumerate() {
        departure_row(&mut items, departure, ROWS[row]);
        if row + 1 < rows.len() {
            items.push(scene::stroke(
                path::line(36.0, ROWS[row] + 50.0, 284.0, ROWS[row] + 50.0),
                DIM,
                1.0,
                false,
            ));
        }
    }
    let pages = board.pages();
    if pages > 1 {
        items.push(scene::text(
            &format!("{}/{}", page + 1, pages),
            C,
            292.0,
            10.0,
            DIM,
            TextStyle::new(),
        ));
    }
    items
}

/// Before the first payload: the bus, and the wait.
fn waiting() -> Vec<Value> {
    vec![
        scene::icon("mdi-bus", C, 132.0, 40.0, DIM),
        scene::text("Waiting for the bus...", C, 184.0, 14.0, WHITE, TextStyle::new()),
        scene::text(
            "Home Assistant pushes the departures",
            C,
            208.0,
            9.5,
            DIM,
            TextStyle::new(),
        ),
    ]
}

/// The stop's name, a bus beside it, and a rule under it.
fn header(stop: &str) -> Vec<Value> {
    let title = if stop.trim().is_empty() {
        "DEPARTURES".to_string()
    } else {
        cut(stop, MAX_STOP)
    };
    vec![
        scene::icon("mdi-bus", 52.0, 58.0, 26.0, TEAL),
        scene::text(
            &title,
            76.0,
            64.0,
            18.0,
            WHITE,
            TextStyle::new().bold().anchor(Anchor::Start).tracking(0.5),
        ),
        scene::stroke(path::line(36.0, 86.0, 284.0, 86.0), DIM, 1.0, false),
    ]
}

/// A stop with nothing leaving it.
fn empty(items: &mut Vec<Value>) {
    items.push(scene::icon("mdi-bus", C, 150.0, 36.0, DIM));
    items.push(scene::text("No departures", C, 202.0, 16.0, WHITE, TextStyle::new()));
}

/// One departure: the bus, the line number, where it goes, and how long.
fn departure_row(items: &mut Vec<Value>, departure: &Departure, y: f32) {
    items.push(scene::icon("mdi-bus", 44.0, y + 16.0, 22.0, DIM));
    items.push(scene::text(
        if departure.route.is_empty() {
            "?"
        } else {
            &departure.route
        },
        72.0,
        y + 14.0,
        22.0,
        WHITE,
        TextStyle::new().bold().anchor(Anchor::Start),
    ));
    if !departure.headsign.is_empty() {
        items.push(scene::text(
            &cut(&departure.headsign, MAX_HEADSIGN),
            72.0,
            y + 32.0,
            12.0,
            DIM,
            TextStyle::new().anchor(Anchor::Start),
        ));
    }
    let when = departure.when();
    if !when.is_empty() {
        items.push(scene::text(
            &when,
            276.0,
            y + 16.0,
            17.0,
            TEAL,
            TextStyle::new().bold().anchor(Anchor::End),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::departures::PER_PAGE;
    use nest_app_sdk::{MAX_ITEMS, MAX_STROKE, MAX_TEXT};

    fn departure(route: &str, minutes: Option<f64>) -> Departure {
        Departure {
            route: route.to_string(),
            headsign: format!("{route} somewhere"),
            minutes,
            time: None,
        }
    }

    /// A board of `count` departures, numbered 50, 51, 52…, one minute apart.
    fn board_of(count: usize) -> Board {
        Board {
            stop: "Gare Centrale".to_string(),
            departures: (0..count)
                .map(|i| departure(&format!("5{i}"), Some(i as f64)))
                .collect(),
        }
    }

    /// A scene is what the docs promise: at most 256 items, each a known kind, in a colour the
    /// wire takes, with the fields that kind has.
    fn check(items: &[Value]) {
        assert!(!items.is_empty(), "a scene with nothing in it is a black screen");
        assert!(items.len() <= MAX_ITEMS, "{} items", items.len());
        for item in items {
            let k = item["k"].as_str().unwrap_or_else(|| panic!("no kind in {item}"));
            let color = item["c"].as_str().unwrap_or_else(|| panic!("no colour in {item}"));
            assert_eq!(color.len(), 7, "{color} is not #rrggbb");
            assert!(color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit()));
            match k {
                "fill" | "stroke" => {
                    let d = item["d"].as_str().expect("path data");
                    assert!(d.starts_with('M'), "{d} does not start with a move");
                    assert!(
                        d.chars()
                            .all(|c| matches!(c, 'M' | 'L' | 'Q' | 'C' | 'Z' | ' ' | '.' | '-' | '0'..='9')),
                        "path data outside the parser's alphabet: {d}"
                    );
                    if k == "stroke" {
                        let w = item["w"].as_f64().expect("stroke width");
                        assert!((0.0..=f64::from(MAX_STROKE)).contains(&w), "{w}");
                    }
                }
                "text" | "icon" => {
                    assert!((4.0..=120.0).contains(&item["size"].as_f64().expect("size")));
                    for axis in ["x", "y"] {
                        assert!(item[axis].as_f64().expect(axis).is_finite());
                    }
                    if k == "text" {
                        let s = item["s"].as_str().expect("the string");
                        assert!(!s.is_empty() && s.chars().count() <= MAX_TEXT, "{s:?}");
                    }
                }
                other => panic!("unknown item kind {other}"),
            }
        }
    }

    fn texts(items: &[Value]) -> Vec<String> {
        items
            .iter()
            .filter(|i| i["k"] == "text")
            .map(|i| i["s"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn icons(items: &[Value]) -> Vec<String> {
        items
            .iter()
            .filter(|i| i["k"] == "icon")
            .map(|i| i["n"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn before_the_first_payload_the_app_waits_with_a_bus_on_screen() {
        let items = screen(None, 0);
        check(&items);
        assert_eq!(icons(&items), ["bus"], "the bus icon, and no header yet");
        assert!(texts(&items).iter().any(|s| s == "Waiting for the bus..."));
        assert!(!texts(&items).iter().any(|s| s == "No departures"));
    }

    #[test]
    fn an_empty_board_says_so_under_the_stop_name() {
        let board = Board {
            stop: "Gare Centrale".to_string(),
            departures: Vec::new(),
        };
        let items = screen(Some(&board), 0);
        check(&items);
        assert!(texts(&items).iter().any(|s| s == "Gare Centrale"));
        assert!(texts(&items).iter().any(|s| s == "No departures"));
        assert!(!texts(&items).iter().any(|s| s == "Waiting for the bus..."));
        // The header's bus, and the one over "No departures".
        assert_eq!(icons(&items), ["bus", "bus"]);

        // A payload with no stop name still has a header.
        let items = screen(Some(&Board::default()), 0);
        assert!(texts(&items).iter().any(|s| s == "DEPARTURES"));
    }

    #[test]
    fn a_page_is_three_rows_a_bus_each_and_the_time_on_the_right() {
        let board = board_of(5);
        let items = screen(Some(&board), 0);
        check(&items);
        assert_eq!(icons(&items).len(), 1 + PER_PAGE, "the header's bus, and one per row");
        assert_eq!(
            items.iter().filter(|i| i["k"] == "icon" && i["x"] == 44.0).count(),
            PER_PAGE
        );
        for route in ["50", "51", "52"] {
            assert!(texts(&items).iter().any(|s| s == route), "{route} is on page 1");
        }
        assert!(!texts(&items).iter().any(|s| s == "53"), "the fourth is not");
        assert!(texts(&items).iter().any(|s| s == "now"), "a bus that is due says so");
        assert!(texts(&items).iter().any(|s| s == "2 min"));
        assert!(texts(&items).iter().any(|s| s == "1/2"), "and it says which page");

        // The right-hand column is right-aligned, teal and bold.
        let when = items.iter().find(|i| i["s"] == "2 min").expect("the minutes");
        assert_eq!((when["a"].as_str(), when["w"].as_str()), (Some("end"), Some("bold")));
        assert_eq!(when["x"], 276.0);
        assert_eq!(when["c"], TEAL);
    }

    #[test]
    fn the_last_page_is_short_and_the_dividers_only_go_between_rows() {
        let board = board_of(4);
        let items = screen(Some(&board), 1);
        check(&items);
        assert_eq!(icons(&items).len(), 2, "the header's bus, and one row");
        assert!(texts(&items).iter().any(|s| s == "53"));
        assert!(texts(&items).iter().any(|s| s == "2/2"));
        assert_eq!(
            items.iter().filter(|i| i["k"] == "stroke" && i["w"] == 1.0).count(),
            1,
            "the header rule alone"
        );

        // Two dividers between three rows, plus the header's rule.
        let items = screen(Some(&board), 0);
        assert_eq!(items.iter().filter(|i| i["k"] == "stroke" && i["w"] == 1.0).count(), 3);

        // One page of three: no page number at all.
        let items = screen(Some(&board_of(3)), 0);
        assert!(!texts(&items).iter().any(|s| s.contains('/')));
    }

    #[test]
    fn a_page_number_out_of_range_wraps_rather_than_showing_an_empty_screen() {
        let board = board_of(4);
        assert_eq!(screen(Some(&board), 7), screen(Some(&board), 1));
        assert!(texts(&screen(Some(&board), 9)).iter().any(|s| s == "2/2"));
    }

    #[test]
    fn long_names_are_cut_and_missing_fields_are_left_out() {
        let board = Board {
            stop: "Gare Centrale de Montréal".to_string(),
            departures: vec![
                departure("55", Some(3.0)),
                Departure {
                    route: "?".to_string(),
                    headsign: "A headsign far longer than the row it has to fit in".to_string(),
                    minutes: None,
                    time: Some("14:32".to_string()),
                },
                Departure::default(),
            ],
        };
        let items = screen(Some(&board), 0);
        check(&items);
        assert_eq!(cut(&board.stop, MAX_STOP), "Gare Centrale de M..");
        assert!(texts(&items).iter().any(|s| s == "Gare Centrale de M.."));
        assert!(
            texts(&items).iter().any(|s| s == "A headsign far longer th.."),
            "{:?}",
            texts(&items)
        );
        assert!(texts(&items).iter().any(|s| s == "14:32"), "the clock time stands in");
        assert_eq!(
            texts(&items).iter().filter(|s| *s == "?").count(),
            2,
            "a nameless bus still leaves, and still has a row"
        );
    }

    #[test]
    fn every_page_of_a_long_board_is_a_well_formed_scene() {
        for count in 0..12 {
            for page in 0..3 {
                let items = screen(Some(&board_of(count)), page);
                check(&items);
                assert!(items.len() < 40, "a board is a small scene: {}", items.len());
            }
        }
        check(&screen(None, 0));
    }
}
