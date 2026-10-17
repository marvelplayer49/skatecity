//! Spots and challenges, picked from the ground scanned around the skater:
//! grind a rail, drop off a ledge, or score a line at a marked spot. One is
//! active at a time; finishing it scores points and another appears.
use crate::scan::{HeightMap, METRE};
use bevy::math::{Vec2, Vec3};

/// Challenges appear this far away (metres)...
const NEAREST: f32 = 8.0;
const FURTHEST: f32 = 25.0;
/// ...and are swapped for a closer one past this.
const ABANDON: f32 = 80.0;
/// Shortest rail worth a challenge, and smallest drop (metres).
const MIN_RAIL: f32 = 3.0;
const MIN_DROP: f32 = 2.0;
/// Close enough to a rail to be grinding it (metres across, up and down).
const ON_RAIL: f32 = 2.0;
const ON_RAIL_HEIGHT: f32 = 1.8;
/// Close enough to the spot to start a line challenge, and the time allowed.
const AT_SPOT: f32 = 8.0;
const LINE_TIME: f32 = 60.0;
/// Pause between one challenge and the next (seconds).
const COOLDOWN: f32 = 2.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Grind = 1,
    Drop = 2,
    Line = 3,
}

#[derive(Clone, Debug)]
pub struct Challenge {
    pub kind: Kind,
    /// Where the marker goes (core space).
    pub at: Vec3,
    rail: Vec<Vec3>,
    /// Metres to grind, metres of drop, or points to score.
    target: f32,
    progress: f32,
    armed: Option<f32>,
}

pub enum Event {
    Message(String),
    Completed { title: String, points: i64 },
}

pub struct Challenges {
    heights: Option<HeightMap>,
    rails: Vec<Vec<Vec3>>,
    pub active: Option<Challenge>,
    pub done: u32,
    last_kind: Kind,
    cooldown: f32,
    rng: u64,
    last_root: Option<Vec3>,
    /// Where the skater left the ground, while in the air.
    air_from: Option<Vec3>,
    was_air: bool,
}

impl Default for Challenges {
    fn default() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Self {
            heights: None,
            rails: Vec::new(),
            active: None,
            done: 0,
            last_kind: Kind::Line,
            cooldown: 0.0,
            rng: seed | 1,
            last_root: None,
            air_from: None,
            was_air: false,
        }
    }
}

fn flat(v: Vec3) -> Vec2 {
    v.truncate()
}

/// Metres between two points, across the ground.
fn across(a: Vec3, b: Vec3) -> f32 {
    flat(a).distance(flat(b)) / METRE
}

fn length(rail: &[Vec3]) -> f32 {
    rail.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>() / METRE
}

/// The closest point on a rail to `p`.
fn closest_on(rail: &[Vec3], p: Vec3) -> Option<Vec3> {
    rail.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            a + ab * t
        })
        .min_by(|x, y| x.distance(p).total_cmp(&y.distance(p)))
}

fn middle(rail: &[Vec3]) -> Vec3 {
    let half = length(rail) * METRE / 2.0;
    let mut gone = 0.0;
    for w in rail.windows(2) {
        let step = w[0].distance(w[1]);
        if gone + step >= half {
            return w[0].lerp(w[1], (half - gone) / step.max(1e-3));
        }
        gone += step;
    }
    rail[0]
}

