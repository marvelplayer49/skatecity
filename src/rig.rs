//! Poses Batman from the skater. As in SkateRim, the two skeletons have
//! different bind poses, so instead of copying rotations each mapped bone is
//! turned until it points where the skater's matching bone points; twist
//! bones, fingers and the cape keep what they had when skating began.
//!
//! Batman's skeleton is a 3ds Max Biped ("Bip01_..."), held by the game as
//! arrays of local and component-space transforms with parent indices.
use crate::scan::{from_skate, skate_dir_to_core};
use crate::space::Xf;
use bevy::math::{Mat3, Quat, Vec3};
use skate_host::bridge::Pose;
use std::collections::HashMap;

const COM: &str = "Bip01";
const PELVIS: &str = "Bip01_Pelvis";
const NECK: &str = "Bip01_Neck";
const HEAD: &str = "Bip01_Head";
const L_THIGH: &str = "Bip01_L_Thigh";
const R_THIGH: &str = "Bip01_R_Thigh";
const L_TOE: &str = "Bip01_L_Toe0";
const R_TOE: &str = "Bip01_R_Toe0";

/// (Batman bone, a bone below it, Skate joint, Skate joint below), parents first.
const CHAINS: [(&str, &str, &str, &str); 16] = [
    ("Bip01_Spine", "Bip01_Spine1", "SPINE", "SPINE1"),
    ("Bip01_Spine1", "Bip01_Spine3", "SPINE1", "SPINE3"),
    ("Bip01_Spine3", NECK, "SPINE3", "NECK"),
    (NECK, HEAD, "NECK", "HEAD"),
    ("Bip01_L_Clavicle", "Bip01_L_UpperArm", "LEFTSHOULDER", "LEFTARM"),
    ("Bip01_L_UpperArm", "Bip01_L_Forearm", "LEFTARM", "LEFTFOREARM"),
    ("Bip01_L_Forearm", "Bip01_L_Hand", "LEFTFOREARM", "LEFTHAND"),
    ("Bip01_R_Clavicle", "Bip01_R_UpperArm", "RIGHTSHOULDER", "RIGHTARM"),
    ("Bip01_R_UpperArm", "Bip01_R_Forearm", "RIGHTARM", "RIGHTFOREARM"),
    ("Bip01_R_Forearm", "Bip01_R_Hand", "RIGHTFOREARM", "RIGHTHAND"),
    (L_THIGH, "Bip01_L_Calf", "LEFTUPLEG", "LEFTLEG"),
    ("Bip01_L_Calf", "Bip01_L_Foot", "LEFTLEG", "LEFTFOOT"),
    ("Bip01_L_Foot", L_TOE, "LEFTFOOT", "LEFTTOEBASE"),
    (R_THIGH, "Bip01_R_Calf", "RIGHTUPLEG", "RIGHTLEG"),
    ("Bip01_R_Calf", "Bip01_R_Foot", "RIGHTLEG", "RIGHTFOOT"),
    ("Bip01_R_Foot", R_TOE, "RIGHTFOOT", "RIGHTTOEBASE"),
];

/// Arms and legs, posed as a whole so the elbow and knee bend in the same
/// plane as the skater's (a single-bone turn leaves them twisted):
/// (upper, lower, end, Skate upper, Skate lower, Skate end, the limb bends
/// toward the front, as elbows do, rather than the back, as knees do).
const LIMBS: [(&str, &str, &str, &str, &str, &str, bool); 4] = [
    (L_THIGH, "Bip01_L_Calf", "Bip01_L_Foot", "LEFTUPLEG", "LEFTLEG", "LEFTFOOT", false),
    (R_THIGH, "Bip01_R_Calf", "Bip01_R_Foot", "RIGHTUPLEG", "RIGHTLEG", "RIGHTFOOT", false),
    ("Bip01_L_UpperArm", "Bip01_L_Forearm", "Bip01_L_Hand", "LEFTARM", "LEFTFOREARM", "LEFTHAND", true),
    ("Bip01_R_UpperArm", "Bip01_R_Forearm", "Bip01_R_Hand", "RIGHTARM", "RIGHTFOREARM", "RIGHTHAND", true),
];

