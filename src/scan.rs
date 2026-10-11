//! The ground around the skater, sampled with the game's ray traces into a
//! height grid and handed to the skate engine as triangles. This is
//! SkateRim's scanner reworked for a game in another process: instead of
//! casting rays itself it hands out batches of rays, and the game's answers
//! arrive a frame later. Distances are in core units (70 to the metre).
use bevy::math::{Vec2, Vec3};
use std::collections::{HashMap, VecDeque};

pub const METRE: f32 = crate::space::CORE_PER_METRE;
const SCALE: f32 = 1.0 / METRE;
/// Game units per inch, for the rail finder's units.
const INCHES_PER_UNIT: f32 = 0.5625;
const START_ABOVE: f32 = 560.0;
/// Rays reach ~40 m down, so the street is found below a rooftop.
const END_BELOW: f32 = 2800.0;
/// Surfaces higher than this above the skater are treated as overhangs
/// (~2 m, so the skater rolls under doorframes and arches).
const HEADROOM: f32 = 140.0;
/// Hits this close to the skater and above their feet are their own body.
const SELF_RADIUS: f32 = 60.0;
const SELF_ABOVE: f32 = 20.0;
/// Height difference that makes a grindable edge (~10 cm: curbs up).
const LIP_DROP: f32 = 7.0;
/// The top of a ledge must be this flat (rise per sample) to ride along.
const LIP_FLAT: f32 = 0.5;
/// Shortest traced rail (~70 cm).
const MIN_GRID_RAIL: f32 = 50.0;
/// Heights above the ground the thin-object probes run at (~35 cm, ~55 cm,
/// ~85 cm, ~1.05 m: planters, benches, railings and stair handrails).
const THIN_LEVELS: [f32; 4] = [24.5, 38.5, 59.5, 73.5];
const THIN_MIN: f32 = 12.0;
const THIN_MAX: f32 = 110.0;
const COLUMN_RAYS: u8 = 10;

/// A ray the game should cast, from and to, in core space.
pub type Ray = (Vec3, Vec3);

#[derive(Clone, Copy)]
struct Column {
    from: f32,
    best: Option<f32>,
    rays: u8,
    done: bool,
}

#[derive(Clone, Copy)]
enum Probe {
    Column(usize),
    /// A railing probe along grid edge number `edge`.
    Level(Vec3, Vec3, usize),
    Top { base: f32, edge: usize },
}

/// A grid point or grid edge, numbered across the whole world so scans
/// centred in different places agree on it.
type PointKey = (i32, i32);
type EdgeKey = (i32, i32, u8);

/// Reference heights within this (~40 cm) count as the same scan height.
const SAME_REFERENCE: f32 = 30.0;

/// What earlier scans found, so the next one only traces new ground. Each
/// entry keeps the height the scan was measured from, since that decides
/// what counts as an overhang.
#[derive(Default)]
pub struct ScanCache {
    spacing: f32,
    columns: HashMap<PointKey, (f32, Option<f32>)>,
    edges: HashMap<EdgeKey, (f32, Vec<Vec3>)>,
}

impl ScanCache {
    pub fn clear(&mut self) {
        self.columns.clear();
        self.edges.clear();
    }
}

pub struct Scan {
    pub center: Vec3,
    spacing: f32,
    half: i32,
    columns: Vec<Column>,
    open: VecDeque<usize>,
    player: Vec2,
    pub rays: usize,
    thin_half: i32,
    edges: Option<Vec<(Vec3, Vec3, usize)>>,
    edge_next: usize,
    /// Each railing edge probed, and the railing tops found along it.
    edge_keys: Vec<EdgeKey>,
    edge_tops: Vec<Vec<Vec3>>,
    /// Railing results reused from earlier scans.
    reuse_edges: HashMap<EdgeKey, Vec<Vec3>>,
    pub reused: usize,
    tops: Vec<(Ray, f32, usize)>,
    thin: Vec<Vec3>,
    pending: Vec<Probe>,
}

