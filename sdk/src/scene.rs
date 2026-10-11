//! Scene items: what the Nest draws.
//!
//! A scene is a list of at most [`MAX_ITEMS`](crate::MAX_ITEMS) items on the 320×320 canvas: a
//! filled path, a stroked path, a text run or a Material Design Icon. A scene *replaces* the
//! whole screen, so an app that animates simply builds a new one and sends it again.
//!
//! [`fill`], [`stroke`], [`text`] and [`icon`] return the JSON objects the wire carries. They
//! sanitise what they can — a size outside 4..120 is clamped, a text longer than 200 characters is
//! cut, a `NaN` coordinate becomes 0 — so that one odd value costs an item's looks, not the
//! scene's validity. Defaults are left out of the object (`w: regular`, `a: middle`, `t: 0`,
//! `r: false`) to keep scenes small.
//!
//! ```
//! use nest_app_sdk::{path, scene, scene::{rgb, Anchor, TextStyle}};
//! use serde_json::json;
//!
//! let items = vec![
//!     scene::fill(path::ring(160.0, 160.0, 150.0, 140.0, 10.0, 70.0), rgb(0x7c, 0xd6, 0xc9)),
//!     scene::stroke(path::arc(160.0, 160.0, 100.0, 0.0, 90.0), "#ffffff", 6.0, true),
//!     scene::text("55", 160.0, 175.0, 64.0, "#ffffff", TextStyle::new().bold().anchor(Anchor::Middle)),
//!     scene::icon("mdi-bus", 160.0, 60.0, 32.0, "#7cd6c9"),
//! ];
//! assert_eq!(items.len(), 4);
//! ```

use serde_json::{Value, json};

use crate::{MAX_SIZE, MAX_STROKE, MAX_TEXT, MIN_SIZE};

/// An opaque `#rrggbb` colour from its channels.
///
/// The wire accepts nothing else: a malformed colour drops that item.
pub fn rgb(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Text weight; `Regular` is the wire default and is left out of the item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Weight {
    /// The device font's regular weight.
    #[default]
    Regular,
    /// The device font's bold weight.
    Bold,
}

/// Where (`x`, `y`) sits in a text run; `Middle` is the wire default and is left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    /// (`x`, `y`) is the start of the run.
    Start,
    /// (`x`, `y`) is the middle of the run.
    #[default]
    Middle,
    /// (`x`, `y`) is the end of the run.
    End,
}

/// A text run's style: weight, anchor and letter tracking.
///
/// ```
/// use nest_app_sdk::scene::{Anchor, TextStyle, Weight};
/// let st = TextStyle::new().bold().anchor(Anchor::Start).tracking(2.0);
/// assert_eq!(st.weight, Weight::Bold);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextStyle {
    /// Regular or bold.
    pub weight: Weight,
    /// Start, middle or end.
    pub anchor: Anchor,
    /// Letter tracking, in em.
    pub tracking: f32,
}

impl TextStyle {
    /// Regular weight, middle anchor, no tracking.
    pub const fn new() -> Self {
        Self {
            weight: Weight::Regular,
            anchor: Anchor::Middle,
            tracking: 0.0,
        }
    }

    /// Bold.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.weight = Weight::Bold;
        self
    }

    /// Anchor the run at `anchor`.
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Track the letters by `em` (0 means the font's own spacing).
    #[must_use]
    pub const fn tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }
}

/// A finite number, or 0.
fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// A number as the wire carries it: three decimals, so that an `f32` on its way to JSON's `f64`
/// does not put `2.4000000953674316` in a scene where `2.4` belongs.
fn wire(v: f32) -> f64 {
    (f64::from(finite(v)) * 1000.0).round() / 1000.0
}

/// A font size inside the wire's 4..120; anything that is not a number becomes the smallest size.
fn size(v: f32) -> f64 {
    wire(finite(v).clamp(MIN_SIZE, MAX_SIZE))
}

/// A filled path: `{"k": "fill", "c": …, "d": …}`.
pub fn fill(d: impl AsRef<str>, color: impl AsRef<str>) -> Value {
    json!({"k": "fill", "c": color.as_ref(), "d": d.as_ref()})
}

