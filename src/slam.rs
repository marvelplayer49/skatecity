//! Bail scoring, Hall of Meat style. The skate engine port doesn't work out
//! injuries itself, so they're read off the ragdoll: every tick of a
//! wipeout, each body part's change in velocity is measured, and a sharp
//! one is that part hitting something. Hard enough hits break bones.
use crate::rig::Joints;
use crate::scan::METRE;
use bevy::math::Vec3;
use std::collections::HashMap;

/// The engine's tick rate.
const TICKS_PER_SECOND: f32 = 60.0;
/// A body part's velocity changing this much in one tick (m/s) is a hit...
const HIT: f32 = 4.0;
/// ...this much is a hard one (a heavier sound)...
const HARD_HIT: f32 = 7.0;
/// ...and this much breaks the bone.
const BREAK: f32 = 10.5;
/// Moving further than this (m) in one tick is a teleport, not motion.
const TELEPORT: f32 = 3.0;

/// The body parts watched, with what breaking each one is called.
const PARTS: [(&str, &str); 13] = [
    ("HEAD", "Fractured Skull"),
    ("SPINE2", "Broken Ribs"),
    ("HIPS", "Broken Pelvis"),
    ("LEFTSHOULDER", "Broken Left Collarbone"),
    ("RIGHTSHOULDER", "Broken Right Collarbone"),
    ("LEFTFOREARM", "Broken Left Arm"),
    ("RIGHTFOREARM", "Broken Right Arm"),
    ("LEFTHAND", "Broken Left Wrist"),
    ("RIGHTHAND", "Broken Right Wrist"),
    ("LEFTLEG", "Broken Left Leg"),
    ("RIGHTLEG", "Broken Right Leg"),
    ("LEFTFOOT", "Broken Left Ankle"),
    ("RIGHTFOOT", "Broken Right Ankle"),
];

/// Something worth a sound: where (core space) and how hard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Light(Vec3),
    Hard(Vec3),
    Break(Vec3),
}

/// A finished slam.
#[derive(Clone, Debug)]
pub struct SlamEnd {
    pub score: i64,
    pub breaks: Vec<&'static str>,
    pub hits: u32,
    /// Metres fallen from where the skater left the ground, top speed (m/s)
    /// and how far they tumbled (m).
    pub fall: f32,
    pub top_speed: f32,
    pub tumble: f32,
}

struct Active {
    start_z: f32,
    min_z: f32,
    top_speed: f32,
    tumble: f32,
    last_root: Vec3,
    hits: u32,
    hit_score: f32,
    broken: Vec<usize>,
    /// Consecutive ticks out of the wipeout, to ride out a flicker.
    out_ticks: u32,
}

#[derive(Default)]
pub struct Slams {
    last_tick: Option<u64>,
    positions: HashMap<&'static str, Vec3>,
    velocities: HashMap<&'static str, Vec3>,
    /// Height the skater left the ground at, while airborne.
    air_start: Option<f32>,
    active: Option<Active>,
}

impl Slams {
    /// Feeds one engine tick. Returns any hits (for sounds) and, when a
    /// wipeout is over, how it went.
    pub fn update(&mut self, tick: u64, state: &str, root: Vec3, speed: f32, joints: &Joints) -> (Vec<Hit>, Option<SlamEnd>) {
        let mut hits = Vec::new();
        let dt = match self.last_tick {
            Some(last) if tick <= last => return (hits, None),
            Some(last) => (tick - last) as f32 / TICKS_PER_SECOND,
            None => 0.0,
        };
        self.last_tick = Some(tick);
        let wipeout = state.contains("Wipeout");

        // Where the fall started: the last time the wheels left the ground.
        if !wipeout {
            if state.contains("Air") {
                self.air_start.get_or_insert(root.z);
            } else {
                self.air_start = None;
            }
        }

        // Each body part's velocity, and how sharply it changed.
        let mut changes: Vec<(usize, f32, Vec3)> = Vec::new();
        let fresh = dt <= 0.0 || dt > 0.1;
        for (i, (name, _)) in PARTS.iter().enumerate() {
            let Some(p) = joints.raw(name) else { continue };
            let previous = self.positions.insert(name, p);
            let Some(prev) = previous.filter(|_| !fresh) else {
                self.velocities.remove(name);
                continue;
            };
            let moved = (p - prev) / METRE;
            if moved.length() > TELEPORT {
                self.velocities.remove(name);
                continue;
            }
            let v = moved / dt;
            if let Some(old) = self.velocities.insert(name, v) {
                changes.push((i, (v - old).length(), p));
            }
        }

        if wipeout && self.active.is_none() {
            let start_z = self.air_start.map_or(root.z, |z| z.max(root.z));
            self.active = Some(Active {
                start_z,
                min_z: root.z,
                top_speed: speed,
                tumble: 0.0,
                last_root: root,
                hits: 0,
                hit_score: 0.0,
                broken: Vec::new(),
                out_ticks: 0,
            });
        }
        let Some(slam) = &mut self.active else { return (hits, None) };

        if wipeout {
            slam.out_ticks = 0;
            slam.min_z = slam.min_z.min(root.z);
            slam.top_speed = slam.top_speed.max(speed);
            let step = (root - slam.last_root).length() / METRE;
            if step < TELEPORT {
                slam.tumble += step;
            }
            slam.last_root = root;
            for (part, change, at) in changes {
                if change < HIT {
                    continue;
                }
                slam.hits += 1;
                slam.hit_score += (change - HIT + 1.0).powi(2) * 15.0;
                if change >= BREAK && !slam.broken.contains(&part) {
                    slam.broken.push(part);
                    hits.push(Hit::Break(at));
                } else if change >= HARD_HIT {
                    hits.push(Hit::Hard(at));
                } else {
                    hits.push(Hit::Light(at));
                }
            }
            return (hits, None);
        }

        // Back on the board (a few ticks running, to be sure).
        slam.out_ticks += 1;
        if slam.out_ticks < 3 {
            return (hits, None);
        }
        (hits, self.finish())
    }

