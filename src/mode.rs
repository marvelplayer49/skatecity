//! Skate mode, frame by frame: what the game reports comes in, what Batman,
//! the camera and the board should do goes back out.
use crate::config::Config;
use crate::ipc::{self, Block, BoardPart, Reply, Request};
use crate::rig::{Joints, Rig};
use crate::scan::{HeightMap, Scan, ScanCache, from_skate, skate_dir_to_core, to_skate};
use crate::score::Line;
use crate::slam::{Hit, SlamEnd, Slams};
use crate::challenge::{Challenges, Event as ChallengeEvent};
use crate::space::{Space, Xf, actor_xf, look_rotator, RUU_PER_RADIAN};
use crate::worker::{self, Collision, Job, Reply as Event, Worker};
use bevy::math::Vec3;
use skate_host::bridge::{InputFrame, Pose};

const BOTH_STICKS: u16 = 0x0040 | 0x0080;
/// LB: glide when in the air.
const GLIDE_BUTTON: u16 = 0x0100;
/// D-pad right: a different challenge.
const SKIP_BUTTON: u16 = 0x0008;
/// RB plus the left stick forward or back in the air: a front or back flip.
const FLIP_BUTTON: u16 = 0x0200;
const FLIP_STICK: i16 = 20000;
/// Points for each flip landed.
const FLIP_POINTS: f32 = 500.0;
const GRAVITY: f32 = 9.81;

/// A flip in progress: +1 front, -1 back.
struct Flip {
    dir: f32,
    elapsed: f32,
    duration: f32,
    /// Flips finished in this air, for naming a double.
    done: u32,
}
const DEFAULT_UNITS_PER_METRE: f32 = 100.0;
/// The board is dropped in from this high (~30 cm) so it lands on the
/// ground rather than starting inside it.
const SPAWN_DROP: f32 = 20.0;
/// Tricks aren't scored for this many frames after getting on the board,
/// so dropping onto it doesn't count as a bailed ollie.
const SETTLE_FRAMES: u32 = 60;
/// Being this far (~1.4 m) under the scanned ground means the skater went
/// through it: put them back.
const FELL_THROUGH: f32 = 100.0;
/// Falling this far (~50 m) with no ground found underneath: put them back.
const FELL_INTO_VOID: f32 = 3500.0;

const PARTS: [&str; 7] = [
    "SKATEBOARD_ROOT",
    "TRUCK_FRONT",
    "TRUCK_BACK",
    "LEFT_WHEELFRONT",
    "RIGHT_WHEELFRONT",
    "LEFT_WHEELBACK",
    "RIGHT_WHEELBACK",
];

/// One simulation tick, in core space.
struct Tick {
    tick: u64,
    root: Vec3,
    joints: Joints,
    board: Vec<(&'static str, Xf)>,
    /// Position, forward and up.
    camera: Option<(Vec3, Vec3, Vec3)>,
    velocity: Vec3,
    state: String,
}

impl Tick {
    fn from_pose(pose: &Pose, scale: f32) -> Self {
        let camera = pose.camera.and_then(|(position, basis, _fov)| {
            let forward = skate_dir_to_core(basis.z_axis).try_normalize()?;
            let up = skate_dir_to_core(basis.y_axis).try_normalize()?;
            Some((from_skate(position), forward, up))
        });
        Self {
            tick: pose.tick,
            root: from_skate(pose.root.w_axis.truncate()),
            joints: Joints::from_pose(pose, scale),
            board: PARTS.iter().filter_map(|n| Some((*n, Joints::transform(pose, n)?))).collect(),
            camera,
            velocity: skate_dir_to_core(pose.velocity),
            state: pose.state.clone(),
        }
    }

    fn part(&self, name: &str) -> Option<Vec3> {
        self.board.iter().find(|(n, _)| *n == name).map(|(_, x)| x.pos)
    }