/// (thigh, calf, foot, Skate ankle, Skate hip, Skate knee): each leg bends
/// so the ankle lands where the skater's is, keeping the feet on the board.
const FEET: [(&str, &str, &str, &str, &str, &str); 2] = [
    (L_THIGH, "Bip01_L_Calf", "Bip01_L_Foot", "LEFTFOOT", "LEFTUPLEG", "LEFTLEG"),
    (R_THIGH, "Bip01_R_Calf", "Bip01_R_Foot", "RIGHTFOOT", "RIGHTUPLEG", "RIGHTLEG"),
];

/// (foot, toe, calf, Skate ankle, Skate toe, Skate knee): how each foot sits.
const SOLES: [(&str, &str, &str, &str, &str, &str); 2] = [
    ("Bip01_L_Foot", L_TOE, "Bip01_L_Calf", "LEFTFOOT", "LEFTTOEBASE", "LEFTLEG"),
    ("Bip01_R_Foot", R_TOE, "Bip01_R_Calf", "RIGHTFOOT", "RIGHTTOEBASE", "RIGHTLEG"),
];

/// Twist bones and the limb each lies along. In Batman's rig they hang off
/// the bone above the limb (thigh twists off the spine, calf twists off the
/// thigh, ...), so turning the limb leaves them behind and the skin they
/// carry tears away from it; they're carried along with their limb instead.
const TWISTS: [(&str, &str); 8] = [
    ("Bip01_LThighTwist", L_THIGH),
    ("Bip01_RThighTwist", R_THIGH),
    ("Bip01_LCalfTwist", "Bip01_L_Calf"),
    ("Bip01_RCalfTwist", "Bip01_R_Calf"),
    ("Bip01_LUpArmTwist", "Bip01_L_UpperArm"),
    ("Bip01_RUpArmTwist", "Bip01_R_UpperArm"),
    ("Bip01_L_Foretwist", "Bip01_L_Forearm"),
    ("Bip01_R_Foretwist", "Bip01_R_Forearm"),
];

/// The skater's joints in core space.
#[derive(Clone)]
pub struct Joints {
    at: HashMap<String, Vec3>,
    /// Unscaled positions, for the feet: they stay on the board whatever
    /// the size of the character.
    raw: HashMap<String, Vec3>,
}

impl Joints {
    pub fn from_pose(pose: &Pose, scale: f32) -> Self {
        let root = from_skate(pose.root.w_axis.truncate());
        let raw: HashMap<String, Vec3> = pose
            .names
            .iter()
            .zip(&pose.bones)
            .map(|(name, m)| (name.clone(), from_skate(pose.root.transform_point3(m.w_axis.truncate()))))
            .collect();
        let at = raw.iter().map(|(name, p)| (name.clone(), root + (*p - root) * scale)).collect();
        Self { at, raw }
    }
    pub fn lerp(&self, next: &Joints, t: f32) -> Joints {
        let mix = |a: &HashMap<String, Vec3>, b: &HashMap<String, Vec3>| {
            b.iter().map(|(name, q)| (name.clone(), a.get(name).map_or(*q, |p| p.lerp(*q, t)))).collect()
        };
        Joints { at: mix(&self.at, &next.at), raw: mix(&self.raw, &next.raw) }
    }
    pub fn get(&self, name: &str) -> Option<Vec3> {
        self.at.get(name).copied()
    }
    /// Joints at the given points (for tests).
    #[cfg(test)]
    pub fn from_points(points: &[(&str, Vec3)]) -> Joints {
        let map: HashMap<String, Vec3> = points.iter().map(|(n, p)| (n.to_string(), *p)).collect();
        Joints { at: map.clone(), raw: map }
    }