    /// Ends any slam in progress (skating stopped), returning how it went.
    pub fn finish(&mut self) -> Option<SlamEnd> {
        let slam = self.active.take()?;
        self.air_start = None;
        let fall = ((slam.start_z - slam.min_z) / METRE).max(0.0);
        let breaks: Vec<&'static str> = slam.broken.iter().map(|&i| PARTS[i].1).collect();
        let score = slam.hit_score
            + breaks.len() as f32 * 1500.0
            + fall * 60.0
            + slam.tumble * 20.0
            + slam.top_speed * 30.0;
        Some(SlamEnd {
            score: ((score / 10.0).round() * 10.0) as i64,
            breaks,
            hits: slam.hits,
            fall,
            top_speed: slam.top_speed,
            tumble: slam.tumble,
        })
    }

    /// Forgets everything (a new session, a teleport back onto the board).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A skater whose every body part sits at `at` (core units).
    fn body(at: Vec3) -> Joints {
        let parts: Vec<(&str, Vec3)> = PARTS.iter().map(|(n, _)| (*n, at)).collect();
        Joints::from_points(&parts)
    }

    #[test]
    fn a_hard_landing_breaks_bones_and_scores() {
        let mut slams = Slams::default();
        let mut tick = 0u64;
        let mut z = 10.0 * METRE;
        let mut step = |slams: &mut Slams, state: &str, z: f32| {
            tick += 1;
            slams.update(tick, state, Vec3::new(0.0, 0.0, z), 15.0, &body(Vec3::new(0.0, 0.0, z)))
        };
        // Rolling, then off a 10 m roof: falling at 15 m/s (0.25 m a tick).
        step(&mut slams, "PhysicsGround", z);
        for _ in 0..10 {
            z -= 0.25 * METRE;
            assert!(step(&mut slams, "KnownAir", z).1.is_none());
        }
        // Wiped out, then smack into the street and stopped dead.
        z -= 0.25 * METRE;
        step(&mut slams, "WipeoutAir", z);
        let (hits, end) = step(&mut slams, "WipeoutGround", z);
        assert!(end.is_none());
        assert!(hits.iter().any(|h| matches!(h, Hit::Break(_))), "{hits:?}");
        for _ in 0..5 {
            step(&mut slams, "WipeoutGround", z);
        }
        // Back on the board.
        let mut end = None;
        for _ in 0..3 {
            end = end.or(step(&mut slams, "PhysicsGround", z).1);
        }
        let end = end.expect("the slam should end once back on the board");
        assert!(end.breaks.contains(&"Fractured Skull"), "{:?}", end.breaks);
        assert!(end.fall > 2.4, "fell {}", end.fall);
        assert!(end.score > 1500, "scored {}", end.score);
    }

    #[test]
    fn rolling_along_is_not_a_slam() {
        let mut slams = Slams::default();
        for tick in 1..100u64 {
            let p = Vec3::new(tick as f32 * 5.0, 0.0, 0.0);
            let (hits, end) = slams.update(tick, "PhysicsGround", p, 5.0, &body(p));
            assert!(hits.is_empty() && end.is_none());
        }
        assert!(slams.finish().is_none());
    }

    #[test]
    fn a_teleport_is_not_a_hit() {
        let mut slams = Slams::default();
        slams.update(1, "WipeoutGround", Vec3::ZERO, 1.0, &body(Vec3::ZERO));
        slams.update(2, "WipeoutGround", Vec3::ZERO, 1.0, &body(Vec3::ZERO));
        let far = Vec3::new(5000.0, 0.0, 0.0);
        let (hits, _) = slams.update(3, "WipeoutGround", far, 1.0, &body(far));
        assert!(hits.is_empty());
    }
}
