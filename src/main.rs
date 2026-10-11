//! SkateCity: Skate 3's skating for Batman: Arkham City.
//!
//! The game is 32-bit and the skate engine is 64-bit, so this helper runs
//! the engine in its own process. SkateCity.cs (a BmSDK script) starts it
//! with the game's process id, and the two talk over shared memory named
//! after that id: the script reports Batman and casts rays, the helper
//! answers with where Batman, his bones, the board and the camera go.
#![windows_subsystem = "windows"]

mod config;
mod ipc;
mod log;
mod mode;
mod os;
mod rails;
mod rig;
mod scan;
mod score;
mod slam;
mod space;
mod worker;

use std::path::PathBuf;

fn main() {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default();
    log::open(&dir.join("SkateCity.log"));
    crate::log!("SkateCity {} starting", env!("CARGO_PKG_VERSION"));
    if let Err(e) = run(&dir) {
        crate::log!("stopped: {e}");
    }
}

fn run(dir: &std::path::Path) -> Result<(), String> {
    let pid: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).ok_or("usage: SkateCity <game process id>")?;
    let base = format!("Local\\SkateCity_{pid}");
    let shared = os::open_shared(&base)?;
    let requests = os::Event::open(&format!("{base}_req"))?;
    let replies = os::Event::open(&format!("{base}_rep"))?;
    let game = os::Process::open(pid).ok_or("the game process could not be opened")?;
    let block = unsafe { ipc::Block::new(shared.0) };
    let config = config::Config::load(&dir.join("SkateCity.ini"));
    crate::log!("attached to the game (process {pid}); settings {config:?}");
    let mut mode = mode::Mode::new(config);
    let mut last_seq = 0u32;
    let mut beat = std::time::Instant::now();
    loop {
        if beat.elapsed().as_secs() >= 10 {
            beat = std::time::Instant::now();
            let (step, frames) = block.breadcrumb();
            crate::log!("game alive: script frame {frames}, step {step}");
        }
        if !requests.wait(500) {
            if game.exited() {
                let (step, frames) = block.breadcrumb();
                crate::log!("the game has closed; the script's last step was {step} in frame {frames} ({})", step_name(step));
                return Ok(());
            }
            continue;
        }
        let req = block.request();
        if req.magic != ipc::MAGIC || req.seq == last_seq {
            continue;
        }
        last_seq = req.seq;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mode.frame(&block, &req)));
        match outcome {
            Ok(reply) => block.write_reply(&reply),
            Err(panic) => {
                let message = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                crate::log!("frame failed: {message}");
                return Err(message);
            }
        }
        replies.set();
    }
}

/// The script's frame steps, as SkateCity.cs numbers them.
fn step_name(step: u32) -> &'static str {
    match step {
        0 => "never ran",
        1 => "starting a frame",
        2 => "finding Batman",
        3 => "reading Batman's position",
        4 => "reading his collision size",
        5 => "reading his mesh transform",
        6 => "reading his bone count",
        7 => "reading his bone names",
        8 => "copying his bones",
        9 => "waiting for the helper",
        10 => "reading the reply",
        11 => "tracing rays",
        12 => "starting or stopping skating",
        13 => "moving Batman",
        14 => "writing his bones",
        15 => "moving the camera",
        16 => "placing the board",
        17 => "reading his bone parents",
        18 => "gliding",
        19 => "playing sounds",
        20 => "between frames (the frame had finished)",
        30 => "drawing the HUD",
        31 => "between frames (HUD drawn)",
        _ => "unknown",
    }
}