    fn forward(&self) -> Option<Vec3> {
        if self.velocity.length() > 1.0 {
            return Some(self.velocity);
        }
        Some(self.part("TRUCK_FRONT")? - self.part("TRUCK_BACK")?)
    }
}

/// What the skater looks like this frame, between two ticks.
struct View {
    root: Vec3,
    joints: Joints,
    board: Vec<(&'static str, Vec3)>,
    camera: Option<(Vec3, Vec3, Vec3)>,
    forward: Option<Vec3>,
}

fn blend(a: &Tick, b: &Tick, t: f32) -> View {
    let board = b
        .board
        .iter()
        .map(|(name, xb)| (*name, a.part(name).unwrap_or(xb.pos).lerp(xb.pos, t)))
        .collect();
    let camera = match (a.camera, b.camera) {
        (Some((pa, fa, ua)), Some((pb, fb, ub))) => {
            Some((pa.lerp(pb, t), fa.lerp(fb, t).normalize_or(fb), ua.lerp(ub, t).normalize_or(ub)))
        }
        (_, c) => c,
    };
    View { root: a.root.lerp(b.root, t), joints: a.joints.lerp(&b.joints, t), board, camera, forward: b.forward() }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Owner {
    Start,
    Stream,
}

struct Outstanding {
    id: u32,
    rays: Vec<ipc::Ray>,
    owner: Owner,
}

enum Phase {
    Idle,
    Scanning { scan: Scan, at: Vec3, heading: f32 },
    Activating,
    Riding,
}

pub struct Mode {
    config: Config,
    space: Space,
    worker: Option<Worker>,
    failed: bool,
    ready: bool,
    phase: Phase,
    epoch: u64,
    previous: Option<Tick>,
    current: Option<Tick>,
    alpha: f32,
    view: Option<View>,
    rig: Option<Rig>,
    skel_seq: u32,
    stream: Option<Scan>,
    installed_at: Option<Vec3>,
    next_batch: u32,
    outstanding: Option<Outstanding>,
    sticks_held: bool,
    toggles_seen: Option<u32>,
    world: u32,
    packet: u32,
    msg_seq: u32,
    msg: String,
    line: Line,
    logged_tick: u64,
    restarts: u32,
    frames: u64,
    no_pad_warned: bool,
    posed_logged: bool,
    /// Where the skater last had wheels on the ground, and their heading.
    last_ground: Option<(Vec3, f32)>,
    /// The ground the skate engine has now.
    heights: Option<HeightMap>,
    /// Falling with no ground scanned below: scan faster.
    urgent: bool,
    glide_held: bool,
    /// Batman left the board to glide; LB again drops him.
    gliding: bool,
    glide_seq: u32,
    glide_end_seq: u32,
    glide_vel: Vec3,
    /// Frames since skating began, and pose snapshots written this run.
    ride_frames: u32,
    snapshots: u32,
    /// Ground scans installed so far, and the count when the skater was
    /// first seen under the scanned ground.
    installs: u32,
    under_since: Option<u32>,
    /// The last finished line, for the result banner.
    result_seq: u32,
    result_kind: u32,
    result_points: u32,
    result_text: String,
    flip: Option<Flip>,
    flips_this_air: u32,
    /// Ground already scanned, reused by the next scan.
    cache: ScanCache,
    /// Scales the riding ray budget down when the game's frame rate drops.
    ray_scale: f32,
    slams: Slams,
    best_slam: i64,
    /// Sounds sent so far, the latest few, and the tick of the last body hit
    /// (so a tumble isn't a wall of noise).
    sound_count: u32,
    sounds: [ipc::SoundEvent; ipc::SOUND_SLOTS],
    last_hit_sound: u64,
    challenges: Challenges,
    skip_held: bool,
    /// Which local player this is (0 is player 1).
    player: usize,
}

impl Mode {
    pub fn new(config: Config, player: usize) -> Self {
        let space = Space::new(if config.units_per_metre > 0.0 { config.units_per_metre } else { DEFAULT_UNITS_PER_METRE });
        Self {
            config,
            space,
            worker: None,
            failed: false,
            ready: false,
            phase: Phase::Idle,
            epoch: 0,
            previous: None,
            current: None,
            alpha: 0.0,
            view: None,
            rig: None,
            skel_seq: 0,
            stream: None,
            installed_at: None,
            next_batch: 1,
            outstanding: None,
            sticks_held: false,
            toggles_seen: None,
            world: 0,
            packet: 0,
            msg_seq: 0,
            msg: String::new(),
            line: Line::default(),
            logged_tick: 0,
            restarts: 0,
            frames: 0,
            no_pad_warned: false,
            posed_logged: false,
            last_ground: None,
            heights: None,
            urgent: false,
            glide_held: false,
            gliding: false,
            glide_seq: 0,
            glide_end_seq: 0,
            glide_vel: Vec3::ZERO,
            ride_frames: 0,
            snapshots: 0,
            installs: 0,
            under_since: None,
            result_seq: 0,
            result_kind: 0,
            result_points: 0,
            result_text: String::new(),
            flip: None,
            flips_this_air: 0,
            cache: ScanCache::default(),
            ray_scale: 1.0,
            slams: Slams::default(),
            best_slam: 0,
            sound_count: 0,
            sounds: [ipc::SoundEvent::default(); ipc::SOUND_SLOTS],
            last_hit_sound: 0,
            challenges: Challenges::default(),
            skip_held: false,
            player,
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.msg = text.into();
        self.msg_seq += 1;
        crate::log!("message: {}", self.msg);
    }

    fn riding(&self) -> bool {
        matches!(self.phase, Phase::Riding)
    }

    /// Handles one request from the game and builds the reply.
    pub fn frame(&mut self, block: &Block, req: &Request) -> Reply {
        self.frames += 1;
        if self.worker.is_none() && !self.failed {
            crate::log!("loading Skate 3 data from {}", self.config.skate_assets.display());
            match worker::spawn(self.config.skate_assets.clone()) {
                Ok(w) => self.worker = Some(w),
                Err(e) => {
                    crate::log!("could not start the skate thread: {e}");
                    self.say(format!("SkateCity: could not start the skate engine ({e})"));
                    self.failed = true;
                }
            }
        }
        self.read_skeleton(block, req);
        self.read_hits(block, req);
        // Trace less per frame while the game runs below ~45 fps.
        if req.dt > 1.0 / 45.0 {
            self.ray_scale = (self.ray_scale * 0.9).max(0.2);
        } else {
            self.ray_scale = (self.ray_scale + 0.02).min(1.0);
        }

        // Toggle: the game counts presses of the toggle key; both sticks
        // clicked together are read here.
        let focused = crate::os::game_focused(req.pid);
        let pad = if focused { crate::os::player_pad(self.player) } else { None };
        let buttons = pad.map_or(0, |p| p.pad.buttons);
        let sticks = buttons & BOTH_STICKS == BOTH_STICKS;
        let mut pressed = sticks && !self.sticks_held;
        self.sticks_held = sticks;
        if let Some(seen) = self.toggles_seen {
            pressed |= req.toggles != seen;
        }
        self.toggles_seen = Some(req.toggles);

        self.receive();

        if req.world != self.world {
            self.world = req.world;
            self.cache.clear();
            self.challenges.reset();
            self.stop("the game loaded a new area");
        }
        let pawn = req.flags & ipc::PAWN_VALID != 0;
        if !pawn && !matches!(self.phase, Phase::Idle) {
            self.stop("the game has no player right now");
        }
        if pressed && pawn {
            if matches!(self.phase, Phase::Idle) {
                self.gliding = false;
                self.start(block, req);
            } else {
                self.stop("toggled off");
            }
        }

        // LB in the air: off the board and into Batman's glide.
        let glide = buttons & GLIDE_BUTTON != 0;
        let glide_pressed = glide && !self.glide_held;
        self.glide_held = glide;
        if glide_pressed && pawn {
            if self.riding() && self.airborne() {
                self.start_glide();
            } else if self.gliding && matches!(self.phase, Phase::Idle) {
                self.gliding = false;
                self.glide_end_seq += 1;
                self.say("Dropping");
            }
        }

        if self.riding() {
            let input = match pad {
                Some(state) => {
                    self.packet = self.packet.wrapping_add(1);
                    let p = state.pad;
                    InputFrame::from_pad(p.buttons, [p.left_trigger, p.right_trigger], [p.lx, p.ly], [p.rx, p.ry], self.packet)
                }
                None => {
                    if !self.no_pad_warned && focused {
                        self.no_pad_warned = true;
                        self.say("SkateCity: no controller found - skating needs an Xbox-style controller");
                    }
                    InputFrame::neutral()
                }
            };
            let step = Job::Step { epoch: self.epoch, dt: req.dt.clamp(0.0, 0.1), input };
            if self.worker.as_ref().is_some_and(|w| w.send.send(step).is_err()) {
                self.stop("skate thread stopped");
            }
            self.ride_frames = self.ride_frames.saturating_add(1);
            self.refresh_view();
            let (stick, held) = pad.map_or((0, false), |p| (p.pad.ly, p.pad.buttons & FLIP_BUTTON != 0));
            self.update_flip(req.dt.clamp(0.0, 0.1), held, stick);
            let skip_down = buttons & SKIP_BUTTON != 0;
            let skip = skip_down && !self.skip_held;
            self.skip_held = skip_down;
            if let Some((state, root)) = self.current.as_ref().map(|t| (t.state.clone(), t.root)) {
                let events = self.challenges.update(req.dt.clamp(0.0, 0.1), &state, root, skip);
                self.challenge_events(events);
            }
            self.stream_ground();
            self.catch_fall();
        }
        self.advance_start();

        let mut reply = Reply { seq: req.seq, ..Reply::default() };
        reply.state = if self.failed {
            ipc::FAILED
        } else if !self.ready {
            ipc::LOADING
        } else {
            match self.phase {
                Phase::Idle => ipc::READY,
                Phase::Scanning { .. } | Phase::Activating => ipc::STARTING,
                Phase::Riding => ipc::RIDING,
            }
        };
        reply.units_per_metre = self.space.ue_units_per_metre();
        reply.toggle_key = self.config.toggle_key as u32;
        reply.glide_seq = self.glide_seq;
        reply.glide_end_seq = self.glide_end_seq;
        reply.glide_vel = self.glide_vel.to_array();
        reply.sound_count = self.sound_count;
        reply.sounds = self.sounds;
        self.issue_rays(block, &mut reply);
        if self.riding() {
            self.fill_outputs(block, req, &mut reply);
        }
        reply.msg_seq = self.msg_seq;
        ipc::write_text(&mut reply.msg, &self.msg);
        if self.config.score_display {
            if self.riding() {
                ipc::write_text(&mut reply.score, &self.line.current());
                reply.hud_points = self.line.points().max(0) as u32;
            }
            reply.hud_total = self.line.total().max(0) as u32;
            reply.result_seq = self.result_seq;
            if self.riding() {
                if let Some((kind, at, text)) = self.current.as_ref().and_then(|t| self.challenges.hud(t.root)) {
                    reply.chal_kind = kind as u32;
                    reply.chal_loc = self.space.to_ue(at).to_array();
                    ipc::write_text(&mut reply.chal_text, &text);
                }
                reply.chal_done = self.challenges.done;
            }
            reply.result_kind = self.result_kind;
            reply.result_points = self.result_points;
            ipc::write_text(&mut reply.result_text, &self.result_text);
        }
        reply
    }

    fn read_skeleton(&mut self, block: &Block, req: &Request) {
        if req.flags & ipc::SKELETON_VALID == 0 || req.skel_seq == self.skel_seq {
            return;
        }
        self.skel_seq = req.skel_seq;
        let n = (req.bone_count as usize).min(ipc::MAX_BONES);
        let rig = Rig::new(block.bone_names(n), block.parents(n));
        crate::log!("skeleton: {} bones, usable {}, missing {:?}", rig.len(), rig.usable(), rig.missing());
        if self.config.units_per_metre > 0.0 {
            self.space = Space::new(self.config.units_per_metre);
        } else {
            let locals: Vec<Xf> = block.locals(n).iter().map(Xf::from_atom).collect();
            // Two estimates: the skeleton's toe-to-head span, and the
            // collision capsule (about a person's height, 1.85 m, tall).
            let from_bones = rig.standing_height(&locals).map(|h| h / self.config.batman_height);
            let from_capsule = (req.half_height > 1.0).then(|| req.half_height * 2.0 / 1.85);
            // The capsule is steady; the skeleton's span changes with
            // whatever animation is playing, so it's only a fallback.
            let chosen = from_capsule.or(from_bones).filter(|u| (20.0..=600.0).contains(u));
            match chosen {
                Some(u) => {
                    self.space = Space::new(u);
                    crate::log!(
                        "size: skeleton says {from_bones:?}, capsule says {from_capsule:?}; using {u:.1} units to the metre"
                    );
                }
                None => crate::log!("could not measure the character; assuming {DEFAULT_UNITS_PER_METRE} units to the metre"),
            }
        }
        self.rig = Some(rig);
    }

    fn read_hits(&mut self, block: &Block, req: &Request) {
        let Some(out) = &self.outstanding else { return };
        if req.ray_batch != out.id {
            return;
        }
        let hits: Vec<Option<Vec3>> = block
            .hits(req.ray_count as usize)
            .iter()
            .map(|h| (h.hit != 0).then(|| self.space.to_core(Vec3::from_array(h.pos))))
            .collect();
        let owner = out.owner;
        self.outstanding = None;
        match owner {
            Owner::Start => {
                if let Phase::Scanning { scan, .. } = &mut self.phase {
                    scan.absorb(&hits);
                }
            }
            Owner::Stream => {
                if let Some(scan) = &mut self.stream {
                    scan.absorb(&hits);
                }
            }
        }
    }

    /// Hands the game the next batch of rays, or the unanswered one again.
    fn issue_rays(&mut self, block: &Block, reply: &mut Reply) {
        if let Some(out) = &self.outstanding {
            let alive = match out.owner {
                Owner::Start => matches!(self.phase, Phase::Scanning { .. }),
                Owner::Stream => self.stream.is_some(),
            };
            if !alive {
                self.outstanding = None;
            }
        }
        if self.outstanding.is_none() {
            let (scan, owner, budget) = match &mut self.phase {
                Phase::Scanning { scan, .. } => (Some(scan), Owner::Start, self.config.start_rays_per_frame),
                _ if self.urgent => (self.stream.as_mut(), Owner::Stream, self.config.start_rays_per_frame),
                _ => {
                    let budget = (self.config.rays_per_frame as f32 * self.ray_scale) as usize;
                    (self.stream.as_mut(), Owner::Stream, budget.max(50))
                }
            };
            if let Some(scan) = scan {
                if !scan.done() {
                    let rays = scan.batch(budget.min(ipc::MAX_RAYS));
                    if !rays.is_empty() {
                        let s = self.space;
                        let rays = rays
                            .iter()
                            .map(|(a, b)| ipc::Ray { from: s.to_ue(*a).to_array(), to: s.to_ue(*b).to_array() })
                            .collect();
                        self.outstanding = Some(Outstanding { id: self.next_batch, rays, owner });
                        self.next_batch = self.next_batch.wrapping_add(1).max(1);
                    }
                }
            }
        }
        if let Some(out) = &self.outstanding {
            block.write_rays(&out.rays);
            reply.ray_batch = out.id;
            reply.ray_count = out.rays.len() as u32;
        }
    }

    fn receive(&mut self) {
        let mut events = Vec::new();
        if let Some(w) = &self.worker {
            loop {
                match w.receive.try_recv() {
                    Ok(r) => events.push(r),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        events.push(Event::Error("the skate thread exited".into()));
                        break;
                    }
                }
            }
        }
        for event in events {
            match event {
                Event::Ready(ms) => {
                    self.ready = true;
                    crate::log!("Skate 3 session ready in {ms}ms");
                    self.say("SkateCity ready: press K or click both sticks to skate");
                }
                Event::Installed { triangles, rails, ms } => {
                    crate::log!("collision installed: {triangles} triangles, {rails} rails, built in {ms}ms");
                }
                Event::Activated(epoch, pose) if epoch == self.epoch && matches!(self.phase, Phase::Activating) => {
                    self.phase = Phase::Riding;
                    self.posed_logged = false;
                    self.ride_frames = 0;
                    crate::log!("skating; state {}", pose.state);
                    self.say("Skating - press K or click both sticks to stop");
                    self.previous = None;
                    self.current = None;
                    self.accept(pose);
                    self.alpha = 1.0;
                    self.refresh_view();
                }
                Event::Frame { epoch, pose, alpha } if epoch == self.epoch && self.riding() => {
                    if let Some((pose, score)) = pose {
                        self.accept(pose);
                        if self.ride_frames < SETTLE_FRAMES {
                            self.line.discard();
                        } else if let Some(end) = {
                            let state = self.current.as_ref().map_or(String::new(), |t| t.state.clone());
                            self.line.update(&score, &state)
                        } {
                            self.result_seq += 1;
                            self.result_kind = end.kind;
                            self.result_points = end.points.max(0) as u32;
                            self.result_text = end.names;
                            let bailed = end.kind == crate::score::BAILED;
                            if let Some(event) = self.challenges.line_ended(end.points, bailed) {
                                self.challenge_events(vec![event]);
                            }
                        }
                    }
                    self.alpha = alpha;
                }
                Event::Error(e) => {
                    crate::log!("skate error: {e}; reloading the session");
                    self.say("SkateCity: the skater bailed hard - reloading, try again in a few seconds");
                    self.stop("error");
                    self.worker = None;
                    self.ready = false;
                    self.restarts += 1;
                    if self.restarts > 20 {
                        crate::log!("too many errors; skating disabled until the game restarts");
                        self.failed = true;
                    }
                    break;
                }
                _ => {}
            }
        }
    }

    fn accept(&mut self, pose: Pose) {
        let tick = Tick::from_pose(&pose, self.config.body_scale);
        if pose.state.contains("Ground") && !pose.state.contains("Wipeout") {
            let heading = tick
                .forward()
                .and_then(|f| f.truncate().try_normalize())
                .map_or(0.0, |f| std::f32::consts::PI - f.x.atan2(f.y));
            self.last_ground = Some((tick.root, heading));
        }
        if tick.tick / 240 != self.logged_tick / 240 {
            self.logged_tick = tick.tick;
            crate::log!("tick {} speed {:.2} m/s state {}", pose.tick, pose.velocity.length(), pose.state);
        }
        self.sound_cues(&tick);
        let (hits, end) = self.slams.update(tick.tick, &tick.state, tick.root, tick.velocity.length(), &tick.joints);
        for hit in hits {
            self.hit_sound(hit, tick.tick);
        }
        if let Some(end) = end {
            self.show_slam(end);
        }
        self.previous = self.current.take();
        self.current = Some(tick);
    }

    fn sound(&mut self, kind: u32, at: Vec3) {
        let slot = self.sound_count as usize % ipc::SOUND_SLOTS;
        self.sounds[slot] = ipc::SoundEvent { kind, loc: self.space.to_ue(at).to_array() };
        self.sound_count = self.sound_count.wrapping_add(1);
    }

    /// Pops, landings and grinds, from the skater's state changing.
    fn sound_cues(&mut self, tick: &Tick) {
        let Some(prev) = self.current.as_ref().map(|t| t.state.clone()) else { return };
        let state = &tick.state;
        let wipeout = state.contains("Wipeout") || prev.contains("Wipeout");
        if wipeout {
            return;
        }
        let (was_air, air) = (prev.contains("Air"), state.contains("Air"));
        if !was_air && air && tick.velocity.z > 1.0 {
            self.sound(ipc::SOUND_POP, tick.root);
        } else if was_air && !air {
            self.sound(ipc::SOUND_LAND, tick.root);
        }
        if !prev.contains("Grind") && state.contains("Grind") {
            self.sound(ipc::SOUND_GRIND, tick.root);
        }
    }

    /// Body hits during a wipeout, thinned out so a tumble doesn't rattle.
    fn hit_sound(&mut self, hit: Hit, tick: u64) {
        let (kind, at, gap) = match hit {
            Hit::Break(at) => (ipc::SOUND_BREAK, at, 0),
            Hit::Hard(at) => (ipc::SOUND_HARD_HIT, at, 4),
            Hit::Light(at) => (ipc::SOUND_HIT, at, 8),
        };
        if tick < self.last_hit_sound + gap {
            return;
        }
        self.last_hit_sound = tick;
        self.sound(kind, at);
    }

    fn challenge_events(&mut self, events: Vec<ChallengeEvent>) {
        for event in events {
            match event {
                ChallengeEvent::Message(text) => self.say(text),
                ChallengeEvent::Completed { title, points } => {
                    crate::log!("challenge done: {title}, {points} pts");
                    self.line.award(points);
                    self.result_seq += 1;
                    self.result_kind = crate::score::CHALLENGE;
                    self.result_points = points.max(0) as u32;
                    self.result_text = format!("{title}|Challenges done: {}", self.challenges.done);
                }
            }
        }
    }

    /// The slam card: score, broken bones and how the fall went.
    fn show_slam(&mut self, end: SlamEnd) {
        crate::log!(
            "wipeout: {} pts, {} hits, {} broken, fell {:.1} m, top {:.1} m/s, tumbled {:.1} m",
            end.score, end.hits, end.breaks.len(), end.fall, end.top_speed, end.tumble
        );
        // Tipping over at a standstill isn't a slam.
        if end.hits == 0 && end.breaks.is_empty() && end.fall < 0.5 && end.top_speed < 2.0 {
            return;
        }
        let injuries = if end.breaks.is_empty() { "Nothing broken".to_string() } else { end.breaks.join(", ") };
        let mut text = format!(
            "{injuries}|Fell {:.1} m   Top speed {:.1} m/s   Tumbled {:.1} m",
            end.fall, end.top_speed, end.tumble
        );
        if end.score > self.best_slam {
            self.best_slam = end.score;
            text.push_str("|NEW BEST SLAM");
        }
        crate::log!("slam: {} pts, {} hits; {}", end.score, end.hits, text.replace('|', "; "));
        if self.config.score_display {
            self.result_seq += 1;
            self.result_kind = crate::score::SLAM;
            self.result_points = end.score.max(0) as u32;
            self.result_text = text;
        }
    }

    fn refresh_view(&mut self) {
        self.view = match (&self.previous, &self.current) {
            (Some(a), Some(b)) => Some(blend(a, b, self.alpha)),
            (None, Some(b)) => Some(blend(b, b, 1.0)),
            _ => None,
        };
    }

    fn start(&mut self, block: &Block, req: &Request) {
        if self.failed {
            self.say("SkateCity: skating is unavailable - see SkateCity.log");
            return;
        }
        if !self.ready {
            self.say("SkateCity is still loading Skate 3 data");
            return;
        }
        let feet = Vec3::from_array(req.actor_loc) - Vec3::Z * req.half_height;
        let at = self.space.to_core(feet);
        let yaw = req.actor_rot[1] as f32 / RUU_PER_RADIAN;
        // Core heading is measured like Skyrim's: clockwise from +y.
        let core_heading = yaw.cos().atan2(-yaw.sin());
        let heading = std::f32::consts::PI - core_heading;
        if let Some(rig) = &mut self.rig {
            let n = rig.len();
            let s = self.space;
            rig.set_base(block.locals(n).iter().map(|a| s.xf_to_core(&Xf::from_atom(a))).collect());
        }
        crate::log!("starting at {feet:?} (core {at:?}), yaw {:.1} degrees", yaw.to_degrees());
        let scan = Scan::with_cache(at, self.config.scan_radius, self.config.scan_spacing, self.config.thin_radius, &self.cache);
        self.phase = Phase::Scanning { scan, at, heading };
        self.outstanding = None;
        self.last_ground = None;
        self.slams.reset();
    }

    /// If the skater is under the scanned ground, they went through it, and
    /// if they have fallen a long way with nothing found below, they left
    /// the world: rescan where they last rode and drop them back in.
    /// Jumping off a building is neither: they're above the street.
    fn catch_fall(&mut self) {
        let (Some(view), Some((ground, heading))) = (&self.view, self.last_ground) else { return };
        let root = view.root;
        let below = self.heights.as_ref().and_then(|h| h.around(root.truncate()));
        let under = matches!(below, Some((lowest, found)) if found >= 5 && root.z < lowest - FELL_THROUGH);
        // Tumbling off a roof edge can briefly put the skater under ground
        // scanned from the roof (ledges and cornices read as roof). Only a
        // scan taken after they dropped, still saying they're underground,
        // means they really went through.
        let through = match (under, self.under_since) {
            (false, _) => {
                self.under_since = None;
                false
            }
            (true, None) => {
                self.under_since = Some(self.installs);
                false
            }
            (true, Some(since)) => self.installs > since,
        };
        let void = root.z < ground.z - FELL_INTO_VOID && !matches!(below, Some((_, found)) if found > 0);
        if !through && !void {
            return;
        }
        self.under_since = None;
        crate::log!("fell {} at {root:?}; putting the skater back at {ground:?}", if through { "through the ground" } else { "out of the world" });
        self.say("Fell through - back on your board");
        if let Some(w) = &self.worker {
            let _ = w.send.send(Job::Suspend);
        }
        self.epoch += 1;
        self.previous = None;
        self.current = None;
        self.view = None;
        self.stream = None;
        self.outstanding = None;
        self.last_ground = None;
        self.slams.reset();
        let scan = Scan::new(ground, self.config.scan_radius, self.config.scan_spacing, self.config.thin_radius);
        self.cache.clear();
        self.phase = Phase::Scanning { scan, at: ground, heading };
    }

    /// Seconds until the skater reaches the scanned ground below, falling
    /// freely; None if there's no ground scanned under them.
    fn time_to_land(&self) -> Option<f32> {
        let tick = self.current.as_ref()?;
        let ground = self.heights.as_ref()?.at(tick.root.truncate())?;
        let height = ((tick.root.z - ground) / crate::scan::METRE).max(0.0);
        let up = tick.velocity.z;
        Some((up + (up * up + 2.0 * GRAVITY * height).sqrt()) / GRAVITY)
    }

    /// Starts, runs and finishes flips; a finished one joins the trick line.
    fn update_flip(&mut self, dt: f32, held: bool, stick: i16) {
        let airborne = self.airborne() && !self.current.as_ref().is_some_and(|t| t.state.contains("Wipeout"));
        if !airborne {
            self.flips_this_air = 0;
        }
        if let Some(flip) = &mut self.flip {
            flip.elapsed += dt;
            if !airborne {
                crate::log!("flip cut short by the landing at {:.0}%", 100.0 * flip.elapsed / flip.duration);
                self.flip = None;
            } else if flip.elapsed >= flip.duration {
                let flip = self.flip.take().unwrap();
                self.flips_this_air = flip.done + 1;
                let base = if flip.dir > 0.0 { "Frontflip" } else { "Backflip" };
                let name = match self.flips_this_air {
                    1 => base.to_string(),
                    2 => format!("Double {base}"),
                    n => format!("{n}x {base}"),
                };
                // Each further flip in the same air is worth more.
                self.line.bonus(&name, FLIP_POINTS * self.flips_this_air as f32);
                crate::log!("flip landed in the air: {name}");
            }
            return;
        }
        if !airborne || !held || stick.unsigned_abs() < FLIP_STICK as u16 || self.ride_frames < SETTLE_FRAMES {
            return;
        }
        // Enough air for a whole turn, with a little to spare to land.
        let time = self.time_to_land().unwrap_or(1.5);
        if time < 0.6 {
            return;
        }
        let duration = (time - 0.15).clamp(0.5, 1.0);
        let dir = if stick > 0 { 1.0 } else { -1.0 };
        crate::log!("{} flip: {:.2}s with {:.2}s of air", if dir > 0.0 { "front" } else { "back" }, duration, time);
        self.flip = Some(Flip { dir, elapsed: 0.0, duration, done: self.flips_this_air });
    }

    /// How far round the flip is: the rotation to apply to the skater and
    /// board about their hips.
    fn flip_turn(&self, view: &View) -> Option<(Vec3, bevy::math::Quat)> {
        let flip = self.flip.as_ref()?;
        let u = (flip.elapsed / flip.duration).clamp(0.0, 1.0);
        let eased = u * u * (3.0 - 2.0 * u);
        let forward = view.forward.map(|f| Vec3::new(f.x, f.y, 0.0)).and_then(|f| f.try_normalize()).unwrap_or(Vec3::X);
        let axis = Vec3::Z.cross(forward).normalize_or(Vec3::Y) * flip.dir;
        let pivot = view.joints.get("HIPS").unwrap_or(view.root);
        Some((pivot, bevy::math::Quat::from_axis_angle(axis, eased * std::f32::consts::TAU)))
    }

    fn airborne(&self) -> bool {
        self.current.as_ref().is_some_and(|t| t.state.contains("Air"))
    }

    /// Ends skating and tells the game to put Batman into his glide, moving
    /// as fast as the board was.
    fn start_glide(&mut self) {
        let v = self.current.as_ref().map_or(Vec3::ZERO, |t| t.velocity);
        let u = self.space.ue_units_per_metre();
        self.glide_vel = Vec3::new(v.x, -v.y, v.z) * u;
        crate::log!("gliding at {:.1} m/s", v.length());
        self.stop("gliding");
        self.gliding = true;
        self.glide_seq += 1;
        self.say("Gliding - LB to drop; you'll be back on the board when you land");
    }

    /// Once the first scan is in, hands it to the skate thread.
    fn advance_start(&mut self) {
        let Phase::Scanning { scan, at, heading } = &self.phase else { return };
        if !scan.done() {
            return;
        }
        let (at, heading) = (*at, *heading);
        let ground = scan.ground();
        scan.store(&mut self.cache);
        crate::log!("scanned {} triangles and {} rails with {} rays", ground.skate.len(), ground.rails.len(), scan.rays);
        if ground.skate.is_empty() {
            self.phase = Phase::Idle;
            self.say("SkateCity: no ground to skate on here");
            return;
        }
        self.installed_at = Some(ground.center);
        self.heights = Some(ground.heights.clone());
        self.challenges.set_world(ground.heights.clone(), ground.rails_core.clone());
        self.installs += 1;
        self.epoch += 1;
        let collision = Collision { skate: ground.skate, inches: ground.inches, grid_rails: ground.rails, rails: self.config.rails };
        if let Some(w) = &self.worker {
            let _ = w.send.send(Job::Activate { epoch: self.epoch, spawn: to_skate(at + Vec3::Z * SPAWN_DROP).to_array(), heading, collision });
        }
        self.phase = Phase::Activating;
    }

    fn stop(&mut self, why: &str) {
        if matches!(self.phase, Phase::Idle) {
            return;
        }
        crate::log!("skating stopped: {why}");
        if let Some(end) = self.slams.finish() {
            self.show_slam(end);
        }
        self.slams.reset();
        if self.riding() {
            self.say("Stopped skating");
        }
        if let Some(w) = &self.worker {
            let _ = w.send.send(Job::Suspend);
        }
        self.epoch += 1;
        self.phase = Phase::Idle;
        self.previous = None;
        self.current = None;
        self.view = None;
        self.stream = None;
        self.outstanding = None;
    }

    /// Keeps collision around the skater as they travel.
    fn stream_ground(&mut self) {
        let Some(view) = &self.view else { return };
        // Measure from the ground last ridden, not from mid-air: from the
        // top of an ollie, doorframes and awnings would count as ground.
        let mut at = view.root;
        if let Some((ground, _)) = self.last_ground {
            at.z = at.z.min(ground.z + 5.0);
        }
        // Dropping with no ground scanned underneath yet: hurry.
        self.urgent = view.root.z < self.installed_at.map_or(view.root.z, |c| c.z) - 150.0
            && !self.heights.as_ref().and_then(|h| h.around(view.root.truncate())).is_some_and(|(_, n)| n > 0);
        if self.stream.is_none() {
            let far = self.installed_at.is_none_or(|c| {
                (at - c).truncate().length() > self.config.scan_radius * 0.3 * crate::scan::METRE || (at.z - c.z).abs() > 150.0
            });
            if far {
                self.stream =
                    Some(Scan::with_cache(at, self.config.scan_radius, self.config.scan_spacing, self.config.thin_radius, &self.cache));
            }
        }
        if self.stream.as_ref().is_some_and(Scan::done) {
            let Some(scan) = self.stream.take() else { return };
            let ground = scan.ground();
            scan.store(&mut self.cache);
            crate::log!("rescan: {} rays, {} columns reused", scan.rays, scan.reused);
            if ground.skate.is_empty() {
                return;
            }
            self.installed_at = Some(ground.center);
            self.heights = Some(ground.heights.clone());
            self.challenges.set_world(ground.heights.clone(), ground.rails_core.clone());
            self.installs += 1;
            let collision = Collision { skate: ground.skate, inches: ground.inches, grid_rails: ground.rails, rails: self.config.rails };
            if let Some(w) = &self.worker {
                let _ = w.send.send(Job::Collision(collision));
            }
        }
    }

    fn fill_outputs(&mut self, block: &Block, req: &Request, reply: &mut Reply) {
        let Some(view) = &self.view else { return };
        // Mid-flip, the skater and board turn about the hips.
        let flipped = self.flip_turn(view).map(|(pivot, rot)| View {
            root: view.root,
            joints: view.joints.rotated(pivot, rot),
            board: view.board.iter().map(|(n, p)| (*n, pivot + rot * (*p - pivot))).collect(),
            camera: view.camera,
            forward: view.forward,
        });
        let view = flipped.as_ref().unwrap_or(view);
        let s = self.space;
        reply.flags |= ipc::SKATING;

        // Batman's capsule follows the board; he faces where it's going.
        let feet = s.to_ue(view.root);
        let pawn_loc = feet + Vec3::Z * req.half_height;
        let yaw = view
            .forward
            .and_then(|f| s.dir_to_ue(f).truncate().try_normalize())
            .map_or(req.actor_rot[1], |f| (f.y.atan2(f.x) * RUU_PER_RADIAN).round() as i32);
        let pawn_rot = [0, yaw, 0];
        reply.pawn_loc = pawn_loc.to_array();
        reply.pawn_rot = pawn_rot;
        reply.flags |= ipc::SET_PAWN;

        if self.config.pose {
            if let Some(rig) = &self.rig {
                // Where the body mesh will be once the pawn has moved: its
                // offset from the pawn stays the same.
                let before = actor_xf(req.actor_loc, req.actor_rot);
                let after = actor_xf(pawn_loc.to_array(), pawn_rot);
                let component = after.mul(&before.inverse()).mul(&Xf::from_ue_matrix(&req.comp_to_world));
                let core_component = s.xf_to_core(&component);
                if let Some(pose) = rig.pose(&core_component, &view.joints, self.config.plant_feet) {
                    if self.snapshots < 12 && self.ride_frames % 90 == 45 {
                        self.snapshots += 1;
                        let state = self.current.as_ref().map_or(String::new(), |t| t.state.clone());
                        snapshot(self.player, self.snapshots, &state, rig, &core_component, &pose, &view.joints, &view.board);
                    }
                    let atoms: Vec<ipc::Atom> = pose.iter().map(|x| s.xf_to_ue(x).to_atom()).collect();
                    block.write_bones(&atoms);
                    reply.bone_count = atoms.len() as u32;
                    reply.flags |= ipc::SET_BONES;
                    if !self.posed_logged {
                        self.posed_logged = true;
                        crate::log!("posing {} bones; pawn at {pawn_loc:?}, component at {:?}", atoms.len(), component.pos);
                    }
                }
            }
        }

        if self.config.skate_camera {
            if let Some((pos, forward, up)) = view.camera {
                if let Some(rot) = look_rotator(s.dir_to_ue(forward), s.dir_to_ue(up)) {
                    reply.cam_loc = s.to_ue(pos).to_array();
                    reply.cam_rot = rot;
                    reply.flags |= ipc::SET_CAMERA;
                }
            }
        }

        if self.config.board {
            if let Some(parts) = board_parts(&view.board, &s) {
                reply.board_count = parts.len() as u32;
                for (i, part) in parts.into_iter().enumerate() {
                    match i {
                        0..8 => reply.board[i] = part,
                        8..10 => reply.board_more[i - 8] = part,
                        _ => {}
                    }
                }
                reply.flags |= ipc::SET_BOARD;
            }
        }
    }
}

/// The board drawn from simple meshes, placed from the simulation's truck
/// and wheel positions. In order (the game script picks materials by it):
/// deck, nose, tail, front truck, back truck, then the four wheels.

fn board_parts(board: &[(&'static str, Vec3)], s: &Space) -> Option<Vec<BoardPart>> {
    let at = |n: &str| board.iter().find(|(name, _)| *name == n).map(|(_, p)| *p);
    let (front, back) = (at("TRUCK_FRONT")?, at("TRUCK_BACK")?);
    let (lf, rf, lb, rb) = (at("LEFT_WHEELFRONT")?, at("RIGHT_WHEELFRONT")?, at("LEFT_WHEELBACK")?, at("RIGHT_WHEELBACK")?);
    let forward = (front - back).try_normalize()?;
    let left = ((lf + lb) - (rf + rb)).try_normalize()?;
    let up = forward.cross(left).try_normalize()?;
    let left = up.cross(forward);
    let metre = s.ue_units_per_metre();
    let m = crate::scan::METRE;
    let part = |p: Vec3, x: Vec3, z: Vec3, size: [f32; 3]| -> Option<BoardPart> {
        Some(BoardPart {
            loc: s.to_ue(p).to_array(),
            rot: look_rotator(s.dir_to_ue(x), s.dir_to_ue(z))?,
            size: [size[0] * metre, size[1] * metre, size[2] * metre],
        })
    };
    // A 0.82 m deck: a flat middle between the trucks' outer edges, and the
    // nose and tail kicked up 20 degrees.
    let middle = (front + back) * 0.5;
    let deck = middle + up * (0.075 * m);
    let flat = 0.56;
    let kick = 0.14;
    let (sin, cos) = 20f32.to_radians().sin_cos();
    let nose_dir = forward * cos + up * sin;
    let tail_dir = -forward * cos + up * sin;
    let nose_hinge = deck + forward * (flat / 2.0 * m);
    let tail_hinge = deck - forward * (flat / 2.0 * m);
    let mut parts = vec![
        part(deck, forward, up, [flat, 0.205, 0.014])?,
        part(nose_hinge + nose_dir * (kick / 2.0 * m), nose_dir, nose_dir.cross(left).normalize(), [kick, 0.195, 0.014])?,
        part(tail_hinge + tail_dir * (kick / 2.0 * m), tail_dir, left.cross(tail_dir).normalize(), [kick, 0.195, 0.014])?,
        part(front + up * (0.035 * m), forward, up, [0.05, 0.17, 0.045])?,
        part(back + up * (0.035 * m), forward, up, [0.05, 0.17, 0.045])?,
    ];
    for w in [lf, rf, lb, rb] {
        parts.push(part(w, forward, up, [0.054, 0.034, 0.054])?);
    }
    Some(parts)
}

/// Writes the skater's joints and Batman's posed bones (core space) to
/// SkateCity_pose_N.json next to the helper, for checking the retarget.
#[allow(clippy::too_many_arguments)]
fn snapshot(
    player: usize,
    n: u32,
    state: &str,
    rig: &Rig,
    component: &Xf,
    pose: &[Xf],
    joints: &Joints,
    board: &[(&'static str, Vec3)],
) {
    let v = |p: Vec3| format!("[{:.2},{:.2},{:.2}]", p.x, p.y, p.z);
    let q = |r: bevy::math::Quat| format!("[{:.4},{:.4},{:.4},{:.4}]", r.x, r.y, r.z, r.w);
    let mut out = String::from("{");
    out += &format!("\"state\":\"{state}\",");
    out += "\"skater\":{";
    out += &joints.names().map(|(name, p)| format!("\"{name}\":{}", v(p))).collect::<Vec<_>>().join(",");
    out += "},\"board\":{";
    out += &board.iter().map(|(name, p)| format!("\"{name}\":{}", v(*p))).collect::<Vec<_>>().join(",");
    out += "},\"bones\":[";
    let base = rig.base_world(component);
    out += &rig
        .names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let w = component.mul(&pose[i]);
            format!(
                "{{\"name\":\"{name}\",\"parent\":{},\"pos\":{},\"rot\":{},\"base\":{},\"base_rot\":{}}}",
                rig.parent(i).map_or(-1, |p| p as i64),
                v(w.pos),
                q(w.rot),
                v(base[i].pos),
                q(base[i].rot)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    out += "]}";
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(std::path::PathBuf::from)).unwrap_or_default();
    let path = dir.join(format!("SkateCity{}_pose_{n}.json", crate::suffix(player)));
    match std::fs::write(&path, out) {
        Ok(()) => crate::log!("pose snapshot {n} ({state}) written"),
        Err(e) => crate::log!("could not write {}: {e}", path.display()),
    }
}