    /// The same joints turned by `rot` about `pivot` (a flip).
    pub fn rotated(&self, pivot: Vec3, rot: Quat) -> Joints {
        let turn = |m: &HashMap<String, Vec3>| m.iter().map(|(n, p)| (n.clone(), pivot + rot * (*p - pivot))).collect();
        Joints { at: turn(&self.at), raw: turn(&self.raw) }
    }

    /// Every joint, unscaled.
    pub fn names(&self) -> impl Iterator<Item = (&str, Vec3)> {
        self.raw.iter().map(|(n, p)| (n.as_str(), *p))
    }
    pub fn raw(&self, name: &str) -> Option<Vec3> {
        self.raw.get(name).copied()
    }
    /// The skater's bone `name` as a core-space transform.
    pub fn transform(pose: &Pose, name: &str) -> Option<Xf> {
        let i = pose.names.iter().position(|n| n == name)?;
        let m = pose.root * pose.bones[i];
        let b = Mat3::from_cols(skate_dir_to_core(Vec3::X), skate_dir_to_core(Vec3::Y), skate_dir_to_core(Vec3::Z));
        let r = Mat3::from_cols(
            m.x_axis.truncate().normalize_or_zero(),
            m.y_axis.truncate().normalize_or_zero(),
            m.z_axis.truncate().normalize_or_zero(),
        );
        Some(Xf { rot: Quat::from_mat3(&(b * r * b.transpose())).normalize(), pos: from_skate(m.w_axis.truncate()), scale: 1.0 })
    }
}

pub struct Rig {
    pub names: Vec<String>,
    parents: Vec<Option<usize>>,
    /// Bones in an order where parents come first.
    order: Vec<usize>,
    index: HashMap<String, usize>,
    /// Local transforms when skating began, in core space.
    base: Vec<Xf>,
    missing: Vec<&'static str>,
}

impl Rig {
    pub fn new(names: Vec<String>, parents: Vec<i32>) -> Self {
        let n = names.len();
        let parents: Vec<Option<usize>> =
            parents.iter().enumerate().map(|(i, &p)| (p >= 0 && (p as usize) < n && p as usize != i).then_some(p as usize)).collect();
        let index: HashMap<String, usize> = names.iter().enumerate().map(|(i, s)| (s.clone(), i)).collect();
        // Depth-first from the roots so parents always precede children.
        let mut children = vec![Vec::new(); n];
        let mut roots = Vec::new();
        for (i, p) in parents.iter().enumerate() {
            match p {
                Some(p) => children[*p].push(i),
                None => roots.push(i),
            }
        }
        let mut order = Vec::with_capacity(n);
        let mut stack: Vec<usize> = roots.into_iter().rev().collect();
        while let Some(i) = stack.pop() {
            if order.len() > n {
                break;
            }
            order.push(i);
            stack.extend(children[i].iter().rev());
        }
        let mut wanted = vec![COM, PELVIS, L_THIGH, R_THIGH, NECK, HEAD];
        for (a, b, _, _) in CHAINS {
            wanted.push(a);
            wanted.push(b);
        }
        let mut missing: Vec<&'static str> = wanted.into_iter().filter(|w| !index.contains_key(*w)).collect();
        missing.sort();
        missing.dedup();
        Self { names, parents, order, index, base: vec![Xf::IDENTITY; n], missing }
    }

    pub fn parent(&self, i: usize) -> Option<usize> {
        self.parents.get(i).copied().flatten()
    }

