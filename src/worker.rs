//! The skate simulation runs on its own thread, as in the mashup: the game
//! thread sends input and collision, the worker publishes poses.
use crate::scan::inches_to_skate;
use bevy::math::Vec3;
use skate_host::bridge::{InputFrame, Pose, PreparedCollision, Score, Session};
use std::sync::mpsc;

pub struct Collision {
    pub skate: Vec<[[f32; 3]; 3]>,
    pub inches: Vec<[Vec3; 3]>,
    /// Rails already traced on the height grid, in skate space.
    pub grid_rails: Vec<Vec<[f32; 3]>>,
    pub rails: bool,
}

pub enum Job {
    Activate { epoch: u64, spawn: [f32; 3], heading: f32, collision: Collision },
    Collision(Collision),
    Step { epoch: u64, dt: f32, input: InputFrame },
    Suspend,
}

pub enum Reply {
    Ready(u128),
    Activated(u64, Pose),
    /// Sent every step: a new pose when the simulation ticked, and how far
    /// into the next tick the game's clock is, for interpolation.
    Frame { epoch: u64, pose: Option<(Pose, Score)>, alpha: f32 },
    Installed { triangles: usize, rails: usize, ms: u128 },
    Error(String),
}

pub struct Worker {
    pub send: mpsc::Sender<Job>,
    pub receive: mpsc::Receiver<Reply>,
}

/// A floor far below the world: the session wants collision before the
/// real ground around the skater is known.
fn placeholder() -> Vec<[[f32; 3]; 3]> {
    let (y, s) = (-5000.0, 10.0);
    vec![[[-s, y, -s], [-s, y, s], [s, y, s]], [[-s, y, -s], [s, y, s], [s, y, -s]]]
}

fn rails_for(collision: &Collision) -> Vec<Vec<[f32; 3]>> {
    if !collision.rails {
        return Vec::new();
    }
    let (found, census) = crate::rails::find(&collision.inches);
    let mut rails: Vec<Vec<[f32; 3]>> =
        found.into_iter().map(|rail| rail.into_iter().map(inches_to_skate).collect()).collect();
    crate::log!(
        "rails: {} from {} edges ({} lips, {} runs), {} traced on the grid",
        census.rails,
        census.candidates,
        census.lips,
        census.runs,
        collision.grid_rails.len()
    );
    rails.extend(collision.grid_rails.iter().cloned());
    rails.truncate(u16::MAX as usize);
    rails
}

pub fn spawn(root: std::path::PathBuf) -> Result<Worker, String> {
    let (send, jobs) = mpsc::channel::<Job>();
    let (publish, receive) = mpsc::channel::<Reply>();
    std::thread::Builder::new()
        .name("skatecity-skate".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let error = publish.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || run(root, jobs, publish)));
            let message = match outcome {
                Ok(Ok(())) => return,
                Ok(Err(e)) => e,
                Err(panic) => panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into()),
            };
            let _ = error.send(Reply::Error(message));
        })
        .map_err(|e| e.to_string())?;
    Ok(Worker { send, receive })
}

fn run(root: std::path::PathBuf, jobs: mpsc::Receiver<Job>, publish: mpsc::Sender<Reply>) -> Result<(), String> {
    let started = std::time::Instant::now();
    let mut session = Session::new(&root, placeholder(), Vec::new(), [0.0, -4990.0, 0.0], 0.0)?;
    session.set_aspect_ratio(16.0 / 9.0);
    if publish.send(Reply::Ready(started.elapsed().as_millis())).is_err() {
        return Ok(());
    }
    let builder = session.collision_builder();
    let (build_send, build_jobs) = mpsc::channel::<Collision>();
    let (built_send, built) = mpsc::channel::<(Result<PreparedCollision, String>, usize, usize, u128)>();
    std::thread::Builder::new()
        .name("skatecity-collision".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            while let Ok(mut collision) = build_jobs.recv() {
                while let Ok(newer) = build_jobs.try_recv() {
                    collision = newer;
                }
                let start = std::time::Instant::now();
                let rails = rails_for(&collision);
                let counts = (collision.skate.len(), rails.len());
                let prepared = builder.build(collision.skate, rails);
                if built_send.send((prepared, counts.0, counts.1, start.elapsed().as_millis())).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let mut epoch = 0;
    let mut accumulated = 0.0f32;
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Activate { epoch: new_epoch, spawn, heading, collision } => {
                epoch = new_epoch;
                accumulated = 0.0;
                let start = std::time::Instant::now();
                let rails = rails_for(&collision);
                let counts = (collision.skate.len(), rails.len());
                session.install_collision(builder.build(collision.skate, rails)?)?;
                let _ = publish.send(Reply::Installed {
                    triangles: counts.0,
                    rails: counts.1,
                    ms: start.elapsed().as_millis(),
                });
                let pose = session.activate(spawn, heading)?;
                if publish.send(Reply::Activated(epoch, pose)).is_err() {
                    break;
                }
            }
            Job::Collision(collision) => {
                let _ = build_send.send(collision);
            }
            Job::Suspend => {
                accumulated = 0.0;
                session.suspend_input();
            }
            Job::Step { epoch: request, dt, input } => {
                if let Ok((prepared, triangles, rails, ms)) = built.try_recv() {
                    match prepared {
                        Ok(prepared) => {
                            session.install_collision(prepared)?;
                            let _ = publish.send(Reply::Installed { triangles, rails, ms });
                        }
                        Err(e) => crate::log!("collision build failed: {e}"),
                    }
                }
                if request != epoch {
                    continue;
                }
                session.collect(input, dt);
                accumulated = (accumulated + dt).min(0.15);
                let mut advanced = false;
                while accumulated >= session.period() {
                    accumulated -= session.period();
                    session.advance()?;
                    advanced = true;
                }
                let pose = if advanced {
                    let pose = session.pose();
                    if !pose.root.is_finite() || pose.bones.iter().any(|b| !b.is_finite()) {
                        return Err("the skate simulation produced a non-finite pose".into());
                    }
                    Some((pose, session.score()))
                } else {
                    None
                };
                let alpha = (accumulated / session.period()).clamp(0.0, 1.0);
                if publish.send(Reply::Frame { epoch, pose, alpha }).is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}