/// A stroked path: `{"k": "stroke", "c": …, "d": …, "w": …, "r": …}`.
///
/// `round` asks for round caps, which is what makes a stroked arc look like the Nest's own ring
/// segments. `w` is clamped to 0..64.
pub fn stroke(d: impl AsRef<str>, color: impl AsRef<str>, width: f32, round: bool) -> Value {
    let mut item = json!({
        "k": "stroke",
        "c": color.as_ref(),
        "d": d.as_ref(),
        "w": wire(width.clamp(0.0, MAX_STROKE)),
    });
    if round {
        item["r"] = json!(true);
    }
    item
}

/// A text run: `{"k": "text", "c": …, "s": …, "x": …, "y": …, "size": …}`.
///
/// The Nest outlines the text with the device font, so an app ships no fonts and cannot embed one.
/// `s` is cut to 200 characters and `size` clamped to 4..120.
pub fn text(s: &str, x: f32, y: f32, size_px: f32, color: impl AsRef<str>, style: TextStyle) -> Value {
    let s: String = s.chars().take(MAX_TEXT).collect();
    let mut item = json!({
        "k": "text",
        "c": color.as_ref(),
        "s": s,
        "x": wire(x),
        "y": wire(y),
        "size": size(size_px),
    });
    if style.weight == Weight::Bold {
        item["w"] = json!("bold");
    }
    match style.anchor {
        Anchor::Start => item["a"] = json!("start"),
        Anchor::End => item["a"] = json!("end"),
        Anchor::Middle => {}
    }
    let tracking = wire(style.tracking);
    if tracking != 0.0 {
        item["t"] = json!(tracking);
    }
    item
}