    /// The bones as they were when skating began, placed at `component`.
    pub fn base_world(&self, component: &Xf) -> Vec<Xf> {
        self.space(&self.base).iter().map(|s| component.mul(s)).collect()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn missing(&self) -> &[&'static str] {
        &self.missing
    }

    pub fn usable(&self) -> bool {
        [COM, PELVIS, L_THIGH, R_THIGH, NECK].iter().all(|n| self.index.contains_key(*n)) && self.order.len() == self.len()
    }

    pub fn set_base(&mut self, locals: Vec<Xf>) {
        if locals.len() == self.len() {
            self.base = locals;
        }
    }

    /// Component-space transforms from local ones.
    pub fn space(&self, locals: &[Xf]) -> Vec<Xf> {
        let mut out = vec![Xf::IDENTITY; self.len()];
        for &i in &self.order {
            out[i] = match self.parents[i] {
                Some(p) => out[p].mul(&locals[i]),
                None => locals[i],
            };
        }
        out
    }

    /// Batman's height from toe to head bone in his own units, for working
    /// out how big a metre is in this game.
    /// The mesh may stand along any axis of its own space, so this is the
    /// straight-line distance from toe to head bone, not a height.
    pub fn standing_height(&self, locals: &[Xf]) -> Option<f32> {
        let space = self.space(locals);
        let at = |n: &str| self.index.get(n).map(|&i| space[i].pos);
        let head = at(HEAD)?;
        let toe = at(L_TOE).or(at(R_TOE))?;
        Some(head.distance(toe))
    }

    /// The skater's pose on Batman: component-space transforms (core space)
    /// for every bone, given where the component sits in the world.
    pub fn pose(&self, component: &Xf, joints: &Joints, plant_feet: bool) -> Option<Vec<Xf>> {
        if !self.usable() {
            return None;
        }
        let mut pose = Posed { rig: self, locals: self.base.clone(), world: Vec::new(), component: *component };
        pose.refresh();
        let bone = |n: &str| self.index.get(n).copied();
        let (com, pelvis, lt, rt, neck) = (bone(COM)?, bone(PELVIS)?, bone(L_THIGH)?, bone(R_THIGH)?, bone(NECK)?);
        let (hips, sk_neck, sk_l, sk_r) = (joints.get("HIPS")?, joints.get("NECK")?, joints.get("LEFTUPLEG")?, joints.get("RIGHTUPLEG")?);

        // Hips: orient the whole body from the skater's spine and hip line.
        let pelvis_at = pose.world[pelvis].pos;
        let target = frame(sk_neck - hips, sk_r - sk_l);
        let forward = target.map(|t| t.z_axis).unwrap_or(Vec3::X);
        let current = frame(pose.world[neck].pos - pelvis_at, pose.world[rt].pos - pose.world[lt].pos);
        if let (Some(current), Some(target)) = (current, target) {
            let turn = Quat::from_mat3(&(target * current.transpose())).normalize();
            let mut w = pose.world[com];
            let offset = pelvis_at - w.pos;
            w.rot = (turn * w.rot).normalize();
            w.pos = hips - turn * offset;
            pose.set(com, w);
        }

        for (name, child, from, to) in CHAINS {
            if let Some(limb) = LIMBS.iter().find(|l| l.0 == name) {
                let (Some(u), Some(l), Some(e)) = (bone(limb.0), bone(limb.1), bone(limb.2)) else { continue };
                let (Some(wu), Some(wl), Some(we)) = (joints.get(limb.3), joints.get(limb.4), joints.get(limb.5)) else { continue };
                pose.limb(u, l, e, (wu, wl, we), forward, limb.6);
                continue;
            }
            if LIMBS.iter().any(|l| l.1 == name) {
                continue;
            }
            let (Some(b), Some(c), Some(f), Some(t)) = (bone(name), bone(child), joints.get(from), joints.get(to)) else {
                continue;
            };
            let w = pose.world[b];
            let (Some(have), Some(wanted)) = ((pose.world[c].pos - w.pos).try_normalize(), (t - f).try_normalize()) else {
                continue;
            };
            let turn = Quat::from_rotation_arc(have, wanted);
            pose.set(b, Xf { rot: (turn * w.rot).normalize(), ..w });
        }

        if plant_feet {
            // If Batman's legs can't reach the board from where his hips are,
            // crouch him a little rather than lock his knees straight.
            let mut drop: f32 = 0.0;
            let mut leg_len: f32 = 0.0;
            for (thigh, calf, foot, target, ..) in FEET {
                let (Some(t), Some(c), Some(f), Some(goal)) = (bone(thigh), bone(calf), bone(foot), joints.raw(target)) else {
                    continue;
                };
                let (hip, knee, ankle) = (pose.world[t].pos, pose.world[c].pos, pose.world[f].pos);
                let reach = (hip.distance(knee) + knee.distance(ankle)) * 0.96;
                leg_len = leg_len.max(reach);
                let v = hip - goal;
                let flat = v.truncate().length_squared();
                let need = if reach * reach > flat { v.z - (reach * reach - flat).sqrt() } else { v.z };
                drop = drop.max(need);
            }
            let drop = drop.clamp(0.0, leg_len * 0.3);
            if drop > 0.01 {
                let mut w = pose.world[com];
                w.pos.z -= drop;
                pose.set(com, w);
            }
            for (thigh, calf, foot, target, sk_hip, sk_knee) in FEET {
                if let (Some(t), Some(c), Some(f), Some(goal)) = (bone(thigh), bone(calf), bone(foot), joints.raw(target)) {
                    // Bend the knee the way the skater's knee bends.
                    let hint = match (joints.raw(sk_hip), joints.raw(sk_knee)) {
                        (Some(h), Some(k)) => {
                            let line = (goal - h).normalize_or_zero();
                            let off = (k - h) - line * (k - h).dot(line);
                            (off.length() > 2.0).then_some(off)
                        }
                        _ => None,
                    };
                    pose.plant(t, c, f, goal, hint, forward);
                }
            }
        }
        // Feet flat on the board: the foot points at the toe and keeps the
        // ankle-knee plane the skater has, so it doesn't roll onto its side.
        for (foot, toe, calf, sk_foot, sk_toe, sk_knee) in SOLES {
            let (Some(f), Some(t), Some(c)) = (bone(foot), bone(toe), bone(calf)) else { continue };
            let (Some(wf), Some(wt), Some(wk)) = (joints.raw(sk_foot), joints.raw(sk_toe), joints.raw(sk_knee)) else { continue };
            let w = pose.world[f];
            let (ft, fk) = (pose.world[t].pos - w.pos, pose.world[c].pos - w.pos);
            let turn = align(ft, ft.cross(fk), wt - wf, (wt - wf).cross(wk - wf));
            pose.set(f, Xf { rot: (turn * w.rot).normalize(), ..w });
        }
        // Twist bones keep the place on their limb they had at the start.
        let base_world = self.base_world(component);
        for (twist, limb) in TWISTS {
            let (Some(t), Some(l)) = (bone(twist), bone(limb)) else { continue };
            if self.parents[t] == Some(l) {
                continue;
            }
            let rel = base_world[l].inverse().mul(&base_world[t]);
            let w = pose.world[l].mul(&rel);
            pose.set(t, w);
        }
        let to_component = component.inverse();
        let out: Vec<Xf> = pose.world.iter().map(|w| to_component.mul(w)).collect();
        out.iter().all(Xf::is_finite).then_some(out)
    }
}

struct Posed<'a> {
    rig: &'a Rig,
    locals: Vec<Xf>,
    world: Vec<Xf>,
    component: Xf,
}

