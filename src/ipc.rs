//! The block of shared memory the game script and the helper talk through.
//! Every field is 4 bytes wide so the 32-bit game and the 64-bit helper lay
//! it out identically; SkateCity.cs mirrors these offsets exactly.
//!
//! Each frame the game writes a request (its state, the results of the rays
//! it was asked to cast, Batman's bones), bumps `seq` and signals; the
//! helper answers with a reply carrying the same `seq`.

pub const SIZE: usize = 1 << 20;
pub const MAGIC: u32 = 0x534B_4331;
pub const MAX_RAYS: usize = 4096;
pub const MAX_BONES: usize = 512;
pub const NAME_LEN: usize = 64;

pub const REQ: usize = 0;
pub const REQ_HITS: usize = 0x1000;
pub const REQ_NAMES: usize = 0x20000;
pub const REQ_PARENTS: usize = 0x28000;
pub const REQ_LOCALS: usize = 0x28800;
pub const REP: usize = 0x40000;
pub const REP_RAYS: usize = 0x41000;
pub const REP_BONES: usize = 0x60000;

/// Request flags.
pub const PAWN_VALID: u32 = 1;
pub const SKELETON_VALID: u32 = 2;

/// Reply flags.
pub const SET_PAWN: u32 = 1;
pub const SET_CAMERA: u32 = 2;
pub const SET_BONES: u32 = 4;
pub const SET_BOARD: u32 = 8;
pub const SKATING: u32 = 16;

/// Reply states.
pub const LOADING: u32 = 0;
pub const READY: u32 = 1;
pub const STARTING: u32 = 2;
pub const RIDING: u32 = 3;
pub const FAILED: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Request {
    pub magic: u32,
    pub seq: u32,
    pub pid: u32,
    pub flags: u32,
    pub dt: f32,
    /// The pawn's location (the middle of its collision cylinder) and rotation.
    pub actor_loc: [f32; 3],
    pub actor_rot: [i32; 3],
    pub half_height: f32,
    /// The body mesh's LocalToWorld, as UE stores it (row-vector rows).
    pub comp_to_world: [f32; 16],
    /// Which batch of rays `REQ_HITS` answers, and how many.
    pub ray_batch: u32,
    pub ray_count: u32,
    /// Bumped whenever the bone names and parents are rewritten.
    pub skel_seq: u32,
    pub bone_count: u32,
    /// How many times the game has seen the toggle key pressed.
    pub toggles: u32,
    /// Bumped each time the game loads into a world.
    pub world: u32,
    /// Which step of its frame the script is on, and how many frames it has
    /// run: if the game dies, the last values say where.
    pub step: u32,
    pub frames: u32,
    pub reserved: [u32; 28],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Hit {
    pub hit: u32,
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Ray {
    pub from: [f32; 3],
    pub to: [f32; 3],
}

/// A sound for the game to play: what (one of the `SOUND_` kinds) and where.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SoundEvent {
    pub kind: u32,
    pub loc: [f32; 3],
}

pub const SOUND_POP: u32 = 1;
pub const SOUND_LAND: u32 = 2;
pub const SOUND_GRIND: u32 = 3;
pub const SOUND_HIT: u32 = 4;
pub const SOUND_HARD_HIT: u32 = 5;
pub const SOUND_BREAK: u32 = 6;
pub const SOUND_SLOTS: usize = 8;

/// One piece of the board: where it goes, and its size in game units
/// (length, width, height).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BoardPart {
    pub loc: [f32; 3],
    pub rot: [i32; 3],
    pub size: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Reply {
    pub seq: u32,
    pub state: u32,
    pub flags: u32,
    pub pawn_loc: [f32; 3],
    pub pawn_rot: [i32; 3],
    pub cam_loc: [f32; 3],
    pub cam_rot: [i32; 3],
    pub cam_fov: f32,
    pub ray_batch: u32,
    pub ray_count: u32,
    pub bone_count: u32,
    pub msg_seq: u32,
    pub board_count: u32,
    pub board: [BoardPart; 8],
    pub msg: [u8; 256],
    pub score: [u8; 128],
    pub units_per_metre: f32,
    /// The keyboard key (a Windows virtual-key code) that toggles skating.
    pub toggle_key: u32,
    /// Batman's velocity (game units per second) for a glide, and counters
    /// bumped to start a glide and to drop out of one.
    pub glide_vel: [f32; 3],
    pub glide_seq: u32,
    pub glide_end_seq: u32,
    /// The trick display: points of the line being skated and the session
    /// total; and the last finished line (bumped `result_seq`, how it went,
    /// its points and trick names).
    pub hud_points: u32,
    pub hud_total: u32,
    pub result_seq: u32,
    pub result_kind: u32,
    pub result_points: u32,
    pub result_text: [u8; 128],
    /// Board pieces beyond the first eight.
    pub board_more: [BoardPart; 2],
    /// Sounds: how many there have been in all, and the latest few, each in
    /// slot `number % SOUND_SLOTS`.
    pub sound_count: u32,
    pub sound_pad: u32,
    pub sounds: [SoundEvent; SOUND_SLOTS],
    pub reserved: [u32; 4],
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            seq: 0,
            state: LOADING,
            flags: 0,
            pawn_loc: [0.0; 3],
            pawn_rot: [0; 3],
            cam_loc: [0.0; 3],
            cam_rot: [0; 3],
            cam_fov: 0.0,
            ray_batch: 0,
            ray_count: 0,
            bone_count: 0,
            msg_seq: 0,
            board_count: 0,
            board: [BoardPart::default(); 8],
            msg: [0; 256],
            score: [0; 128],
            units_per_metre: 0.0,
            toggle_key: 0,
            glide_vel: [0.0; 3],
            glide_seq: 0,
            glide_end_seq: 0,
            hud_points: 0,
            hud_total: 0,
            result_seq: 0,
            result_kind: 0,
            result_points: 0,
            result_text: [0; 128],
            board_more: [BoardPart::default(); 2],
            sound_count: 0,
            sound_pad: 0,
            sounds: [SoundEvent::default(); SOUND_SLOTS],
            reserved: [0; 4],
        }
    }
}

