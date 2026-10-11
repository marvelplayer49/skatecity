//! SkateCity.ini, next to the helper.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub skate_assets: PathBuf,
    pub toggle_key: i32,
    pub scan_radius: f32,
    pub scan_spacing: f32,
    pub rays_per_frame: usize,
    pub start_rays_per_frame: usize,
    pub thin_radius: f32,
    pub rails: bool,
    pub pose: bool,
    pub plant_feet: bool,
    pub board: bool,
    pub skate_camera: bool,
    pub body_scale: f32,
    pub score_display: bool,
    /// Unreal units per metre; 0 works it out from Batman's height.
    pub units_per_metre: f32,
    pub batman_height: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // The mashup's release unzipped in Downloads, unless the ini says otherwise.
            skate_assets: std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join(
                "Downloads\\2010-Rust-Rewrite-Mashup-windows-x64\\2010-Rust-Rewrite-Mashup\\skate-data\\assets",
            ),
            toggle_key: 0x4B,
            scan_radius: 16.0,
            scan_spacing: 0.5,
            rays_per_frame: 1000,
            start_rays_per_frame: 4000,
            thin_radius: 8.0,
            rails: true,
            pose: true,
            plant_feet: true,
            board: true,
            skate_camera: true,
            body_scale: 1.05,
            score_display: true,
            units_per_metre: 0.0,
            batman_height: 1.62,
        }
    }
}

fn number(value: &str) -> Option<f32> {
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok().map(|v| v as f32);
    }
    value.parse().ok()
}

impl Config {
    pub fn load(path: &Path) -> Self {
        let mut c = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            crate::log!("no {}; using defaults", path.display());
            return c;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with([';', '#', '[']) {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            let flag = matches!(value, "1" | "true" | "yes" | "on");
            let n = number(value);
            match key.as_str() {
                "skate_assets" => c.skate_assets = PathBuf::from(value.trim_matches('"')),
                "toggle_key" => c.toggle_key = n.map_or(c.toggle_key, |v| v as i32),
                "scan_radius" => c.scan_radius = n.unwrap_or(c.scan_radius).clamp(4.0, 40.0),
                "scan_spacing" => c.scan_spacing = n.unwrap_or(c.scan_spacing).clamp(0.25, 2.0),
                "rays_per_frame" => c.rays_per_frame = n.map_or(c.rays_per_frame, |v| v.clamp(20.0, 4000.0) as usize),
                "start_rays_per_frame" => {
                    c.start_rays_per_frame = n.map_or(c.start_rays_per_frame, |v| v.clamp(100.0, 4096.0) as usize)
                }
                "thin_radius" => c.thin_radius = n.unwrap_or(c.thin_radius).clamp(0.0, 30.0),
                "rails" => c.rails = flag,
                "pose" => c.pose = flag,
                "plant_feet" => c.plant_feet = flag,
                "board" => c.board = flag,
                "skate_camera" => c.skate_camera = flag,
                "body_scale" => c.body_scale = n.unwrap_or(c.body_scale).clamp(0.5, 2.0),
                "score_display" => c.score_display = flag,
                "units_per_metre" => c.units_per_metre = n.unwrap_or(0.0).clamp(0.0, 1000.0),
                "batman_height" => c.batman_height = n.unwrap_or(c.batman_height).clamp(1.0, 2.5),
                // Read by the game script.
                "board_grip_material" | "board_metal_material" | "board_wheel_material" => {}
                _ => crate::log!("unknown setting {key}"),
            }
        }
        c
    }
}