impl Posed<'_> {
    fn refresh(&mut self) {
        let space = self.rig.space(&self.locals);
        self.world = space.iter().map(|s| self.component.mul(s)).collect();
    }

    /// Puts bone `i` at `world`, carrying everything below it along.
    fn set(&mut self, i: usize, world: Xf) {
        let parent = self.rig.parents[i].map_or(self.component, |p| self.world[p]);
        self.locals[i] = parent.inverse().mul(&world);
        self.refresh();
    }

    /// Poses an arm or leg: each bone points along the skater's, and the
    /// limb bends in the same plane, so knees and elbows face the right way.
    fn limb(&mut self, upper: usize, lower: usize, end: usize, want: (Vec3, Vec3, Vec3), forward: Vec3, bends_forward: bool) {
        let (wa, wb, wc) = want;
        let n_want = (wb - wa).cross(wc - wb);
        let (a, b, c) = (self.world[upper].pos, self.world[lower].pos, self.world[end].pos);
        let (l1, l2) = ((b - a).length(), (c - b).length());
        if l1 < 1e-3 || l2 < 1e-3 {
            return;
        }
        // A nearly straight limb has no bend plane of its own: assume the
        // one it would bend in (knees forward, elbows up in front).
        let mut n_have = (b - a).cross(c - b);
        if n_have.length() < 0.08 * l1 * l2 {
            let lean = if bends_forward { forward } else { -forward };
            n_have = (b - a).cross((c - b) + lean * (0.4 * l2));
        }
        let straight_want = n_want.length() < 0.03 * (wb - wa).length() * (wc - wb).length();
        let turn = if straight_want {
            Quat::from_rotation_arc((b - a).normalize(), (wb - wa).normalize_or(b - a))
        } else {
            align(b - a, n_have, wb - wa, n_want)
        };
        let w = self.world[upper];
        self.set(upper, Xf { rot: (turn * w.rot).normalize(), ..w });
        let n_have = turn * n_have;
        let (b, c) = (self.world[lower].pos, self.world[end].pos);
        let turn = if straight_want {
            Quat::from_rotation_arc((c - b).normalize_or(Vec3::Z), (wc - wb).normalize_or(c - b))
        } else {
            align(c - b, n_have, wc - wb, n_want)
        };
        let w = self.world[lower];
        self.set(lower, Xf { rot: (turn * w.rot).normalize(), ..w });
    }

    /// Two-bone IK: bends the knee in its current plane so the ankle
    /// reaches `goal`, keeping the foot's own orientation.
    fn plant(&mut self, thigh: usize, calf: usize, foot: usize, goal: Vec3, hint: Option<Vec3>, forward: Vec3) {
        let hip = self.world[thigh].pos;
        let knee = self.world[calf].pos;
        let foot_rot = self.world[foot].rot;
        let (upper, lower) = (hip.distance(knee), knee.distance(self.world[foot].pos));
        if upper < 1.0 || lower < 1.0 {
            return;
        }
        let Some(dir) = (goal - hip).try_normalize() else { return };
        let reach = hip.distance(goal).clamp((upper - lower).abs() + 0.1, upper + lower - 0.1);
        let cos_hip = ((upper * upper + reach * reach - lower * lower) / (2.0 * upper * reach)).clamp(-1.0, 1.0);
        // Which way the knee points: the skater's knee, else Batman's own
        // if it is clearly bent, else straight ahead.
        let perp = |v: Vec3| (v - dir * v.dot(dir)).try_normalize();
        let own = (knee - hip) - dir * (knee - hip).dot(dir);
        let bend = hint
            .and_then(perp)
            .or_else(|| (own.length() > 0.1 * upper).then(|| perp(own)).flatten())
            .or_else(|| perp(forward))
            .or_else(|| dir.any_orthonormal_vector().try_normalize());
        let Some(bend) = bend else { return };
        let new_knee = hip + dir * (upper * cos_hip) + bend * (upper * (1.0 - cos_hip * cos_hip).max(0.0).sqrt());
        let new_ankle = hip + dir * reach;

        let w = self.world[thigh];
        if let (Some(a), Some(b)) = ((knee - hip).try_normalize(), (new_knee - hip).try_normalize()) {
            self.set(thigh, Xf { rot: (Quat::from_rotation_arc(a, b) * w.rot).normalize(), ..w });
        }
        let w = self.world[calf];
        let ankle_now = self.world[foot].pos;
        if let (Some(a), Some(b)) = ((ankle_now - w.pos).try_normalize(), (new_ankle - w.pos).try_normalize()) {
            self.set(calf, Xf { rot: (Quat::from_rotation_arc(a, b) * w.rot).normalize(), ..w });
        }
        let w = self.world[foot];
        self.set(foot, Xf { rot: foot_rot, ..w });
    }
}