pub fn write_text(out: &mut [u8], text: &str) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(out.len() - 1);
    out[..n].copy_from_slice(&bytes[..n]);
    out[n..].fill(0);
}

/// A bone transform as UE's FBoneAtom stores it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Atom {
    pub quat: [f32; 4],
    pub pos: [f32; 3],
    pub scale: f32,
}

pub struct Block {
    base: *mut u8,
}

unsafe impl Send for Block {}

impl Block {
    /// # Safety
    /// `base` must point at `SIZE` bytes that stay mapped.
    pub unsafe fn new(base: *mut u8) -> Self {
        Self { base }
    }

    fn at<T>(&self, offset: usize) -> *mut T {
        unsafe { self.base.add(offset).cast() }
    }

    /// The script's breadcrumb: (step, frames).
    pub fn breadcrumb(&self) -> (u32, u32) {
        unsafe { (std::ptr::read_volatile(self.at::<u32>(REQ + 136)), std::ptr::read_volatile(self.at::<u32>(REQ + 140))) }
    }

    pub fn request(&self) -> Request {
        unsafe { std::ptr::read_volatile(self.at::<Request>(REQ)) }
    }

    pub fn hits(&self, count: usize) -> Vec<Hit> {
        let count = count.min(MAX_RAYS);
        (0..count).map(|i| unsafe { std::ptr::read_volatile(self.at::<Hit>(REQ_HITS).add(i)) }).collect()
    }

    pub fn bone_names(&self, count: usize) -> Vec<String> {
        (0..count.min(MAX_BONES))
            .map(|i| {
                let mut raw = [0u8; NAME_LEN];
                unsafe { std::ptr::copy_nonoverlapping(self.at::<u8>(REQ_NAMES + i * NAME_LEN), raw.as_mut_ptr(), NAME_LEN) };
                let end = raw.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
                String::from_utf8_lossy(&raw[..end]).into_owned()
            })
            .collect()
    }

    pub fn parents(&self, count: usize) -> Vec<i32> {
        (0..count.min(MAX_BONES)).map(|i| unsafe { std::ptr::read_volatile(self.at::<i32>(REQ_PARENTS).add(i)) }).collect()
    }

    pub fn locals(&self, count: usize) -> Vec<Atom> {
        (0..count.min(MAX_BONES)).map(|i| unsafe { std::ptr::read_volatile(self.at::<Atom>(REQ_LOCALS).add(i)) }).collect()
    }

