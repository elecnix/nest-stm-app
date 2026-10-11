//! Path data for scene items: absolute coordinates on the 320×320 canvas.
//!
//! The Nest's parser accepts exactly five commands — `M`, `L`, `Q`, `C` and `Z` — with absolute
//! coordinates, so every builder here emits that subset and nothing else. Angles follow the
//! Nest's own convention: **0° is 12 o'clock and angles increase clockwise**, which is how its
//! dials, rings and carousels are laid out. In canvas coordinates (y grows downwards) that means
//!
//! ```text
//!             0°
//!        270°  ⊕  90°
//!            180°
//! ```
//!
//! Numbers are written with two decimals, like the Nest writes its own paths; a value that is not
//! finite is written as `0` so that a NaN can never reach the wire.

use std::fmt::Write as _;

use crate::MAX_SEGS;

/// The point at `deg` degrees clockwise from 12 o'clock, `r` px from (`cx`, `cy`).
///
/// Useful for placing ticks, labels and arrowheads around a dial.
pub fn point_at(cx: f32, cy: f32, r: f32, deg: f32) -> (f32, f32) {
    let a = deg.to_radians();
    (cx + r * a.sin(), cy - r * a.cos())
}

/// `v` as path data writes it: two decimals, never `NaN` or `inf`.
fn num(v: f32) -> String {
    format!("{:.2}", if v.is_finite() { v } else { 0.0 })
}

/// A path under construction, in absolute canvas coordinates.
///
/// The builder caps itself at [`MAX_SEGS`](crate::MAX_SEGS) segments: once the path holds that
/// many, further `*_to` and `close` calls write nothing, and a path that would have been longer
/// comes out truncated. The Nest drops a whole item whose path is over the cap, so a truncated
/// path still draws while an over-long one draws nothing at all.
///
/// ```
/// use nest_app_sdk::path::Path;
/// let d = Path::at(40.0, 40.0).line_to(280.0, 40.0).quad_to(300.0, 60.0, 280.0, 80.0).into_string();
/// assert!(d.starts_with("M40.00 40.00L280.00 40.00Q"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Path {
    d: String,
    /// Segments written so far, so that the builder can stop at the cap without
    /// reading `d` back.
    segs: usize,
}

impl Path {
    /// An empty path; every `*_to` method appends a segment.
    pub fn new() -> Self {
        Self::default()
    }

    /// A path starting at (`x`, `y`) — the `M` segment every path needs first.
    pub fn at(x: f32, y: f32) -> Self {
        Self {
            d: format!("M{} {}", num(x), num(y)),
            segs: 1,
        }
    }

    /// Whether the path has reached the cap: nothing more is written to it.
    fn full(&self) -> bool {
        self.segs >= MAX_SEGS
    }

    /// Append a line to (`x`, `y`).
    #[must_use]
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        if self.full() {
            return self;
        }
        let _ = write!(self.d, "L{} {}", num(x), num(y));
        self.segs += 1;
        self
    }

    /// Append a quadratic curve to (`x`, `y`) with control point (`cx`, `cy`).
    #[must_use]
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        if self.full() {
            return self;
        }
        let _ = write!(self.d, "Q{} {} {} {}", num(cx), num(cy), num(x), num(y));
        self.segs += 1;
        self
    }

    /// Append a cubic curve to (`x`, `y`) with control points (`ax`, `ay`) and (`bx`, `by`).
    #[must_use]
    pub fn cubic_to(mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) -> Self {
        if self.full() {
            return self;
        }
        let _ = write!(
            self.d,
            "C{} {} {} {} {} {}",
            num(ax),
            num(ay),
            num(bx),
            num(by),
            num(x),
            num(y)
        );
        self.segs += 1;
        self
    }

    /// Append the arc from `a0` to `a1` degrees clockwise from 12 o'clock, as cubic segments of
    /// at most 90° each, and leave the current point at `a1` (or at the last segment written, if
    /// the arc runs into the segment cap).
    ///
    /// The current point must already be `point_at(cx, cy, r, a0)` — every builder in this module
    /// does that for you, and so should yours.
    #[must_use]
    pub fn arc_to(mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Self {
        let span = if a1.is_finite() { a1 - a0 } else { 0.0 };
        let n = ((span.abs() / 90.0).ceil() as usize).max(1);
        let step = span.to_radians() / n as f32;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let pt = |a: f32| point_at(cx, cy, r, a.to_degrees());
        let tan = |a: f32| (r * a.cos(), r * a.sin()); // d/da of pt, in canvas coordinates
        let mut a = a0.to_radians();
        for _ in 0..n {
            if self.full() {
                break;
            }
            let b = a + step;
            let ((x0, y0), (x1, y1)) = (pt(a), pt(b));
            let ((dx0, dy0), (dx1, dy1)) = (tan(a), tan(b));
            let _ = write!(
                self.d,
                "C{} {} {} {} {} {}",
                num(x0 + k * dx0),
                num(y0 + k * dy0),
                num(x1 - k * dx1),
                num(y1 - k * dy1),
                num(x1),
                num(y1)
            );
            self.segs += 1;
            a = b;
        }
        self
    }

    /// Close the path back to its first point.
    #[must_use]
    pub fn close(mut self) -> Self {
        if self.full() {
            return self;
        }
        self.d.push('Z');
        self.segs += 1;
        self
    }

    /// The path data.
    pub fn as_str(&self) -> &str {
        &self.d
    }

    /// The path data, owned.
    pub fn into_string(self) -> String {
        self.d
    }
}