pub struct Ground {
    pub center: Vec3,
    /// Skate space: metres, y up.
    pub skate: Vec<[[f32; 3]; 3]>,
    /// The same faces in the rail finder's units: inches, z up.
    pub inches: Vec<[Vec3; 3]>,
    /// Edges traced on the height grid, in skate space.
    pub rails: Vec<Vec<[f32; 3]>>,
    /// The ground heights the faces were made from, in core space.
    pub heights: HeightMap,
}

/// The scanned ground height at each grid point (core units).
#[derive(Clone)]
pub struct HeightMap {
    origin: Vec2,
    spacing: f32,
    side: usize,
    heights: Vec<Option<f32>>,
}

impl HeightMap {
    /// The ground height at the sample nearest `p`.
    pub fn at(&self, p: Vec2) -> Option<f32> {
        let i = ((p.x - self.origin.x) / self.spacing).round() as i64;
        let j = ((p.y - self.origin.y) / self.spacing).round() as i64;
        let side = self.side as i64;
        if i < 0 || j < 0 || i >= side || j >= side {
            return None;
        }
        self.heights[(j * side + i) as usize]
    }

    /// The ground heights within a sample of `p`: (lowest, how many found),
    /// or None if `p` is outside the scanned square.
    pub fn around(&self, p: Vec2) -> Option<(f32, usize)> {
        let i = ((p.x - self.origin.x) / self.spacing).round() as i64;
        let j = ((p.y - self.origin.y) / self.spacing).round() as i64;
        let side = self.side as i64;
        if i < 1 || j < 1 || i >= side - 1 || j >= side - 1 {
            return None;
        }
        let mut lowest = f32::INFINITY;
        let mut found = 0;
        for y in j - 1..=j + 1 {
            for x in i - 1..=i + 1 {
                if let Some(h) = self.heights[(y * side + x) as usize] {
                    lowest = lowest.min(h);
                    found += 1;
                }
            }
        }
        Some((lowest, found))
    }
}

