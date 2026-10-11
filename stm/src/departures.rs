//! The payload Home Assistant pushes, and what a departure board makes of it.
//!
//! ```json
//! {"stop": "Gare Centrale", "departures": [{"route": "55", "headsign": "Saint-Laurent",
//! "minutes": 3, "time": "14:32"}]}
//! ```
//!
//! Nothing here trusts the payload. Home Assistant is the one talking to the transit API, and this
//! app is the last link in that chain: a missing field, a string where a number belongs, a list of
//! something other than objects, or no object at all each end in a board that still draws
//! something sane. The app never fetches anything itself.

use serde_json::Value;

/// How many departures are shown on one page.
pub const PER_PAGE: usize = 3;

/// The longest stop name drawn before it is cut, in characters.
pub const MAX_STOP: usize = 20;

/// The longest headsign drawn before it is cut, in characters.
pub const MAX_HEADSIGN: usize = 26;

/// One bus leaving a stop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Departure {
    /// The line number as the transit agency prints it, e.g. `55`.
    pub route: String,
    /// Where it is going, e.g. `Saint-Laurent`.
    pub headsign: String,
    /// Minutes until it leaves, when the payload has them.
    pub minutes: Option<f64>,
    /// The clock time it leaves, when the payload has that instead.
    pub time: Option<String>,
}

impl Departure {
    /// What the right-hand column says: how long, or when.
    pub fn when(&self) -> String {
        match self.minutes {
            // Due, or overdue: the board the transit agency has says the same.
            Some(m) if m <= 0.0 => "now".to_string(),
            Some(m) if m.fract() == 0.0 => format!("{:.0} min", m.clamp(0.0, 999.0)),
            Some(m) => format!("{:.1} min", m.clamp(0.0, 999.0)),
            None => self.time.clone().unwrap_or_default(),
        }
    }
}

/// A stop and the departures Home Assistant last reported for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    /// The stop's name.
    pub stop: String,
    /// In the order Home Assistant sent them, soonest first.
    pub departures: Vec<Departure>,
}

impl Board {
    /// Read a payload, keeping whatever can be kept.
    pub fn parse(data: &Value) -> Board {
        let stop = data.get("stop").and_then(text).unwrap_or_default();
        let departures = data
            .get("departures")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Departure::parse).collect())
            .unwrap_or_default();
        Board { stop, departures }
    }

    /// How many pages of [`PER_PAGE`] departures there are, at least one.
    pub fn pages(&self) -> usize {
        self.departures.len().div_ceil(PER_PAGE).max(1)
    }

    /// One page of departures; an empty slice for a page that does not exist.
    pub fn page(&self, page: usize) -> &[Departure] {
        self.departures.chunks(PER_PAGE).nth(page).unwrap_or(&[])
    }
}

impl Departure {
    /// One element of the `departures` list; `None` for anything that is not an object.
    fn parse(value: &Value) -> Option<Departure> {
        value.as_object()?;
        Some(Departure {
            route: value.get("route").and_then(text).unwrap_or_else(|| "?".to_string()),
            headsign: value.get("headsign").and_then(text).unwrap_or_default(),
            minutes: value.get("minutes").and_then(number),
            time: value.get("time").and_then(text),
        })
    }
}

