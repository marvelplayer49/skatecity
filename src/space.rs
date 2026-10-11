//! Coordinate spaces. The skating code from SkateRim works in "core" space:
//! Skyrim's units (70 to the metre), z up, right-handed. Unreal is z up too
//! but left-handed (y points right of x), so going between them mirrors y,
//! which flips the sense of every rotation, and rescales the units.
use bevy::math::{Mat3, Mat4, Quat, Vec3};

use crate::ipc::Atom;

pub const CORE_PER_METRE: f32 = 70.0;
pub const RUU_PER_RADIAN: f32 = 32768.0 / std::f32::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct Space {
    /// Core units per Unreal unit.
    pub k: f32,
}

impl Space {
    pub fn new(ue_units_per_metre: f32) -> Self {
        Self { k: CORE_PER_METRE / ue_units_per_metre.max(1.0) }
    }
    pub fn ue_units_per_metre(&self) -> f32 {
        CORE_PER_METRE / self.k
    }
    pub fn to_core(&self, p: Vec3) -> Vec3 {
        Vec3::new(p.x, -p.y, p.z) * self.k
    }
    pub fn to_ue(&self, p: Vec3) -> Vec3 {
        Vec3::new(p.x, -p.y, p.z) / self.k
    }
    pub fn dir_to_ue(&self, d: Vec3) -> Vec3 {
        Vec3::new(d.x, -d.y, d.z)
    }
    pub fn xf_to_core(&self, x: &Xf) -> Xf {
        Xf { rot: mirror(x.rot), pos: self.to_core(x.pos), scale: x.scale }
    }
    pub fn xf_to_ue(&self, x: &Xf) -> Xf {
        Xf { rot: mirror(x.rot), pos: self.to_ue(x.pos), scale: x.scale }
    }
}

/// A rotation as seen in the y-mirrored space.
pub fn mirror(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, q.y, -q.z, q.w)
}

/// A rigid transform with uniform scale: p -> rot * (scale * p) + pos.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub rot: Quat,
    pub pos: Vec3,
    pub scale: f32,
}

impl Default for Xf {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Xf {
    pub const IDENTITY: Xf = Xf { rot: Quat::IDENTITY, pos: Vec3::ZERO, scale: 1.0 };

    pub fn mul(&self, b: &Xf) -> Xf {
        Xf { rot: (self.rot * b.rot).normalize(), pos: self.pos + self.rot * (b.pos * self.scale), scale: self.scale * b.scale }
    }
    pub fn inverse(&self) -> Xf {
        let rot = self.rot.inverse();
        let scale = if self.scale.abs() > 1e-6 { 1.0 / self.scale } else { 1.0 };
        Xf { rot, pos: rot * (-self.pos) * scale, scale }
    }
    #[allow(dead_code)]
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.pos + self.rot * (p * self.scale)
    }
    pub fn from_atom(a: &Atom) -> Xf {
        let q = Quat::from_xyzw(a.quat[0], a.quat[1], a.quat[2], a.quat[3]);
        let rot = if q.length_squared() > 1e-8 { q.normalize() } else { Quat::IDENTITY };
        Xf { rot, pos: Vec3::from_array(a.pos), scale: if a.scale.abs() > 1e-6 { a.scale } else { 1.0 } }
    }
    pub fn to_atom(&self) -> Atom {
        Atom { quat: self.rot.to_array(), pos: self.pos.to_array(), scale: self.scale }
    }
    /// From an Unreal FMatrix laid out in memory (rows are the axes).
    pub fn from_ue_matrix(m: &[f32; 16]) -> Xf {
        let m = Mat4::from_cols_array(m);
        let x = m.x_axis.truncate();
        let scale = x.length().max(1e-6);
        let basis = Mat3::from_cols(x / scale, m.y_axis.truncate().normalize_or_zero(), m.z_axis.truncate().normalize_or_zero());
        Xf { rot: Quat::from_mat3(&basis).normalize(), pos: m.w_axis.truncate(), scale }
    }
    pub fn is_finite(&self) -> bool {
        self.rot.is_finite() && self.pos.is_finite() && self.scale.is_finite()
    }
}