impl From<Path> for String {
    fn from(p: Path) -> Self {
        p.d
    }
}

impl From<String> for Path {
    fn from(d: String) -> Self {
        // Counted once on the way in: one segment per command letter, and this module
        // writes nothing else, so a path that came in as data still knows how full it is.
        let segs = d
            .bytes()
            .filter(|b| matches!(b, b'M' | b'L' | b'Q' | b'C' | b'Z'))
            .count();
        Self { d, segs }
    }
}

/// A move to (`x`, `y`): a path with a single point, which fills and strokes as nothing.
pub fn move_to(x: f32, y: f32) -> String {
    Path::at(x, y).into_string()
}

/// A straight line from (`x1`, `y1`) to (`x2`, `y2`).
pub fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> String {
    Path::at(x1, y1).line_to(x2, y2).into_string()
}

/// An open polyline through `pts`; nothing is drawn if there are fewer than two points. The
/// builder caps itself at [`MAX_SEGS`](crate::MAX_SEGS) segments, so a longer run of points comes
/// out truncated rather than over the cap (an over-long path draws nothing at all).
pub fn polyline(pts: &[(f32, f32)]) -> String {
    let mut p = Path::new();
    for (i, &(x, y)) in pts.iter().enumerate() {
        p = if i == 0 { Path::at(x, y) } else { p.line_to(x, y) };
    }
    p.into_string()
}

/// A closed polygon through `pts`; nothing is drawn if there are fewer than three points.
pub fn polygon(pts: &[(f32, f32)]) -> String {
    match pts.len() {
        0..=2 => String::new(),
        _ => Path::from(polyline(pts)).close().into_string(),
    }
}

/// A circle of radius `r` centred on (`cx`, `cy`), as four cubic quarter arcs.
pub fn circle(cx: f32, cy: f32, r: f32) -> String {
    let (x, y) = point_at(cx, cy, r, 0.0);
    Path::at(x, y).arc_to(cx, cy, r, 0.0, 360.0).close().into_string()
}

/// An open arc of radius `r` from `a0` to `a1` degrees clockwise from 12 o'clock.
///
/// Stroke it with round caps to get the ring segments the Nest's own dials are made of.
pub fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> String {
    let (x, y) = point_at(cx, cy, r, a0);
    Path::at(x, y).arc_to(cx, cy, r, a0, a1).into_string()
}

/// A closed annulus segment between `r_outer` and `r_inner` from `a0` to `a1` degrees clockwise
/// from 12 o'clock: a filled ring arc, with the inner edge traced back the other way so the hole
/// comes out hollow under a non-zero fill.
pub fn ring(cx: f32, cy: f32, r_outer: f32, r_inner: f32, a0: f32, a1: f32) -> String {
    let (x, y) = point_at(cx, cy, r_outer, a0);
    let (ix, iy) = point_at(cx, cy, r_inner, a1);
    Path::at(x, y)
        .arc_to(cx, cy, r_outer, a0, a1)
        .line_to(ix, iy)
        .arc_to(cx, cy, r_inner, a1, a0)
        .close()
        .into_string()
}

/// A rectangle at (`x`, `y`) of `w`×`h`, with corners rounded by `rx` (clamped to half the sides).
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, rx: f32) -> String {
    let r = rx.max(0.0).min(w.abs() / 2.0).min(h.abs() / 2.0);
    let corners = [
        (x + w - r, y + r, 0.0),
        (x + w - r, y + h - r, 90.0),
        (x + r, y + h - r, 180.0),
        (x + r, y + r, 270.0),
    ];
    let next = [(x + w, y + h - r), (x + r, y + h), (x, y + r), (x + r, y)];
    let mut p = Path::at(x + r, y).line_to(x + w - r, y);
    for (i, &(cx, cy, a)) in corners.iter().enumerate() {
        p = p.arc_to(cx, cy, r, a, a + 90.0).line_to(next[i].0, next[i].1);
    }
    p.close().into_string()
}

