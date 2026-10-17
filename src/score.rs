//! Trick names and the score line, as SkateRim shows them.
use skate_host::bridge::Score;


/// How a line ended.
pub const CLEAN: u32 = 1;
pub const LANDED: u32 = 2;
pub const SKETCHY: u32 = 3;
pub const BAILED: u32 = 4;
/// A wipeout's own score card.
pub const SLAM: u32 = 5;
/// A challenge finished.
pub const CHALLENGE: u32 = 6;

/// A finished line, for the result banner.
pub struct LineEnd {
    pub names: String,
    pub points: i64,
    pub kind: u32,
}

/// The tricks of the line being skated, for the score display.
#[derive(Default)]
pub struct Line {
    active: bool,
    /// The skate engine has a line going (rather than only flips of ours).
    engine: bool,
    tricks: Vec<String>,
    points: f32,
    /// Points for flips, which the skate engine doesn't know about.
    bonus: f32,
    total: f64,
}

impl Line {
    /// Adds a trick the helper did itself (a flip) to the line.
    pub fn bonus(&mut self, name: &str, points: f32) {
        self.active = true;
        self.tricks.push(name.to_string());
        self.bonus += points;
    }

    /// Returns how the line went when it ends. `state` is the skater's
    /// state, to tell a landing from a bail.
    pub fn update(&mut self, score: &Score, state: &str) -> Option<LineEnd> {
        if score.active {
            self.engine = true;
            let name = pretty(&score.trick);
            if !name.is_empty() && self.tricks.last() != Some(&name) {
                self.tricks.push(name);
            }
            self.points = score.points;
            self.active = true;
            return None;
        }
        if !self.active {
            return None;
        }
        // A line of only our own flips ends when the skater comes down.
        if !self.engine && state.contains("Air") {
            return None;
        }
        self.active = false;
        self.engine = false;
        let bonus = std::mem::take(&mut self.bonus);
        let tricks = std::mem::take(&mut self.tricks);
        if tricks.is_empty() {
            return None;
        }
        let names = chain(&tricks, 4);
        let bailed = state.contains("Wipeout") || (score.points <= 0.0 && bonus <= 0.0);
        let points = score.points.max(0.0) + bonus;
        let end = if !bailed && points > 0.0 {
            self.total += f64::from(points);
            let kind = if score.clean {
                CLEAN
            } else if score.sketchy {
                SKETCHY
            } else {
                LANDED
            };
            LineEnd { names, points: points as i64, kind }
        } else {
            LineEnd { names, points: 0, kind: BAILED }
        };
        crate::log!("line: {} - {} pts, {} (total {})", end.names, end.points, kind_name(end.kind), thousands(self.total as i64));
        Some(end)
    }

    /// Adds points earned outside a trick line (a challenge) to the total.
    pub fn award(&mut self, points: i64) {
        self.total += points.max(0) as f64;
    }

    /// Forgets a line in progress without scoring it.
    pub fn discard(&mut self) {
        self.active = false;
        self.engine = false;
        self.bonus = 0.0;
        self.tricks.clear();
        self.points = 0.0;
    }

    /// The tricks being linked right now (empty when not in a line).
    pub fn current(&self) -> String {
        if !self.active { String::new() } else { chain(&self.tricks, 4) }
    }

    pub fn points(&self) -> i64 {
        if self.active { (self.points + self.bonus) as i64 } else { 0 }
    }

    pub fn total(&self) -> i64 {
        self.total as i64
    }
}

/// "A + B + C", keeping the first and the last few when the line is long.
fn chain(tricks: &[String], keep: usize) -> String {
    if tricks.len() > keep {
        format!("{} + ... + {}", tricks[0], tricks[tricks.len() - (keep - 1)..].join(" + "))
    } else {
        tricks.join(" + ")
    }
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        CLEAN => "clean",
        SKETCHY => "sketchy",
        BAILED => "bailed",
        _ => "landed",
    }
}

/// Skate's trick ids as people say them: "ID TRICK FLIP HEELFLIP" ->
/// "Heelflip", "GRIND FS 5 0" -> "FS 5-0 Grind".
fn pretty(label: &str) -> String {
    let mut words: Vec<String> = label
        .split(|c: char| c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .map(str::to_uppercase)
        .collect();
    for prefix in [["ID"], ["TRICK"]] {
        if words.first().map(String::as_str) == Some(prefix[0]) {
            words.remove(0);
        }
    }
    let category = match words.first().map(String::as_str) {
        Some("FLIP") | Some("GRAB") | Some("GROUND") => {
            let c = words.remove(0);
            if c == "GROUND" && words.first().map(String::as_str) == Some("TRICK") {
                words.remove(0);
            }
            Some(c)
        }
        Some("GRIND") | Some("SLIDE") => Some(words.remove(0)),
        _ => None,
    };
    // "5 0" and "50 50" read as "5-0" and "50-50".
    let mut joined: Vec<String> = Vec::new();
    for w in words {
        if let Some(last) = joined.last_mut() {
            if last.chars().all(|c| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_digit()) {
                *last = format!("{last}-{w}");
                continue;
            }
        }
        joined.push(w);
    }
    let title = |w: &str| -> String {
        if matches!(w, "FS" | "BS" | "180" | "360" | "540") || w.chars().any(|c| c.is_ascii_digit()) {
            return w.to_string();
        }
        let lower = w.to_lowercase();
        let mut c = lower.chars();
        c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
    };
    let mut name: Vec<String> = joined.iter().map(|w| title(w)).collect();
    if category.as_deref() == Some("GRIND") && !name.iter().any(|w| w == "Slide") {
        name.push("Grind".into());
    }
    name.join(" ")
}

fn thousands(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trick_names_read_naturally() {
        assert_eq!(pretty("ID TRICK FLIP HEELFLIP"), "Heelflip");
        assert_eq!(pretty("GRIND FS 5 0"), "FS 5-0 Grind");
        assert_eq!(pretty("GRIND FS LIP SLIDE"), "FS Lip Slide");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn a_line_scores_when_it_ends() {
        let mut line = Line::default();
        let live = |trick: &str, points: f32| Score { trick: trick.into(), points, active: true, clean: false, sketchy: false };
        assert!(line.update(&live("ID TRICK FLIP KICKFLIP", 50.0), "KnownAir").is_none());
        assert!(line.update(&live("GRIND FS 50 50", 120.0), "GrindFiftyFifty").is_none());
        assert_eq!(line.current(), "Kickflip + FS 50-50 Grind");
        assert_eq!(line.points(), 120);
        let end = line.update(&Score { trick: String::new(), points: 120.0, active: false, clean: true, sketchy: false }, "PhysicsGround").unwrap();
        assert_eq!((end.points, end.kind), (120, CLEAN));
        assert_eq!(line.total(), 120);
        assert_eq!(line.current(), "");
    }

    #[test]
    fn flips_score_when_the_skater_lands() {
        let mut line = Line::default();
        let idle = Score::default();
        line.bonus("Backflip", 500.0);
        assert_eq!(line.points(), 500);
        // Still in the air: the line isn't over.
        assert!(line.update(&idle, "KnownAir").is_none());
        let end = line.update(&idle, "PhysicsGround").unwrap();
        assert_eq!((end.points, end.kind, end.names.as_str()), (500, LANDED, "Backflip"));
        // Bailing loses it.
        line.bonus("Frontflip", 500.0);
        let end = line.update(&idle, "WipeoutGround").unwrap();
        assert_eq!(end.kind, BAILED);
        assert_eq!(line.total(), 500);
    }
}