    pub fn write_rays(&self, rays: &[Ray]) {
        for (i, r) in rays.iter().take(MAX_RAYS).enumerate() {
            unsafe { std::ptr::write_volatile(self.at::<Ray>(REP_RAYS).add(i), *r) };
        }
    }

    pub fn write_bones(&self, bones: &[Atom]) {
        for (i, b) in bones.iter().take(MAX_BONES).enumerate() {
            unsafe { std::ptr::write_volatile(self.at::<Atom>(REP_BONES).add(i), *b) };
        }
    }

    /// Writes the reply, `seq` last so the game never sees a half-written one.
    pub fn write_reply(&self, reply: &Reply) {
        let mut body = *reply;
        body.seq = 0;
        unsafe {
            let p = self.at::<Reply>(REP);
            let current = std::ptr::read_volatile(p).seq;
            body.seq = current;
            std::ptr::write_volatile(p, body);
            std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
            std::ptr::write_volatile(self.at::<u32>(REP), reply.seq);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_matches_the_script() {
        assert_eq!(std::mem::size_of::<Request>(), 256);
        assert_eq!(std::mem::size_of::<Hit>(), 28);
        assert_eq!(std::mem::size_of::<Ray>(), 24);
        assert_eq!(std::mem::size_of::<BoardPart>(), 36);
        assert_eq!(std::mem::size_of::<Atom>(), 32);
        assert_eq!(std::mem::offset_of!(Request, comp_to_world), 48);
        assert_eq!(std::mem::offset_of!(Request, ray_batch), 112);
        assert_eq!(std::mem::offset_of!(Request, toggles), 128);
        assert_eq!(std::mem::offset_of!(Request, world), 132);
        assert_eq!(std::mem::offset_of!(Request, step), 136);
        assert_eq!(std::mem::offset_of!(Reply, toggle_key), 760);
        assert_eq!(std::mem::offset_of!(Reply, glide_vel), 764);
        assert_eq!(std::mem::offset_of!(Reply, glide_seq), 776);
        assert_eq!(std::mem::offset_of!(Reply, glide_end_seq), 780);
        assert_eq!(std::mem::offset_of!(Reply, hud_points), 784);
        assert_eq!(std::mem::offset_of!(Reply, hud_total), 788);
        assert_eq!(std::mem::offset_of!(Reply, result_seq), 792);
        assert_eq!(std::mem::offset_of!(Reply, result_kind), 796);
        assert_eq!(std::mem::offset_of!(Reply, result_points), 800);
        assert_eq!(std::mem::offset_of!(Reply, result_text), 804);
        assert_eq!(std::mem::offset_of!(Reply, board_more), 932);
        assert_eq!(std::mem::offset_of!(Reply, sound_count), 1004);
        assert_eq!(std::mem::offset_of!(Reply, sounds), 1012);
        assert_eq!(std::mem::size_of::<SoundEvent>(), 16);
        assert!(std::mem::size_of::<Reply>() <= REP_RAYS - REP);
        assert_eq!(std::mem::offset_of!(Request, frames), 140);
        assert!(REQ_NAMES + MAX_BONES * NAME_LEN <= REQ_PARENTS);
        assert!(REQ_PARENTS + MAX_BONES * 4 <= REQ_LOCALS);
        assert_eq!(std::mem::offset_of!(Reply, board), 84);
        assert_eq!(std::mem::offset_of!(Reply, msg), 372);
        assert_eq!(std::mem::offset_of!(Reply, score), 628);
        assert_eq!(std::mem::offset_of!(Reply, units_per_metre), 756);
        assert_eq!(std::mem::offset_of!(Reply, toggle_key), 760);
        assert_eq!(std::mem::size_of::<Reply>(), 1156);
        assert!(REQ_HITS + MAX_RAYS * 28 <= REQ_NAMES);
        assert!(REQ_LOCALS + MAX_BONES * 32 <= REP);
        assert!(REP + std::mem::size_of::<Reply>() <= REP_RAYS);
        assert!(REP_RAYS + MAX_RAYS * 24 <= REP_BONES);
        assert!(REP_BONES + MAX_BONES * 32 <= SIZE);
    }
}