/// A string, or the number printed as one — a route is `"55"` about as often as it is `55`.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A number, or a string that reads as one: `3` and `"3"` mean the same three minutes.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// `s`, cut to `max` characters with `..` when it had to be — the Nest clips anything wider than
/// the canvas, and a cut string reads better than a clipped one.
pub fn cut(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(2)).collect();
    format!("{}..", head.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn departure(route: &str, minutes: Option<f64>) -> Departure {
        Departure {
            route: route.to_string(),
            headsign: format!("{route} somewhere"),
            minutes,
            time: None,
        }
    }

    fn board(count: usize) -> Board {
        Board {
            stop: "Gare Centrale".to_string(),
            departures: (0..count)
                .map(|i| departure(&format!("5{i}"), Some(i as f64)))
                .collect(),
        }
    }

    #[test]
    fn a_full_payload_reads_back_as_sent() {
        let payload = json!({
            "stop": "Gare Centrale",
            "departures": [
                {"route": "55", "headsign": "Saint-Laurent", "minutes": 3, "time": "14:32"},
                {"route": "165", "headsign": "Côte-des-Neiges", "minutes": 11, "time": "14:40"},
            ],
            "updated": "2026-01-01T14:29:00Z"
        });
        let parsed = Board::parse(&payload);
        assert_eq!(parsed.stop, "Gare Centrale");
        assert_eq!(parsed.departures.len(), 2);
        assert_eq!(
            parsed.departures[0],
            Departure {
                route: "55".to_string(),
                headsign: "Saint-Laurent".to_string(),
                minutes: Some(3.0),
                time: Some("14:32".to_string()),
            }
        );
        // An extra field is not an error.
        assert_eq!(parsed.departures[1].route, "165");
        assert_eq!(parsed.departures[0].when(), "3 min");
    }

    #[test]
    fn every_field_may_be_missing() {
        let parsed = Board::parse(&json!({"departures": [{}]}));
        assert_eq!(parsed.stop, "");
        assert_eq!(parsed.departures[0].route, "?", "a bus with no number still leaves");
        assert_eq!(parsed.departures[0].headsign, "");
        assert_eq!(parsed.departures[0].minutes, None);
        assert_eq!(parsed.departures[0].when(), "", "and it says when only if it can");

        // Numbers may arrive as strings, and `minutes` may be a float.
        let parsed = Board::parse(&json!({"stop": 55, "departures": [{"route": 55, "minutes": "3.5"}]}));
        assert_eq!(parsed.stop, "55");
        assert_eq!(parsed.departures[0].route, "55");
        assert_eq!(parsed.departures[0].minutes, Some(3.5));
        assert_eq!(parsed.departures[0].when(), "3.5 min");

        // A payload with no minutes falls back to the clock time.
        let parsed = Board::parse(&json!({"departures": [{"route": "55", "time": "14:32"}]}));
        assert_eq!(parsed.departures[0].when(), "14:32");
        // And a minutes that is neither a number nor a number-shaped string is no value at all.
        let parsed = Board::parse(&json!({"departures": [{"minutes": "soon", "time": "14:32"}]}));
        assert_eq!(parsed.departures[0].when(), "14:32");
    }

    #[test]
    fn nonsense_payloads_read_as_an_empty_board_rather_than_a_crash() {
        for payload in [
            json!(null),
            json!("Gare Centrale"),
            json!(42),
            json!([]),
            json!({"departures": "none"}),
            json!({"departures": {}}),
            json!({"stop": {"name": "Gare Centrale"}}),
            json!({"stop": ["Gare Centrale"]}),
            json!({}),
        ] {
            let parsed = Board::parse(&payload);
            assert_eq!(parsed.stop, "", "{payload}");
            assert!(parsed.departures.is_empty(), "{payload}");
            assert_eq!(parsed.pages(), 1, "an empty board is still one page");
            assert!(parsed.page(0).is_empty());
        }

        // A list of things that are not departures keeps only the ones that are.
        let parsed = Board::parse(&json!({"departures": ["55", {"route": "55"}, 7, null]}));
        assert_eq!(parsed.departures.len(), 1);
        assert_eq!(parsed.departures[0].route, "55");
    }

    #[test]
    fn when_says_how_long_and_never_nonsense() {
        let cases = [
            (Some(0.0), "now"),
            (Some(-2.0), "now"),
            (Some(3.0), "3 min"),
            (Some(3.5), "3.5 min"),
            (Some(1.0), "1 min"),
            (Some(999.0), "999 min"),
            (Some(1.0e9), "999 min"),
        ];
        for (minutes, expected) in cases {
            let d = Departure {
                minutes,
                ..Departure::default()
            };
            assert_eq!(d.when(), expected, "{minutes:?}");
        }
    }

    #[test]
    fn paging_hands_out_every_departure_once() {
        assert_eq!(board(0).pages(), 1);
        assert_eq!(board(1).pages(), 1);
        assert_eq!(board(3).pages(), 1);
        assert_eq!(board(4).pages(), 2);
        assert_eq!(board(7).pages(), 3);

        assert_eq!(board(7).page(0).len(), PER_PAGE);
        assert_eq!(board(7).page(1).len(), PER_PAGE);
        assert_eq!(board(7).page(2).len(), 1);
        assert!(board(7).page(3).is_empty(), "off the end is empty, not a panic");
        assert_eq!(board(4).page(1)[0].route, "53");
    }

    #[test]
    fn cutting_a_string_keeps_it_readable() {
        assert_eq!(cut("Gare Centrale", MAX_STOP), "Gare Centrale");
        assert_eq!(cut("  Gare Centrale  ", MAX_STOP), "Gare Centrale");
        assert_eq!(cut("Gare Centrale de Montréal", MAX_STOP), "Gare Centrale de M..");
        assert_eq!(cut("", MAX_STOP), "");
        assert_eq!(cut("abcdef", 6), "abcdef");
        assert_eq!(cut("abcdefg", 6), "abcd..");
        for max in [4usize, 5, 20] {
            assert!(cut("Côte-des-Neiges", max).chars().count() <= max, "{max}");
        }
    }
}