fn thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Challenges {
    /// The latest scanned ground, to find challenges in.
    pub fn set_world(&mut self, heights: HeightMap, rails: Vec<Vec<Vec3>>) {
        self.heights = Some(heights);
        self.rails = rails;
    }

    /// Forget the ground and any challenge (a new area was loaded).
    pub fn reset(&mut self) {
        let done = self.done;
        *self = Self { done, ..Self::default() };
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> Option<T> {
        if items.is_empty() {
            return None;
        }
        let i = (self.random() % items.len() as u64) as usize;
        Some(items[i].clone())
    }

    fn in_range(root: Vec3, at: Vec3, height: f32) -> bool {
        let d = across(root, at);
        (NEAREST..=FURTHEST).contains(&d) && ((at.z - root.z) / METRE).abs() <= height
    }

    fn rail_spots(&self, root: Vec3) -> Vec<Vec<Vec3>> {
        self.rails
            .iter()
            .filter(|r| r.len() >= 2 && length(r) >= MIN_RAIL && Self::in_range(root, middle(r), 6.0))
            .cloned()
            .collect()
    }

    /// Ledge tops with a big drop next to them: (the top, the drop in metres).
    fn drop_spots(&self, root: Vec3) -> Vec<(Vec3, f32)> {
        let Some(heights) = &self.heights else { return Vec::new() };
        let step = heights.spacing();
        let mut out = Vec::new();
        for (p, h) in heights.samples() {
            let Some(h) = h else { continue };
            let top = p.extend(h);
            if !Self::in_range(root, top, 4.0) {
                continue;
            }
            for dir in [Vec2::X, Vec2::Y, -Vec2::X, -Vec2::Y] {
                let (Some(below), Some(back)) = (heights.at(p + dir * step * 2.0), heights.at(p - dir * step)) else {
                    continue;
                };
                let drop = (h - below) / METRE;
                if drop >= MIN_DROP && (back - h).abs() < 15.0 {
                    out.push((top, drop));
                    break;
                }
            }
        }
        out
    }

    /// Open, flat ground for a line challenge.
    fn line_spots(&self, root: Vec3) -> Vec<Vec3> {
        let Some(heights) = &self.heights else { return Vec::new() };
        let step = heights.spacing();
        heights
            .samples()
            .filter_map(|(p, h)| {
                let h = h?;
                let at = p.extend(h);
                if !Self::in_range(root, at, 2.0) || across(root, at) > 30.0 {
                    return None;
                }
                for dx in [-2.0, 0.0, 2.0] {
                    for dy in [-2.0, 0.0, 2.0] {
                        let n = heights.at(p + Vec2::new(dx, dy) * step)?;
                        if (n - h).abs() > 10.0 {
                            return None;
                        }
                    }
                }
                Some(at)
            })
            .collect()
    }

    /// Picks the next challenge, trying a different kind from the last.
    fn spawn(&mut self, root: Vec3) -> Option<Challenge> {
        let order = match self.last_kind {
            Kind::Grind => [Kind::Drop, Kind::Line, Kind::Grind],
            Kind::Drop => [Kind::Line, Kind::Grind, Kind::Drop],
            Kind::Line => [Kind::Grind, Kind::Drop, Kind::Line],
        };
        for kind in order {
            let made = match kind {
                Kind::Grind => {
                    let spots = self.rail_spots(root);
                    self.pick(&spots).map(|rail| {
                        let target = (length(&rail) * 0.3).clamp(1.0, 2.5);
                        Challenge { kind, at: middle(&rail) + Vec3::Z * 35.0, rail, target, progress: 0.0, armed: None }
                    })
                }
                Kind::Drop => {
                    let spots = self.drop_spots(root);
                    self.pick(&spots)
                        .map(|(top, drop)| Challenge { kind, at: top, rail: Vec::new(), target: drop, progress: 0.0, armed: None })
                }
                Kind::Line => {
                    let spots = self.line_spots(root);
                    let target = (250 + 75 * self.done.min(8)) as f32;
                    self.pick(&spots)
                        .map(|at| Challenge { kind, at, rail: Vec::new(), target, progress: 0.0, armed: None })
                }
            };
            if let Some(c) = made {
                self.last_kind = kind;
                return Some(c);
            }
        }
        None
    }

    fn complete(&mut self, title: String, points: i64) -> Event {
        self.active = None;
        self.done += 1;
        self.cooldown = COOLDOWN;
        Event::Completed { title, points }
    }

    /// One frame of skating. `skip` swaps the challenge for another.
    pub fn update(&mut self, dt: f32, state: &str, root: Vec3, skip: bool) -> Vec<Event> {
        let mut events = Vec::new();
        let step = self.last_root.map_or(0.0, |last| {
            let d = last.distance(root) / METRE;
            if d < 3.0 { d } else { 0.0 }
        });
        self.last_root = Some(root);
        let wipeout = state.contains("Wipeout");
        let air = state.contains("Air") && !wipeout;
        let took_off = air && !self.was_air;
        let landed = !air && self.was_air;
        self.was_air = air;
        if took_off {
            self.air_from = Some(root);
        }
        let air_from = if landed { self.air_from.take() } else { None };

        if skip && self.active.is_some() {
            self.active = None;
            self.cooldown = 0.5;
            events.push(Event::Message("Challenge skipped".into()));
            return events;
        }
        if self.active.as_ref().is_some_and(|c| across(root, c.at) > ABANDON) {
            self.active = None;
        }
        if self.active.is_none() {
            self.cooldown -= dt;
            if self.cooldown > 0.0 {
                return events;
            }
            match self.spawn(root) {
                Some(c) => {
                    crate::log!("challenge: {:?} at {:?}, target {:.1}", c.kind, c.at, c.target);
                    self.active = Some(c);
                }
                None => self.cooldown = COOLDOWN,
            }
            return events;
        }

        let Some(c) = &mut self.active else { return events };
        match c.kind {
            Kind::Grind => {
                let on = state.contains("Grind")
                    && closest_on(&c.rail, root).is_some_and(|p| {
                        across(p, root) <= ON_RAIL && ((p.z - root.z) / METRE).abs() <= ON_RAIL_HEIGHT
                    });
                if on {
                    c.progress += step;
                }
                if c.progress >= c.target {
                    let points = 300 + (100.0 * c.target) as i64;
                    let title = format!("Ground the rail ({:.1} m)", c.progress);
                    events.push(self.complete(title, points));
                }
            }
            Kind::Drop => {
                if let Some(from) = air_from {
                    let fell = (from.z - root.z) / METRE;
                    if across(from, c.at) <= 7.0 && fell >= c.target * 0.5 {
                        if wipeout {
                            events.push(Event::Message("Bailed the drop - try again".into()));
                        } else {
                            let points = 200 + (100.0 * fell) as i64;
                            let title = format!("Dropped {fell:.1} m and rode away");
                            events.push(self.complete(title, points));
                        }
                    }
                }
            }
            Kind::Line => {
                if let Some(left) = &mut c.armed {
                    *left -= dt;
                    if *left <= 0.0 {
                        c.armed = None;
                        events.push(Event::Message("Out of time - head back to the marker".into()));
                    }
                } else if across(root, c.at) <= AT_SPOT && !wipeout {
                    c.armed = Some(LINE_TIME);
                    events.push(Event::Message(format!("Go! Score {} in one line", thousands(c.target as i64))));
                }
            }
        }
        events
    }

    /// A trick line just ended: does it finish a line challenge?
    pub fn line_ended(&mut self, points: i64, bailed: bool) -> Option<Event> {
        let c = self.active.as_ref()?;
        if c.kind != Kind::Line || c.armed.is_none() || bailed || (points as f32) < c.target {
            return None;
        }
        let title = format!("Scored {} in one line", thousands(points));
        Some(self.complete(title, (c.target / 2.0) as i64))
    }

    /// What the HUD shows: the kind, where the marker goes, and the text.
    pub fn hud(&self, root: Vec3) -> Option<(Kind, Vec3, String)> {
        let c = self.active.as_ref()?;
        let away = across(root, c.at);
        let distance = if away > AT_SPOT { format!(" - {away:.0} m away") } else { String::new() };
        let text = match c.kind {
            Kind::Grind => format!("Grind the rail: {:.1} / {:.1} m{distance}", c.progress.min(c.target), c.target),
            Kind::Drop => format!("Drop {:.0} m off the ledge and ride away{distance}", c.target),
            Kind::Line => match c.armed {
                Some(left) => format!("Score {} in one line: {:.0}s left", thousands(c.target as i64), left.max(0.0)),
                None => format!("Score {} in one line - get to the marker{distance}", thousands(c.target as i64)),
            },
        };
        Some((c.kind, c.at, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Scan;

    /// A scanned world: flat ground, a raised ledge, and a long rail.
    fn world(ground: impl Fn(Vec2) -> f32) -> HeightMap {
        let mut scan = Scan::new(Vec3::ZERO, 20.0, 0.5, 0.0);
        for _ in 0..10_000 {
            if scan.done() {
                break;
            }
            let rays = scan.batch(1000);
            let hits: Vec<Option<Vec3>> = rays
                .iter()
                .map(|(a, b)| {
                    let z = ground(a.truncate());
                    ((a.z - b.z).abs() > 1.0 && z <= a.z).then(|| a.truncate().extend(z))
                })
                .collect();
            scan.absorb(&hits);
        }
        scan.ground().heights
    }

    fn rail() -> Vec<Vec3> {
        vec![Vec3::new(1000.0, -200.0, 50.0), Vec3::new(1000.0, 200.0, 50.0)]
    }

    #[test]
    fn grinding_the_rail_finishes_the_challenge() {
        let mut c = Challenges::default();
        c.set_world(world(|_| 0.0), vec![rail()]);
        c.last_kind = Kind::Line; // so a grind comes next
        assert!(c.update(0.1, "PhysicsGround", Vec3::ZERO, false).is_empty());
        assert_eq!(c.active.as_ref().map(|a| a.kind), Some(Kind::Grind));
        // Grinding along the rail.
        let mut done = false;
        for i in 0..60 {
            let p = Vec3::new(1000.0, -200.0 + i as f32 * 7.0, 50.0);
            for e in c.update(1.0 / 30.0, "GrindFiftyFifty", p, false) {
                done |= matches!(e, Event::Completed { .. });
            }
        }
        assert!(done);
        assert_eq!(c.done, 1);
    }

    #[test]
    fn dropping_off_the_ledge_finishes_the_challenge() {
        // Standing on a 4 m block; the street is below to the east.
        let block = |p: Vec2| if p.x < 1200.0 { 280.0 } else { 0.0 };
        let mut c = Challenges::default();
        c.set_world(world(block), Vec::new());
        c.last_kind = Kind::Grind; // so a drop comes next
        let start = Vec3::new(500.0, 0.0, 280.0);
        c.update(0.1, "PhysicsGround", start, false);
        let spot = c.active.clone().expect("a drop challenge");
        assert_eq!(spot.kind, Kind::Drop);
        // Roll to the edge, fly off, land on the street.
        let edge = spot.at;
        c.update(0.1, "PhysicsGround", edge, false);
        c.update(0.1, "KnownAir", edge, false);
        let events = c.update(0.1, "PhysicsGround", Vec3::new(edge.x + 100.0, edge.y, 0.0), false);
        assert!(events.iter().any(|e| matches!(e, Event::Completed { .. })));
    }

    #[test]
    fn a_line_at_the_spot_finishes_the_challenge() {
        let mut c = Challenges::default();
        c.set_world(world(|_| 0.0), Vec::new());
        c.last_kind = Kind::Drop; // so a line comes next
        c.update(0.1, "PhysicsGround", Vec3::ZERO, false);
        let spot = c.active.clone().expect("a line challenge");
        assert_eq!(spot.kind, Kind::Line);
        // A big line away from the spot doesn't count...
        assert!(c.line_ended(5000, false).is_none());
        // ...one at the spot does, unless bailed.
        let events = c.update(0.1, "PhysicsGround", spot.at, false);
        assert!(events.iter().any(|e| matches!(e, Event::Message(_))));
        assert!(c.line_ended(5000, true).is_none());
        assert!(matches!(c.line_ended(5000, false), Some(Event::Completed { .. })));
    }

    #[test]
    fn skipping_brings_a_new_challenge() {
        let mut c = Challenges::default();
        c.set_world(world(|_| 0.0), vec![rail()]);
        c.update(0.1, "PhysicsGround", Vec3::ZERO, false);
        assert!(c.active.is_some());
        c.update(0.1, "PhysicsGround", Vec3::ZERO, true);
        assert!(c.active.is_none());
        for _ in 0..6 {
            c.update(0.1, "PhysicsGround", Vec3::ZERO, false);
        }
        assert!(c.active.is_some());
    }
}