impl Scan {
    pub fn new(center: Vec3, radius_m: f32, spacing_m: f32, thin_radius_m: f32) -> Self {
        let spacing = spacing_m * METRE;
        let half = (radius_m / spacing_m).ceil() as i32;
        let side = (2 * half + 1) as usize;
        let start = Column { from: center.z + START_ABOVE, best: None, rays: 0, done: false };
        Self {
            center,
            spacing,
            half,
            columns: vec![start; side * side],
            open: (0..side * side).collect(),
            player: center.truncate(),
            rays: 0,
            thin_half: ((thin_radius_m / spacing_m).ceil() as i32).min(half),
            edges: None,
            edge_next: 0,
            edge_keys: Vec::new(),
            edge_tops: Vec::new(),
            reuse_edges: HashMap::new(),
            reused: 0,
            tops: Vec::new(),
            thin: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// A scan that takes whatever earlier scans already found here (at the
    /// same height) and only traces the rest.
    pub fn with_cache(center: Vec3, radius_m: f32, spacing_m: f32, thin_radius_m: f32, cache: &ScanCache) -> Self {
        let mut scan = Self::new(center, radius_m, spacing_m, thin_radius_m);
        if (cache.spacing - scan.spacing).abs() > 1e-3 {
            return scan;
        }
        let side = scan.side();
        let same = |reference: f32| (reference - center.z).abs() <= SAME_REFERENCE;
        let mut open = VecDeque::new();
        for index in 0..side * side {
            match cache.columns.get(&scan.key(index % side, index / side)) {
                Some(&(reference, best)) if same(reference) => {
                    let c = &mut scan.columns[index];
                    c.best = best;
                    c.done = true;
                    scan.reused += 1;
                }
                _ => open.push_back(index),
            }
        }
        scan.open = open;
        scan.reuse_edges = cache
            .edges
            .iter()
            .filter(|(_, (reference, _))| same(*reference))
            .map(|(k, (_, tops))| (*k, tops.clone()))
            .collect();
        scan
    }

    /// Saves what this (finished) scan found for the next one, and forgets
    /// what is now far away.
    pub fn store(&self, cache: &mut ScanCache) {
        if (cache.spacing - self.spacing).abs() > 1e-3 {
            cache.clear();
            cache.spacing = self.spacing;
        }
        let side = self.side();
        for (index, c) in self.columns.iter().enumerate() {
            if c.done {
                cache.columns.insert(self.key(index % side, index / side), (self.center.z, c.best));
            }
        }
        for (key, tops) in self.edge_keys.iter().zip(&self.edge_tops) {
            cache.edges.insert(*key, (self.center.z, tops.clone()));
        }
        let (cx, cy) = self.key(side / 2, side / 2);
        let reach = self.half * 3;
        cache.columns.retain(|&(x, y), _| (x - cx).abs() <= reach && (y - cy).abs() <= reach);
        cache.edges.retain(|&(x, y, _), _| (x - cx).abs() <= reach && (y - cy).abs() <= reach);
    }

    /// The world-wide number of grid point (i, j).
    fn key(&self, i: usize, j: usize) -> PointKey {
        let snap = |v: f32| (v / self.spacing).round() as i32;
        (snap(self.center.x) + i as i32 - self.half, snap(self.center.y) + j as i32 - self.half)
    }

    fn side(&self) -> usize {
        (2 * self.half + 1) as usize
    }

    fn point(&self, i: usize, j: usize) -> Vec2 {
        let snap = |v: f32| (v / self.spacing).round() * self.spacing;
        Vec2::new(
            snap(self.center.x) + (i as i32 - self.half) as f32 * self.spacing,
            snap(self.center.y) + (j as i32 - self.half) as f32 * self.spacing,
        )
    }

    pub fn done(&self) -> bool {
        self.open.is_empty()
            && self.pending.is_empty()
            && self.tops.is_empty()
            && self.edges.as_ref().is_some_and(|e| self.edge_next >= e.len())
    }

    /// The next rays to cast, at most `budget`. Call `absorb` with their
    /// results before asking again.
    pub fn batch(&mut self, budget: usize) -> Vec<Ray> {
        debug_assert!(self.pending.is_empty());
        let mut out = Vec::new();
        let bottom = self.center.z - END_BELOW;
        let side = self.side();
        // Every open column casts its next ray down from where the last one hit.
        let n = self.open.len().min(budget);
        for _ in 0..n {
            let Some(index) = self.open.pop_front() else { break };
            let xy = self.point(index % side, index / side);
            let c = self.columns[index];
            out.push((xy.extend(c.from), xy.extend(bottom)));
            self.pending.push(Probe::Column(index));
        }
        while out.len() < budget {
            let Some((ray, base, edge)) = self.tops.pop() else { break };
            out.push(ray);
            self.pending.push(Probe::Top { base, edge });
        }
        // Railing probes need the finished height grid.
        if self.open.is_empty() && !self.pending.iter().any(|p| matches!(p, Probe::Column(_))) {
            if self.edges.is_none() {
                self.edges = Some(self.thin_edges());
            }
            while out.len() < budget {
                let Some(&(a, b, edge)) = self.edges.as_ref().and_then(|e| e.get(self.edge_next)) else { break };
                self.edge_next += 1;
                out.push((a, b));
                self.pending.push(Probe::Level(a, b, edge));
            }
        }
        self.rays += out.len();
        out
    }

    /// The game's answers to the last batch: the hit point and whether what
    /// was hit is solid ground, for each ray in order.
    pub fn absorb(&mut self, hits: &[Option<Vec3>]) {
        let pending = std::mem::take(&mut self.pending);
        let side = self.side();
        for (k, probe) in pending.into_iter().enumerate() {
            let hit = hits.get(k).copied().flatten();
            match probe {
                Probe::Column(index) => {
                    let xy = self.point(index % side, index / side);
                    let reference = self.center.z;
                    let near = xy.distance(self.player) < SELF_RADIUS;
                    let c = &mut self.columns[index];
                    c.rays += 1;
                    match hit {
                        None => c.done = true,
                        Some(p) => {
                            c.from = p.z - 4.0;
                            if !(near && p.z > reference + SELF_ABOVE) {
                                c.best = Some(p.z);
                                if p.z <= reference + HEADROOM {
                                    c.done = true;
                                }
                            }
                        }
                    }
                    if c.rays >= COLUMN_RAYS {
                        c.done = true;
                    }
                    if !c.done {
                        self.open.push_back(index);
                    }
                }
                Probe::Level(a, b, edge) => {
                    let Some(p) = hit else { continue };
                    let step = (b - a).normalize_or_zero() * 3.0;
                    let probe = p.truncate() + step.truncate();
                    let base = self.base_of(a, b);
                    self.tops.push(((probe.extend(base + THIN_MAX + 40.0), probe.extend(base - 10.0)), base, edge));
                }
                Probe::Top { base, edge } => {
                    let Some(top) = hit else { continue };
                    let rise = top.z - base;
                    if (THIN_MIN..=THIN_MAX).contains(&rise) {
                        self.thin.push(top);
                        if let Some(tops) = self.edge_tops.get_mut(edge) {
                            tops.push(top);
                        }
                    }
                }
            }
        }
    }

    /// Level rays along every grid edge near the skater whose ends are on
    /// roughly the same ground; edges an earlier scan probed give their
    /// railings straight away instead.
    fn thin_edges(&mut self) -> Vec<(Vec3, Vec3, usize)> {
        let mut edges = Vec::new();
        if self.thin_half <= 0 {
            return edges;
        }
        let side = self.side() as i32;
        let (lo, hi) = (self.half - self.thin_half, self.half + self.thin_half);
        for j in lo.max(0)..=hi.min(side - 1) {
            for i in lo.max(0)..=hi.min(side - 1) {
                let Some(ha) = self.height(i, j) else { continue };
                for (di, dj) in [(1, 0), (0, 1)] {
                    let Some(hb) = self.height(i + di, j + dj) else { continue };
                    // Steps rise up to ~85 cm between samples on a stair.
                    if (ha - hb).abs() > 60.0 {
                        continue;
                    }
                    let base = ha.max(hb);
                    let (gx, gy) = self.key(i as usize, j as usize);
                    let key = (gx, gy, di as u8);
                    if let Some(tops) = self.reuse_edges.get(&key) {
                        self.thin.extend(tops.iter().copied());
                        self.edge_keys.push(key);
                        self.edge_tops.push(tops.clone());
                        continue;
                    }
                    let edge = self.edge_keys.len();
                    self.edge_keys.push(key);
                    self.edge_tops.push(Vec::new());
                    let pa = self.point(i as usize, j as usize);
                    let pb = self.point((i + di) as usize, (j + dj) as usize);
                    for level in THIN_LEVELS {
                        edges.push((pa.extend(base + level), pb.extend(base + level), edge));
                    }
                }
            }
        }
        edges
    }

    fn base_of(&self, a: Vec3, b: Vec3) -> f32 {
        let ground = |p: Vec3| {
            let i = ((p.x - self.point(0, 0).x) / self.spacing).round() as i32;
            let j = ((p.y - self.point(0, 0).y) / self.spacing).round() as i32;
            self.height(i, j)
        };
        match (ground(a), ground(b)) {
            (Some(x), Some(y)) => x.max(y),
            _ => a.z - THIN_LEVELS[0],
        }
    }

    fn height(&self, i: i32, j: i32) -> Option<f32> {
        let side = self.side() as i32;
        if i < 0 || j < 0 || i >= side || j >= side {
            return None;
        }
        self.columns[(j * side + i) as usize].best
    }

    pub fn ground(&self) -> Ground {
        let side = self.side();
        let at = |i: usize, j: usize| -> Option<Vec3> {
            let z = self.columns[j * side + i].best?;
            Some(self.point(i, j).extend(z))
        };
        let mut faces: Vec<[Vec3; 3]> = Vec::new();
        for j in 0..side - 1 {
            for i in 0..side - 1 {
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                match (a, b, c, d) {
                    (Some(a), Some(b), Some(c), Some(d)) => {
                        if (a.z - c.z).abs() <= (b.z - d.z).abs() {
                            faces.push([a, b, c]);
                            faces.push([a, c, d]);
                        } else {
                            faces.push([a, b, d]);
                            faces.push([b, c, d]);
                        }
                    }
                    (Some(a), Some(b), Some(c), None) => faces.push([a, b, c]),
                    (Some(a), Some(b), None, Some(d)) => faces.push([a, b, d]),
                    (Some(a), None, Some(c), Some(d)) => faces.push([a, c, d]),
                    (None, Some(b), Some(c), Some(d)) => faces.push([b, c, d]),
                    _ => {}
                }
            }
        }
        Ground {
            heights: HeightMap {
                origin: self.point(0, 0),
                spacing: self.spacing,
                side,
                heights: self.columns.iter().map(|c| c.best).collect(),
            },
            center: self.center,
            skate: faces.iter().map(|f| f.map(|p| to_skate(p).to_array())).collect(),
            inches: faces.iter().map(|f| f.map(|p| p * INCHES_PER_UNIT)).collect(),
            rails: self
                .grid_rails()
                .into_iter()
                .chain(chain_points(self.thin.clone(), self.spacing))
                .map(|rail| rail.into_iter().map(|p| to_skate(p).to_array()).collect())
                .collect(),
        }
    }

    fn grid_rails(&self) -> Vec<Vec<Vec3>> {
        let side = self.side() as i32;
        let dirs = [(1, 0), (0, 1), (-1, 0), (0, -1)];
        let mut points: Vec<Vec3> = Vec::new();
        for j in 0..side {
            for i in 0..side {
                let Some(h) = self.height(i, j) else { continue };
                let here = self.point(i as usize, j as usize);
                for (k, (di, dj)) in dirs.iter().enumerate() {
                    let Some(below) = self.height(i + di, j + dj) else { continue };
                    if h - below < LIP_DROP {
                        continue;
                    }
                    let (bi, bj) = dirs[(k + 2) % 4];
                    let flat = self
                        .height(i + bi, j + bj)
                        .is_some_and(|back| (back - h).abs() <= LIP_FLAT * self.spacing || h - back >= LIP_DROP);
                    if !flat {
                        continue;
                    }
                    let out = Vec2::new(*di as f32, *dj as f32);
                    points.push((here + out * self.spacing * 0.45).extend(h));
                }
            }
        }
        chain_points(points, self.spacing)
    }
}

/// Joins nearby edge points into polylines, smooths and straightens them.
fn chain_points(points: Vec<Vec3>, spacing: f32) -> Vec<Vec<Vec3>> {
    let reach = spacing * 1.05;
    let cell = |p: Vec3| ((p.x / spacing).floor() as i32, (p.y / spacing).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        grid.entry(cell(*p)).or_default().push(i);
    }
    let neighbours = |i: usize| -> Vec<usize> {
        let p = points[i];
        let (cx, cy) = cell(p);
        let mut out = Vec::new();
        for x in cx - 2..=cx + 2 {
            for y in cy - 2..=cy + 2 {
                for &k in grid.get(&(x, y)).into_iter().flatten() {
                    let q = points[k];
                    if k != i && q.truncate().distance(p.truncate()) <= reach && (q.z - p.z).abs() < 20.0 {
                        out.push(k);
                    }
                }
            }
        }
        out
    };
    let links: Vec<Vec<usize>> = (0..points.len()).map(neighbours).collect();
    let mut used = vec![false; points.len()];
    let mut rails = Vec::new();
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_by_key(|&i| links[i].len());
    for start in order {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain = vec![points[start]];
        let mut tip = start;
        let mut heading: Option<Vec3> = None;
        loop {
            let next = links[tip].iter().copied().filter(|&k| !used[k]).max_by(|&a, &b| {
                let score = |k: usize| {
                    let d = (points[k] - points[tip]).normalize_or_zero();
                    heading.map_or(0.0, |h| h.dot(d)) - (points[k] - points[tip]).length() / spacing * 0.1
                };
                score(a).total_cmp(&score(b))
            });
            let Some(next) = next else { break };
            let step = (points[next] - points[tip]).normalize_or_zero();
            if heading.is_some_and(|h| h.dot(step) < 0.0) {
                break;
            }
            heading = Some(heading.map_or(step, |h| (h * 2.0 + step).normalize_or_zero()));
            used[next] = true;
            chain.push(points[next]);
            tip = next;
        }
        if chain.len() < 3 {
            continue;
        }
        for rail in split_at_turns(simplify(smooth(&chain), 6.0)) {
            if length(&rail) >= MIN_GRID_RAIL {
                rails.push(rail);
            }
        }
    }
    rails
}

fn smooth(chain: &[Vec3]) -> Vec<Vec3> {
    let n = chain.len();
    (0..n)
        .map(|i| {
            if i == 0 || i + 1 == n {
                return chain[i];
            }
            let lo = i.saturating_sub(2);
            let hi = (i + 2).min(n - 1);
            let sum: Vec3 = chain[lo..=hi].iter().copied().sum();
            sum / (hi - lo + 1) as f32
        })
        .collect()
}

fn simplify(points: Vec<Vec3>, tolerance: f32) -> Vec<Vec3> {
    if points.len() < 3 {
        return points;
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let line = b - a;
    let (mut far, mut index) = (0.0, 0);
    for (i, p) in points.iter().enumerate().skip(1).take(points.len() - 2) {
        let t = if line.length_squared() > 0.0 { ((*p - a).dot(line) / line.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        let d = p.distance(a + line * t);
        if d > far {
            far = d;
            index = i;
        }
    }
    if far <= tolerance {
        return vec![a, b];
    }
    let mut left = simplify(points[..=index].to_vec(), tolerance);
    let right = simplify(points[index..].to_vec(), tolerance);
    left.pop();
    left.extend(right);
    left
}

fn split_at_turns(points: Vec<Vec3>) -> Vec<Vec<Vec3>> {
    let mut out = vec![vec![points[0]]];
    for w in 1..points.len() {
        let current = out.last_mut().unwrap();
        if current.len() >= 2 {
            let a = (current[current.len() - 1] - current[current.len() - 2]).normalize_or_zero();
            let b = (points[w] - current[current.len() - 1]).normalize_or_zero();
            if a.dot(b) < 0.75 {
                let joint = current[current.len() - 1];
                out.push(vec![joint]);
            }
        }
        out.last_mut().unwrap().push(points[w]);
    }
    out.into_iter().filter(|r| r.len() >= 2).collect()
}

fn length(points: &[Vec3]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// Core units, z up, to Skate's metres with y up.
pub fn to_skate(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, -p.y) * SCALE
}

pub fn from_skate(p: Vec3) -> Vec3 {
    Vec3::new(p.x, -p.z, p.y) / SCALE
}

/// Skate's direction basis seen from core space: skate y is core z.
pub fn skate_dir_to_core(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y)
}

pub fn inches_to_skate(p: Vec3) -> [f32; 3] {
    to_skate(p / INCHES_PER_UNIT).to_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a scan to completion against a height function.
    fn run(scan: &mut Scan, ground: impl Fn(Vec2) -> Option<f32>) {
        for _ in 0..10_000 {
            if scan.done() {
                return;
            }
            let rays = scan.batch(500);
            let hits: Vec<Option<Vec3>> = rays
                .iter()
                .map(|(a, b)| {
                    if (a.z - b.z).abs() > 1.0 {
                        // A downward ray hits the ground if the ground is below its start.
                        let z = ground(a.truncate())?;
                        (z <= a.z && z >= b.z).then(|| a.truncate().extend(z))
                    } else {
                        None
                    }
                })
                .collect();
            scan.absorb(&hits);
        }
        panic!("scan never finished");
    }

    #[test]
    fn flat_ground_becomes_a_full_grid() {
        let mut scan = Scan::new(Vec3::new(10.0, 20.0, 100.0), 4.0, 0.5, 2.0);
        run(&mut scan, |_| Some(100.0));
        let g = scan.ground();
        let side = scan.side();
        assert_eq!(g.skate.len(), 2 * (side - 1) * (side - 1));
        assert!(g.rails.is_empty());
    }

    #[test]
    fn a_second_scan_nearby_reuses_the_first() {
        let ground = |p: Vec2| Some(if p.x > 200.0 { 35.0 } else { 0.0 });
        let mut first = Scan::new(Vec3::ZERO, 8.0, 0.5, 2.0);
        run(&mut first, ground);
        let mut cache = ScanCache::default();
        first.store(&mut cache);
        // A metre along: most of the ground is already known.
        let mut second = Scan::with_cache(Vec3::new(70.0, 0.0, 0.0), 8.0, 0.5, 2.0, &cache);
        run(&mut second, ground);
        assert!(second.reused * 2 > second.columns.len(), "reused {} of {}", second.reused, second.columns.len());
        assert!(second.rays * 2 < first.rays, "first {} rays, second {}", first.rays, second.rays);
        // And it found the same ground as a fresh scan would.
        let mut fresh = Scan::new(Vec3::new(70.0, 0.0, 0.0), 8.0, 0.5, 2.0);
        run(&mut fresh, ground);
        assert_eq!(second.ground().skate.len(), fresh.ground().skate.len());
        assert_eq!(second.ground().rails.len(), fresh.ground().rails.len());
        // A scan from a different height doesn't trust the cache.
        let high = Scan::with_cache(Vec3::new(70.0, 0.0, 300.0), 8.0, 0.5, 2.0, &cache);
        assert_eq!(high.reused, 0);
    }

    #[test]
    fn a_ledge_gives_a_rail() {
        let mut scan = Scan::new(Vec3::ZERO, 8.0, 0.5, 0.0);
        run(&mut scan, |p| Some(if p.x > 40.0 { 35.0 } else { 0.0 }));
        let g = scan.ground();
        assert!(!g.rails.is_empty());
    }

    #[test]
    fn overhangs_are_looked_through() {
        // A roof 3 m up over the middle: the scan should find the floor.
        let mut scan = Scan::new(Vec3::ZERO, 2.0, 0.5, 0.0);
        let roof = |p: Vec2| p.length() < 60.0;
        for _ in 0..1000 {
            if scan.done() {
                break;
            }
            let rays = scan.batch(100);
            let hits: Vec<Option<Vec3>> = rays
                .iter()
                .map(|(a, b)| {
                    if (a.z - b.z).abs() < 1.0 {
                        return None;
                    }
                    if roof(a.truncate()) && a.z > 210.0 {
                        Some(a.truncate().extend(210.0))
                    } else {
                        (a.z >= 0.0).then(|| a.truncate().extend(0.0))
                    }
                })
                .collect();
            scan.absorb(&hits);
        }
        assert!(scan.done());
        let side = scan.side();
        let mid = scan.columns[(side / 2) * side + side / 2];
        assert_eq!(mid.best, Some(0.0));
    }
}