/// The rotation taking direction `d1` to `d2` and the plane normal `n1` to
/// `n2` (as near as the directions allow).
fn align(d1: Vec3, n1: Vec3, d2: Vec3, n2: Vec3) -> Quat {
    let basis = |d: Vec3, n: Vec3| -> Option<Mat3> {
        let x = d.try_normalize()?;
        let y = (n - x * n.dot(x)).try_normalize()?;
        Some(Mat3::from_cols(x, y, x.cross(y)))
    };
    match (basis(d1, n1), basis(d2, n2)) {
        (Some(f1), Some(f2)) => Quat::from_mat3(&(f2 * f1.transpose())).normalize(),
        _ => Quat::from_rotation_arc(d1.normalize_or(Vec3::X), d2.normalize_or(Vec3::X)),
    }
}

/// An orthonormal frame with `up` exact and `right` made perpendicular.
fn frame(up: Vec3, right: Vec3) -> Option<Mat3> {
    let up = up.try_normalize()?;
    let forward = up.cross(right).try_normalize()?;
    let right = forward.cross(up);
    Some(Mat3::from_cols(right, up, forward))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stick figure with Biped names: root, pelvis, spine up to the head,
    /// arms off the top of the spine and legs off the pelvis.
    fn figure() -> (Vec<String>, Vec<i32>, Vec<Xf>) {
        let bones: Vec<(&str, i32, Vec3)> = vec![
            (COM, -1, Vec3::new(0.0, 0.0, 70.0)),
            (PELVIS, 0, Vec3::ZERO),
            ("Bip01_Spine", 1, Vec3::new(0.0, 0.0, 5.0)),
            ("Bip01_Spine1", 2, Vec3::new(0.0, 0.0, 8.0)),
            ("Bip01_Spine2", 3, Vec3::new(0.0, 0.0, 8.0)),
            ("Bip01_Spine3", 4, Vec3::new(0.0, 0.0, 8.0)),
            (NECK, 5, Vec3::new(0.0, 0.0, 8.0)),
            (HEAD, 6, Vec3::new(0.0, 0.0, 6.0)),
            (L_THIGH, 1, Vec3::new(0.0, 6.0, -2.0)),
            ("Bip01_L_Calf", 8, Vec3::new(0.0, 0.0, -32.0)),
            ("Bip01_L_Foot", 9, Vec3::new(0.0, 0.0, -32.0)),
            (L_TOE, 10, Vec3::new(8.0, 0.0, -4.0)),
            (R_THIGH, 1, Vec3::new(0.0, -6.0, -2.0)),
            ("Bip01_R_Calf", 12, Vec3::new(0.0, 0.0, -32.0)),
            ("Bip01_R_Foot", 13, Vec3::new(0.0, 0.0, -32.0)),
            (R_TOE, 14, Vec3::new(8.0, 0.0, -4.0)),
        ];
        let names = bones.iter().map(|b| b.0.to_string()).collect();
        let parents = bones.iter().map(|b| b.1).collect();
        let locals = bones.iter().map(|b| Xf { pos: b.2, ..Xf::IDENTITY }).collect();
        (names, parents, locals)
    }

    fn joints(at: &[(&str, Vec3)]) -> Joints {
        let map: HashMap<String, Vec3> = at.iter().map(|(n, p)| (n.to_string(), *p)).collect();
        Joints { at: map.clone(), raw: map }
    }

    #[test]
    fn hips_follow_the_skater() {
        let (names, parents, locals) = figure();
        let mut rig = Rig::new(names, parents);
        assert!(rig.missing().iter().all(|m| m.contains("Clavicle") || m.to_lowercase().contains("arm") || m.contains("Hand")), "{:?}", rig.missing());
        assert!(rig.usable());
        rig.set_base(locals.clone());
        // The skater stands somewhere else, turned 90 degrees.
        let hips = Vec3::new(500.0, 300.0, 72.0);
        let j = joints(&[
            ("HIPS", hips),
            ("NECK", hips + Vec3::new(0.0, 0.0, 35.0)),
            ("LEFTUPLEG", hips + Vec3::new(-6.0, 0.0, -2.0)),
            ("RIGHTUPLEG", hips + Vec3::new(6.0, 0.0, -2.0)),
        ]);
        let component = Xf { pos: Vec3::new(10.0, 0.0, 0.0), ..Xf::IDENTITY };
        let pose = rig.pose(&component, &j, false).unwrap();
        let world: Vec<Xf> = pose.iter().map(|s| component.mul(s)).collect();
        let pelvis = world[1].pos;
        assert!(pelvis.distance(hips) < 0.01, "pelvis at {pelvis}");
        // The thighs now line up with the skater's: left is -x.
        assert!(world[8].pos.x < world[12].pos.x);
        // The head is above the pelvis.
        assert!(world[7].pos.z > pelvis.z + 30.0);
    }

    #[test]
    fn twist_bones_follow_their_limb() {
        let (mut names, mut parents, mut locals) = figure();
        // A thigh twist hanging off the pelvis, halfway down the left thigh.
        names.push("Bip01_LThighTwist".into());
        parents.push(1);
        locals.push(Xf { pos: Vec3::new(0.0, 6.0, -18.0), ..Xf::IDENTITY });
        let mut rig = Rig::new(names, parents);
        rig.set_base(locals);
        let hips = Vec3::new(0.0, 0.0, 70.0);
        let j = joints(&[
            ("HIPS", hips),
            ("NECK", hips + Vec3::new(0.0, 0.0, 35.0)),
            ("LEFTUPLEG", hips + Vec3::new(0.0, 6.0, -2.0)),
            ("RIGHTUPLEG", hips + Vec3::new(0.0, -6.0, -2.0)),
            // Left leg kicked straight forward.
            ("LEFTLEG", hips + Vec3::new(32.0, 6.0, -2.0)),
            ("LEFTFOOT", hips + Vec3::new(64.0, 6.0, -2.0)),
        ]);
        let pose = rig.pose(&Xf::IDENTITY, &j, false).unwrap();
        let (thigh, calf, twist) = (pose[8].pos, pose[9].pos, pose[16].pos);
        let mid = (thigh + calf) * 0.5;
        assert!(twist.distance(mid) < 0.5, "twist at {twist}, thigh middle at {mid}");
    }

    #[test]
    fn standing_height_reads_head_to_toe() {
        let (names, parents, locals) = figure();
        let rig = Rig::new(names, parents);
        let h = rig.standing_height(&locals).unwrap();
        assert!((h - Vec3::new(8.0, 6.0, 113.0).length()).abs() < 0.01, "{h}");
    }

    #[test]
    fn align_matches_direction_and_plane() {
        let q = align(Vec3::X, Vec3::Y, Vec3::Z, Vec3::X);
        assert!((q * Vec3::X).distance(Vec3::Z) < 1e-4);
        assert!((q * Vec3::Y).distance(Vec3::X) < 1e-4);
    }

    #[test]
    fn feet_reach_their_targets() {
        let (names, parents, locals) = figure();
        let mut rig = Rig::new(names, parents);
        rig.set_base(locals);
        let hips = Vec3::new(0.0, 0.0, 60.0);
        let goal = Vec3::new(10.0, 8.0, 4.0);
        let j = joints(&[
            ("HIPS", hips),
            ("NECK", hips + Vec3::new(0.0, 0.0, 35.0)),
            ("LEFTUPLEG", hips + Vec3::new(0.0, 6.0, -2.0)),
            ("RIGHTUPLEG", hips + Vec3::new(0.0, -6.0, -2.0)),
            ("LEFTFOOT", goal),
        ]);
        let pose = rig.pose(&Xf::IDENTITY, &j, true).unwrap();
        assert!(pose[10].pos.distance(goal) < 0.5, "ankle at {}", pose[10].pos);
    }
}