/// Unreal's FRotationMatrix as a column-vector basis (columns are the
/// rotated x, y and z axes), from pitch, yaw and roll in Unreal units.
pub fn rotator_basis(rot: [i32; 3]) -> Mat3 {
    let (p, y, r) = (rot[0] as f32 / RUU_PER_RADIAN, rot[1] as f32 / RUU_PER_RADIAN, rot[2] as f32 / RUU_PER_RADIAN);
    let (sp, cp) = p.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sr, cr) = r.sin_cos();
    Mat3::from_cols(
        Vec3::new(cp * cy, cp * sy, sp),
        Vec3::new(sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp),
        Vec3::new(-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp),
    )
}

/// Unreal's FMatrix::Rotator: pitch, yaw, roll in Unreal units from a basis.
pub fn basis_rotator(m: Mat3) -> [i32; 3] {
    let x = m.x_axis;
    let pitch = x.z.atan2((x.x * x.x + x.y * x.y).sqrt());
    let yaw = x.y.atan2(x.x);
    let ruu = |a: f32| (a * RUU_PER_RADIAN).round() as i32;
    let sy = rotator_basis([ruu(pitch), ruu(yaw), 0]).y_axis;
    let roll = m.z_axis.dot(sy).atan2(m.y_axis.dot(sy));
    [ruu(pitch), ruu(yaw), ruu(roll)]
}

/// An Unreal actor transform from location and rotation.
pub fn actor_xf(loc: [f32; 3], rot: [i32; 3]) -> Xf {
    Xf { rot: Quat::from_mat3(&rotator_basis(rot)).normalize(), pos: Vec3::from_array(loc), scale: 1.0 }
}

/// The rotator that points x along `forward` with z as near `up` as possible.
pub fn look_rotator(forward: Vec3, up: Vec3) -> Option<[i32; 3]> {
    let x = forward.try_normalize()?;
    let y = up.cross(x).try_normalize()?;
    let z = x.cross(y);
    Some(basis_rotator(Mat3::from_cols(x, y, z)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotators_round_trip() {
        for rot in [[0, 0, 0], [2000, 16384, 0], [-3000, -9000, 1200], [8000, 30000, -4000]] {
            let back = basis_rotator(rotator_basis(rot));
            for i in 0..3 {
                assert!((back[i] - rot[i]).abs() <= 2, "{rot:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn mirroring_keeps_points_and_rotations_consistent() {
        let s = Space::new(100.0);
        let ue = Xf { rot: Quat::from_axis_angle(Vec3::new(0.3, -0.5, 0.8).normalize(), 0.9), pos: Vec3::new(10.0, 20.0, -5.0), scale: 1.0 };
        let p = Vec3::new(3.0, -7.0, 2.0);
        let core = s.xf_to_core(&ue);
        let a = s.to_core(ue.point(p));
        let b = core.point(s.to_core(p));
        assert!(a.distance(b) < 1e-3, "{a} vs {b}");
    }

    #[test]
    fn transforms_compose_and_invert() {
        let a = Xf { rot: Quat::from_rotation_z(0.7), pos: Vec3::new(1.0, 2.0, 3.0), scale: 1.5 };
        let b = Xf { rot: Quat::from_rotation_x(-0.4), pos: Vec3::new(-2.0, 0.5, 1.0), scale: 0.8 };
        let p = Vec3::new(0.3, -1.0, 2.0);
        assert!(a.mul(&b).point(p).distance(a.point(b.point(p))) < 1e-4);
        assert!(a.inverse().point(a.point(p)).distance(p) < 1e-4);
    }

    #[test]
    fn ue_matrix_reads_rows_as_axes() {
        let basis = rotator_basis([0, 16384, 0]);
        let mut m = [0.0f32; 16];
        m[..3].copy_from_slice(&basis.x_axis.to_array());
        m[4..7].copy_from_slice(&basis.y_axis.to_array());
        m[8..11].copy_from_slice(&basis.z_axis.to_array());
        m[12..15].copy_from_slice(&[5.0, 6.0, 7.0]);
        m[15] = 1.0;
        let x = Xf::from_ue_matrix(&m);
        // Yaw 90 degrees points x along +y in Unreal.
        assert!(x.rot.mul_vec3(Vec3::X).distance(Vec3::Y) < 1e-4);
        assert!(x.point(Vec3::ZERO).distance(Vec3::new(5.0, 6.0, 7.0)) < 1e-4);
    }
}