/// A single quadratic curve from (`x0`, `y0`) to (`x1`, `y1`) bending towards (`cx`, `cy`).
pub fn quad(x0: f32, y0: f32, cx: f32, cy: f32, x1: f32, y1: f32) -> String {
    Path::at(x0, y0).quad_to(cx, cy, x1, y1).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_SEGS;

    /// A stand-in for the Nest's parser: absolute `M/L/Q/C/Z` only, arity checked, and every
    /// number finite. Panics — in a test — on anything else.
    fn parse(d: &str) -> Vec<(char, Vec<f32>)> {
        let mut tok = String::new();
        for c in d.chars() {
            match c {
                ',' => tok.push(' '),
                c if c.is_ascii_alphabetic() => {
                    tok.push(' ');
                    tok.push(c);
                    tok.push(' ');
                }
                c => tok.push(c),
            }
        }
        let tok: Vec<&str> = tok.split_whitespace().collect();
        let mut segs = Vec::new();
        let mut i = 0;
        while i < tok.len() {
            let (cmd, n) = match tok[i] {
                "M" => ('M', 2),
                "L" => ('L', 2),
                "Q" => ('Q', 4),
                "C" => ('C', 6),
                "Z" => ('Z', 0),
                other => panic!("bad command {other:?} in {d:?}"),
            };
            let cmd = if cmd == 'M' && !segs.is_empty() { 'L' } else { cmd };
            let mut args = Vec::new();
            for k in 1..=n {
                let v: f32 = tok[i + k]
                    .parse()
                    .unwrap_or_else(|e| panic!("bad number in {d:?}: {e}"));
                assert!(v.is_finite(), "not finite in {d:?}");
                args.push(v);
            }
            i += n + 1;
            segs.push((cmd, args));
        }
        segs
    }

    fn assert_valid(d: &str) -> Vec<(char, Vec<f32>)> {
        let segs = parse(d);
        assert!(!segs.is_empty(), "empty path");
        assert_eq!(segs[0].0, 'M', "path must start with a move: {d}");
        assert!(segs.len() <= MAX_SEGS, "too many segments: {}", segs.len());
        segs
    }

    fn close_to(a: (f32, f32), b: (f32, f32)) {
        assert!((a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05, "{a:?} != {b:?}");
    }

    #[test]
    fn angles_start_at_twelve_and_go_clockwise() {
        close_to(point_at(160.0, 160.0, 100.0, 0.0), (160.0, 60.0));
        close_to(point_at(160.0, 160.0, 100.0, 90.0), (260.0, 160.0));
        close_to(point_at(160.0, 160.0, 100.0, 180.0), (160.0, 260.0));
        close_to(point_at(160.0, 160.0, 100.0, 270.0), (60.0, 160.0));
    }

    #[test]
    fn builders_emit_absolute_commands_only() {
        let cases = [
            move_to(1.0, 2.0),
            line(1.0, 2.0, 3.0, 4.0),
            polyline(&[(0.0, 0.0), (10.0, 20.0), (30.0, 40.0)]),
            polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]),
            circle(160.0, 160.0, 100.0),
            arc(160.0, 160.0, 100.0, 30.0, 200.0),
            arc(160.0, 160.0, 100.0, 200.0, 30.0),
            ring(160.0, 160.0, 150.0, 120.0, 10.0, 70.0),
            rounded_rect(20.0, 20.0, 100.0, 40.0, 12.0),
            quad(20.0, 20.0, 160.0, 60.0, 300.0, 20.0),
        ];
        for d in &cases {
            assert_valid(d);
            for c in d.chars() {
                assert!(
                    matches!(c, 'M' | 'L' | 'Q' | 'C' | 'Z' | ' ' | '.' | '-' | '0'..='9'),
                    "unexpected {c:?} in {d}"
                );
            }
        }
        // Nothing is drawn for a degenerate polygon.
        assert_eq!(polygon(&[(0.0, 0.0), (1.0, 1.0)]), "");
    }

    #[test]
    fn circle_is_four_cubics_and_closed() {
        let segs = assert_valid(&circle(160.0, 160.0, 100.0));
        assert_eq!(segs.len(), 6, "a move, four quarter arcs and a close");
        assert_eq!(segs[0].0, 'M');
        assert!(segs[1..5].iter().all(|s| s.0 == 'C'));
        assert_eq!(segs[5].0, 'Z');
        // The move is due north of the centre, and the first cubic ends due east.
        close_to((segs[0].1[0], segs[0].1[1]), (160.0, 60.0));
        close_to((segs[1].1[4], segs[1].1[5]), (260.0, 160.0));
    }

    #[test]
    fn arc_spans_sixty_degrees_at_a_time() {
        // A 180° arc is two cubics, not four: the segments stay within 90°.
        assert_eq!(assert_valid(&arc(160.0, 160.0, 50.0, 0.0, 180.0)).len(), 3);
        assert_eq!(assert_valid(&arc(160.0, 160.0, 50.0, 0.0, 90.0)).len(), 2);
        assert_eq!(assert_valid(&arc(160.0, 160.0, 50.0, 0.0, 91.0)).len(), 3);
    }

    #[test]
    fn arc_direction_matters() {
        // A clockwise arc and its reverse start at their own a0 and sweep opposite ways.
        let (a, b) = (arc(0.0, 0.0, 10.0, 10.0, 80.0), arc(0.0, 0.0, 10.0, 80.0, 10.0));
        let sa = assert_valid(&a);
        close_to((sa[0].1[0], sa[0].1[1]), point_at(0.0, 0.0, 10.0, 10.0));
        let last = sa.last().expect("a segment");
        close_to(
            (last.1[last.1.len() - 2], last.1[last.1.len() - 1]),
            point_at(0.0, 0.0, 10.0, 80.0),
        );
        assert_ne!(a, b);
        assert_valid(&b);
    }

    #[test]
    fn ring_walks_out_and_back() {
        let segs = assert_valid(&ring(160.0, 160.0, 150.0, 120.0, 10.0, 190.0));
        assert_eq!(segs[0].0, 'M', "outer arc starts at its own move");
        assert_eq!(segs.last().expect("a segment").0, 'Z');
        // M, two cubics for the 180° outer arc, L to the inner edge, two cubics back, Z.
        assert_eq!(segs.len(), 7);
        close_to((segs[0].1[0], segs[0].1[1]), point_at(160.0, 160.0, 150.0, 10.0));
        let l = segs.iter().position(|s| s.0 == 'L').expect("line to the inner edge");
        close_to((segs[l].1[0], segs[l].1[1]), point_at(160.0, 160.0, 120.0, 190.0));
        let last = segs.last().expect("a segment");
        assert!(last.1.is_empty(), "Z carries no coordinates");
        close_to(
            (segs[segs.len() - 2].1[4], segs[segs.len() - 2].1[5]),
            point_at(160.0, 160.0, 120.0, 10.0),
        );
    }

    #[test]
    fn rounded_rect_clamps_its_radius() {
        let segs = assert_valid(&rounded_rect(0.0, 0.0, 100.0, 20.0, 999.0));
        // A 10 px radius on both sides: M, L, then four (arc, L) pairs, then Z.
        assert_eq!(segs.len(), 11);
        close_to((segs[0].1[0], segs[0].1[1]), (10.0, 0.0));
    }

    #[test]
    fn non_finite_numbers_never_reach_the_wire() {
        for d in [
            line(f32::NAN, 0.0, 1.0, f32::INFINITY),
            circle(f32::NAN, f32::NAN, 10.0),
            arc(0.0, 0.0, 10.0, f32::NAN, 90.0),
            quad(0.0, f32::NAN, 1.0, 1.0, 2.0, 2.0),
        ] {
            assert_valid(&d);
        }
    }

    #[test]
    fn the_builder_caps_itself_at_the_segment_limit() {
        // 5000 points: the builder stops at the cap, and what comes out still parses (the
        // Nest drops a whole item whose path is over it).
        let pts: Vec<(f32, f32)> = (0..5000).map(|i| (i as f32 % 320.0, i as f32 % 320.0)).collect();
        let segs = assert_valid(&polyline(&pts));
        assert_eq!(segs.len(), MAX_SEGS, "the move, then MAX_SEGS - 1 lines");
        // A polygon on top of the same points gets no close beyond the cap either.
        assert!(assert_valid(&polygon(&pts)).len() <= MAX_SEGS);
        // A path built by hand stops there too, whatever the command.
        let mut p = Path::at(0.0, 0.0);
        for _ in 0..MAX_SEGS {
            p = p.line_to(1.0, 1.0).quad_to(2.0, 2.0, 3.0, 3.0).close();
        }
        assert_eq!(assert_valid(&p.into_string()).len(), MAX_SEGS);
        // An arc that would run past the cap is cut short, not refused.
        let segs = assert_valid(&arc(160.0, 160.0, 100.0, 0.0, 400_000.0));
        assert_eq!(segs.len(), MAX_SEGS);
    }

    #[test]
    fn the_builder_chains() {
        let d = Path::at(10.0, 10.0)
            .line_to(20.0, 10.0)
            .quad_to(25.0, 15.0, 20.0, 20.0)
            .cubic_to(15.0, 25.0, 15.0, 25.0, 10.0, 20.0)
            .close()
            .into_string();
        let segs = assert_valid(&d);
        assert_eq!(segs.iter().map(|s| s.0).collect::<Vec<_>>(), ['M', 'L', 'Q', 'C', 'Z']);
        assert_eq!(String::from(Path::at(1.0, 2.0)), "M1.00 2.00");
    }
}