/// A Material Design Icon: `{"k": "icon", "c": …, "n": …, "x": …, "y": …, "size": …}`.
///
/// `name` may carry the `mdi-` prefix or not; the prefix is dropped, so either spelling reaches the
/// same icon. An unknown name draws nothing (the manifest's `icon` falls back to the lightbulb
/// instead).
pub fn icon(name: &str, x: f32, y: f32, size_px: f32, color: impl AsRef<str>) -> Value {
    json!({
        "k": "icon",
        "c": color.as_ref(),
        "n": name.strip_prefix("mdi-").unwrap_or(name),
        "x": wire(x),
        "y": wire(y),
        "size": size(size_px),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_STROKE, MAX_TEXT};

    fn keys(v: &Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().expect("an object").keys().cloned().collect();
        k.sort();
        k
    }

    fn assert_color(v: &Value) {
        let c = v["c"].as_str().expect("a colour");
        assert_eq!(c.len(), 7, "{c} is not #rrggbb");
        assert!(c.starts_with('#'));
        assert!(c[1..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn rgb_writes_six_lower_hex_digits() {
        assert_eq!(rgb(0, 0, 0), "#000000");
        assert_eq!(rgb(0x7c, 0xd6, 0xc9), "#7cd6c9");
        assert_eq!(rgb(255, 255, 255), "#ffffff");
    }

    #[test]
    fn fill_and_stroke_are_shaped_as_the_spec_says() {
        let f = fill("M0 0L1 1Z", "#ffffff");
        assert_eq!(keys(&f), ["c", "d", "k"]);
        assert_eq!(f["k"], "fill");
        assert_color(&f);

        let s = stroke("M0 0L1 1", "#7cd6c9", 2.5, true);
        assert_eq!(keys(&s), ["c", "d", "k", "r", "w"]);
        assert_eq!(s["k"], "stroke");
        assert_eq!(s["w"], 2.5);
        assert_eq!(s["r"], true);
        assert_color(&s);

        // Butt caps are the default and stay off the wire.
        let s = stroke("M0 0L1 1", "#ffffff", 1.0, false);
        assert_eq!(keys(&s), ["c", "d", "k", "w"]);
        assert_eq!(s.get("r"), None);
    }

    #[test]
    fn stroke_width_is_clamped() {
        assert_eq!(stroke("M0 0", "#ffffff", 900.0, false)["w"], MAX_STROKE);
        assert_eq!(stroke("M0 0", "#ffffff", -4.0, false)["w"], 0.0);
        assert_eq!(stroke("M0 0", "#ffffff", f32::NAN, false)["w"], 0.0);
    }

    #[test]
    fn text_carries_only_what_it_needs() {
        let t = text("64°", 160.0, 140.0, 40.0, "#ffffff", TextStyle::new());
        assert_eq!(keys(&t), ["c", "k", "s", "size", "x", "y"]);
        assert_eq!(t["s"], "64°");
        assert_eq!(t["size"], 40.0);
        assert_color(&t);

        let t = text(
            "HI",
            1.0,
            2.0,
            12.0,
            "#ffffff",
            TextStyle::new().bold().anchor(Anchor::End).tracking(1.5),
        );
        assert_eq!(keys(&t), ["a", "c", "k", "s", "size", "t", "w", "x", "y"]);
        assert_eq!(t["w"], "bold");
        assert_eq!(t["a"], "end");
        assert_eq!(t["t"], 1.5);

        assert_eq!(
            text("HI", 1.0, 2.0, 12.0, "#ffffff", TextStyle::new().anchor(Anchor::Start))["a"],
            "start"
        );
    }

    #[test]
    fn text_and_sizes_are_clamped_not_rejected() {
        let long = "é".repeat(500);
        let t = text(&long, 0.0, 0.0, 12.0, "#ffffff", TextStyle::new());
        assert_eq!(t["s"].as_str().expect("a string").chars().count(), MAX_TEXT);

        assert_eq!(
            text("x", 0.0, 0.0, 1000.0, "#ffffff", TextStyle::new())["size"],
            MAX_SIZE
        );
        assert_eq!(text("x", 0.0, 0.0, 1.0, "#ffffff", TextStyle::new())["size"], MIN_SIZE);
        assert_eq!(
            text("x", 0.0, 0.0, f32::NAN, "#ffffff", TextStyle::new())["size"],
            MIN_SIZE
        );
        let t = text("x", f32::INFINITY, f32::NAN, 12.0, "#ffffff", TextStyle::new());
        assert_eq!((t["x"].as_f64(), t["y"].as_f64()), (Some(0.0), Some(0.0)));
    }

    #[test]
    fn numbers_read_the_way_they_were_written() {
        // An `f32` on its way onto the wire must not grow a tail of `f64` digits.
        let t = text(
            "HI",
            160.0,
            46.0,
            15.0,
            "#ffffff",
            TextStyle::new().bold().tracking(2.4),
        );
        let rendered = serde_json::to_string(&t).expect("json");
        assert!(rendered.contains("\"t\":2.4"), "{rendered}");
        assert!(rendered.contains("\"size\":15.0"), "{rendered}");
        assert!(rendered.len() < 100, "{rendered}");
        assert_eq!(stroke("M0 0", "#ffffff", 2.4, true)["w"], 2.4);
        assert_eq!(icon("eye", 0.1, 0.2, 12.0, "#ffffff")["x"], 0.1);
    }

    #[test]
    fn icon_names_normalise_and_sizes_clamp() {
        let i = icon("mdi-bus", 160.0, 60.0, 32.0, "#7cd6c9");
        assert_eq!(keys(&i), ["c", "k", "n", "size", "x", "y"]);
        assert_eq!(i["k"], "icon");
        assert_eq!(i["n"], "bus");
        assert_eq!(i["size"], 32.0);
        assert_color(&i);

        assert_eq!(icon("bus", 0.0, 0.0, 8.0, "#ffffff")["n"], "bus");
        assert_eq!(icon("mdi-bus", 0.0, 0.0, 4.0, "#ffffff")["size"], MIN_SIZE);
    }

    #[test]
    fn a_style_reads_like_a_builder() {
        let st = TextStyle::default();
        assert_eq!(st, TextStyle::new());
        let st = st.bold().anchor(Anchor::Start).tracking(2.0);
        assert_eq!(st.weight, Weight::Bold);
        assert_eq!(st.anchor, Anchor::Start);
        assert_eq!(st.tracking, 2.0);
    }
}
