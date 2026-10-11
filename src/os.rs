//! The Windows calls the helper needs, declared by hand: the controller,
//! the keyboard, and the shared memory and events it talks to the game over.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct Gamepad {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct XInputState {
    pub packet: u32,
    pub pad: Gamepad,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
mod imp {
    use super::{XInputState, wide};
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
        fn MapViewOfFile(mapping: *mut c_void, access: u32, high: u32, low: u32, bytes: usize) -> *mut c_void;
        fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
        fn SetEvent(event: *mut c_void) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> *mut c_void;
        fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
    }
    #[link(name = "xinput")]
    unsafe extern "system" {
        fn XInputGetState(index: u32, state: *mut XInputState) -> u32;
    }

    const FILE_MAP_ALL_ACCESS: u32 = 0xF001F;
    const EVENT_ACCESS: u32 = 0x0002 | 0x0010_0000;
    const SYNCHRONIZE: u32 = 0x0010_0000;

    pub struct Shared(pub *mut u8);
    unsafe impl Send for Shared {}

    pub fn open_shared(name: &str) -> Result<Shared, String> {
        let name = wide(name);
        unsafe {
            let mapping = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, name.as_ptr());
            if mapping.is_null() {
                return Err("the game's shared memory was not found".into());
            }
            let view = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, 0);
            if view.is_null() {
                return Err("the game's shared memory could not be mapped".into());
            }
            Ok(Shared(view.cast()))
        }
    }

    pub struct Event(*mut c_void);
    unsafe impl Send for Event {}

    impl Event {
        pub fn open(name: &str) -> Result<Self, String> {
            let w = wide(name);
            let h = unsafe { OpenEventW(EVENT_ACCESS, 0, w.as_ptr()) };
            if h.is_null() { Err(format!("event {name} was not found")) } else { Ok(Self(h)) }
        }
        pub fn set(&self) {
            unsafe { SetEvent(self.0) };
        }
        /// True if signalled within `ms`.
        pub fn wait(&self, ms: u32) -> bool {
            unsafe { WaitForSingleObject(self.0, ms) == 0 }
        }
    }

    pub struct Process(*mut c_void);

    impl Process {
        pub fn open(pid: u32) -> Option<Self> {
            let h = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
            (!h.is_null()).then_some(Self(h))
        }
        pub fn exited(&self) -> bool {
            unsafe { WaitForSingleObject(self.0, 0) == 0 }
        }
    }

    pub fn game_focused(game_pid: u32) -> bool {
        unsafe {
            let window = GetForegroundWindow();
            let mut pid = 0u32;
            !window.is_null() && {
                GetWindowThreadProcessId(window, &mut pid);
                pid == game_pid
            }
        }
    }

    pub fn pad(index: u32) -> Option<XInputState> {
        let mut state = XInputState::default();
        (unsafe { XInputGetState(index, &mut state) } == 0).then_some(state)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::XInputState;
    pub struct Shared(pub *mut u8);
    unsafe impl Send for Shared {}
    pub fn open_shared(_name: &str) -> Result<Shared, String> {
        let _ = super::wide("");
        Ok(Shared(Box::leak(vec![0u8; crate::ipc::SIZE].into_boxed_slice()).as_mut_ptr()))
    }
    pub struct Event;
    impl Event {
        pub fn open(_name: &str) -> Result<Self, String> {
            Ok(Self)
        }
        pub fn set(&self) {}
        pub fn wait(&self, ms: u32) -> bool {
            std::thread::sleep(std::time::Duration::from_millis(ms.min(16) as u64));
            false
        }
    }
    pub struct Process;
    impl Process {
        pub fn open(_pid: u32) -> Option<Self> {
            Some(Self)
        }
        pub fn exited(&self) -> bool {
            false
        }
    }
    pub fn game_focused(_pid: u32) -> bool {
        true
    }
    pub fn pad(_index: u32) -> Option<XInputState> {
        None
    }
}

pub use imp::*;

/// The first connected controller, if any.
pub fn first_pad() -> Option<XInputState> {
    (0..4).find_map(pad)
}
