// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! The libretro C ABI for AnticRust.
//!
//! On most cores this layer is plumbing. On this one it is half the product:
//! the project exists because RetroAchievements cannot support the Atari 5200
//! on any existing core, and both failures live here, not in the emulation.
//!
//! **Memory, for achievements.** `a5200` exposes no memory, so RA has nothing
//! to watch. This core exposes the 16K of RAM two ways:
//!
//! - `retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM)`, 0x4000 bytes. rcheevos
//!   has no region table for console 50 (Atari 5200) yet, and with none it
//!   ignores memory maps and lays SYSTEM_RAM out from address 0
//!   (`rc_libretro_memory_init_without_regions`). That puts RA address N at
//!   6502 address $N, which is what a set designer wants.
//! - `SET_MEMORY_MAPS`, one descriptor for $0000-$3FFF, for when rcheevos gains
//!   a table (proposed in docs/notes/RA-PROPOSAL.md). The two agree, so a set
//!   written today keeps its addresses then.
//!
//! The cartridge is not published. Bounty Bob banks two of its windows, and a
//! descriptor over a switchable bank is correct only until the next switch.
//!
//! **Content only from the frontend.** `atari800` can open files from its own
//! menu, so the bytes RA hashed and the bytes running can differ. This core
//! has no content file IO at all; see `antic_core::ContentSource`.
//!
//! **No BIOS, ever.** Always the HLE image. The core never asks for a
//! firmware file and has nothing to configure.
//!
//! Plumbing lifted from ColecoRust's `coleco-libretro` (origin/main), itself
//! lifted from MegaRust: declare XRGB8888 explicitly (value 1, not 2), stamp
//! the build. Struct layouts and constants are from the libretro API header,
//! checked against the header our host compiles
//! (TrophyHubLibretroHost/src/libretro/retro_types.h).
//!
//! # Controls
//!
//! A 5200 controller is an analog stick that does not self-centre, two
//! fire buttons on each side (the two bottoms are one input, the two tops
//! another), a twelve-key pad, and Start, Pause and Reset. The stick is read
//! as a pot count, 1 to 227, with 114 at rest.
//!
//! The mapping is chosen so Trophy Hub can reuse its Intellivision pad (an
//! analog stick, a three-button T, a twelve-key panel in the same grid): every
//! input below is what that pad already sends, except the Select pill and a
//! single L pill, for Pause and Reset.
//!
//! | 5200 | RetroPad |
//! |---|---|
//! | stick | left analog, proportional; D-pad, full deflection |
//! | bottom fire | B or A |
//! | top fire | Y or X |
//! | Start, Pause, Reset | START, SELECT, L |
//! | keypad 1-4, 6-9 | right analog, the eight directions of the grid |
//! | keypad 5, 0 | R3, L3 |
//! | keypad `*`, `#` | L2, R2 (Intellivision's Clear and Enter) |
//!
//! The right-stick keys follow the grid around 5: up-left is 1, up is 2,
//! up-right is 3, left 4, right 6, down-left 7, down 8, down-right 9. A
//! direction counts once the stick is past half way on that axis. Keyboard on
//! port 1: digits, `*`, `#`.

#![allow(clippy::missing_safety_doc)]

use antic_core::cart::Cart;
use antic_core::gtia::{palette, HEIGHT, WIDTH};
use antic_core::machine::Machine;
use antic_core::pokey::{Key, POT_CENTRE, SAMPLE_RATE};
use std::ffi::{c_char, c_uint, c_void};

pub const RETRO_API_VERSION: u32 = 1;

const RETRO_ENVIRONMENT_EXPERIMENTAL: c_uint = 0x10000;
const RETRO_ENVIRONMENT_SET_PIXEL_FORMAT: c_uint = 10;
const RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS: c_uint = 11;
/// `36 | EXPERIMENTAL`. Sent bare, 36 is ignored, and 35 is
/// SET_CONTROLLER_INFO, which crashed a host given the wrong payload.
const RETRO_ENVIRONMENT_SET_MEMORY_MAPS: c_uint = 36 | RETRO_ENVIRONMENT_EXPERIMENTAL;
/// `42 | EXPERIMENTAL`: tells RetroArch the core supports achievements.
const RETRO_ENVIRONMENT_SET_SUPPORT_ACHIEVEMENTS: c_uint = 42 | RETRO_ENVIRONMENT_EXPERIMENTAL;
/// `retro_pixel_format`: 0 = 0RGB1555, 1 = XRGB8888, 2 = RGB565.
const RETRO_PIXEL_FORMAT_XRGB8888: c_uint = 1;

const RETRO_REGION_NTSC: c_uint = 0;
const RETRO_MEMORY_SYSTEM_RAM: c_uint = 2;
/// `RETRO_MEMDESC_SYSTEM_RAM`, bit 2 of the descriptor flags.
const RETRO_MEMDESC_SYSTEM_RAM: u64 = 1 << 2;

const RETRO_DEVICE_JOYPAD: c_uint = 1;
const RETRO_DEVICE_KEYBOARD: c_uint = 3;
const RETRO_DEVICE_ANALOG: c_uint = 5;
const ANALOG_LEFT: c_uint = 0;
const ANALOG_RIGHT: c_uint = 1;
const AXIS_X: c_uint = 0;
const AXIS_Y: c_uint = 1;
/// Half deflection counts as pressed for a stick standing in for a key.
const ANALOG_PRESSED: i16 = 0x4000;

const JOY_B: c_uint = 0;
const JOY_Y: c_uint = 1;
const JOY_SELECT: c_uint = 2;
const JOY_START: c_uint = 3;
const JOY_UP: c_uint = 4;
const JOY_DOWN: c_uint = 5;
const JOY_LEFT: c_uint = 6;
const JOY_RIGHT: c_uint = 7;
const JOY_A: c_uint = 8;
const JOY_X: c_uint = 9;
const JOY_L: c_uint = 10;
const JOY_L2: c_uint = 12;
const JOY_R2: c_uint = 13;
const JOY_L3: c_uint = 14;
const JOY_R3: c_uint = 15;

/// Keys on RetroPad buttons, first held wins.
const KEY_BUTTONS: [(c_uint, Key); 7] = [
    (JOY_START, Key::Start),
    (JOY_SELECT, Key::Pause),
    (JOY_L, Key::Reset),
    (JOY_R3, Key::Digit(5)),
    (JOY_L3, Key::Digit(0)),
    (JOY_L2, Key::Star),
    (JOY_R2, Key::Hash),
];

/// Keypad keys on the right stick: (x sign, y sign) -> key, the grid around 5.
const KEY_DIRECTIONS: [((i8, i8), Key); 8] = [
    ((-1, -1), Key::Digit(1)),
    ((0, -1), Key::Digit(2)),
    ((1, -1), Key::Digit(3)),
    ((-1, 0), Key::Digit(4)),
    ((1, 0), Key::Digit(6)),
    ((-1, 1), Key::Digit(7)),
    ((0, 1), Key::Digit(8)),
    ((1, 1), Key::Digit(9)),
];

/// libretro keyboard codes: '0'-'9' are ASCII, `*` is 42 and keypad `*`
/// 268, `#` is 35.
const KEYBOARD: [(c_uint, Key); 13] = [
    (48, Key::Digit(0)),
    (49, Key::Digit(1)),
    (50, Key::Digit(2)),
    (51, Key::Digit(3)),
    (52, Key::Digit(4)),
    (53, Key::Digit(5)),
    (54, Key::Digit(6)),
    (55, Key::Digit(7)),
    (56, Key::Digit(8)),
    (57, Key::Digit(9)),
    (42, Key::Star),
    (268, Key::Star),
    (35, Key::Hash),
];

/// `strings -a libanticrust_libretro.so | grep build=` names the commit.
#[used]
static BUILD_STAMP: &[u8] = concat!("ANTICRUST build=", env!("ANTICRUST_BUILD_ID"), "\0").as_bytes();

#[repr(C)]
pub struct RetroSystemInfo {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub need_fullpath: bool,
    pub block_extract: bool,
}

#[repr(C)]
pub struct RetroGameGeometry {
    pub base_width: c_uint,
    pub base_height: c_uint,
    pub max_width: c_uint,
    pub max_height: c_uint,
    pub aspect_ratio: f32,
}

#[repr(C)]
pub struct RetroSystemTiming {
    pub fps: f64,
    pub sample_rate: f64,
}

#[repr(C)]
pub struct RetroSystemAvInfo {
    pub geometry: RetroGameGeometry,
    pub timing: RetroSystemTiming,
}

#[repr(C)]
pub struct RetroGameInfo {
    pub path: *const c_char,
    pub data: *const c_void,
    pub size: usize,
    pub meta: *const c_char,
}

#[repr(C)]
pub struct RetroInputDescriptor {
    pub port: c_uint,
    pub device: c_uint,
    pub index: c_uint,
    pub id: c_uint,
    pub description: *const c_char,
}

#[repr(C)]
pub struct RetroMemoryDescriptor {
    pub flags: u64,
    pub ptr: *mut c_void,
    pub offset: usize,
    pub start: usize,
    pub select: usize,
    pub disconnect: usize,
    pub len: usize,
    pub addrspace: *const c_char,
}

#[repr(C)]
pub struct RetroMemoryMap {
    pub descriptors: *const RetroMemoryDescriptor,
    pub num_descriptors: c_uint,
}

type EnvironmentFn = unsafe extern "C" fn(c_uint, *mut c_void) -> bool;
type VideoRefreshFn = unsafe extern "C" fn(*const c_void, c_uint, c_uint, usize);
type AudioSampleFn = unsafe extern "C" fn(i16, i16);
type AudioSampleBatchFn = unsafe extern "C" fn(*const i16, usize) -> usize;
type InputPollFn = unsafe extern "C" fn();
type InputStateFn = unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint) -> i16;

/// Everything between `retro_load_game` and `retro_unload_game`. A libretro
/// core is a singleton by contract.
struct Core {
    machine: Box<Machine>,
    /// The cartridge as the frontend handed it, for a power cycle.
    rom: Vec<u8>,
    pal: [[u8; 3]; 256],
    video: Vec<u32>,
    stereo: Vec<i16>,
}

static mut CORE: Option<Core> = None;
static mut ENVIRON: Option<EnvironmentFn> = None;
static mut VIDEO: Option<VideoRefreshFn> = None;
static mut AUDIO_BATCH: Option<AudioSampleBatchFn> = None;
static mut INPUT_POLL: Option<InputPollFn> = None;
static mut INPUT_STATE: Option<InputStateFn> = None;

#[allow(static_mut_refs)]
fn core() -> Option<&'static mut Core> {
    unsafe { CORE.as_mut() }
}

#[no_mangle]
pub extern "C" fn retro_api_version() -> c_uint {
    RETRO_API_VERSION
}

#[no_mangle]
pub unsafe extern "C" fn retro_set_environment(cb: EnvironmentFn) {
    ENVIRON = Some(cb);
}

#[no_mangle]
pub unsafe extern "C" fn retro_set_video_refresh(cb: VideoRefreshFn) {
    VIDEO = Some(cb);
}

#[no_mangle]
pub unsafe extern "C" fn retro_set_audio_sample(_cb: AudioSampleFn) {}

#[no_mangle]
pub unsafe extern "C" fn retro_set_audio_sample_batch(cb: AudioSampleBatchFn) {
    AUDIO_BATCH = Some(cb);
}

#[no_mangle]
pub unsafe extern "C" fn retro_set_input_poll(cb: InputPollFn) {
    INPUT_POLL = Some(cb);
}

#[no_mangle]
pub unsafe extern "C" fn retro_set_input_state(cb: InputStateFn) {
    INPUT_STATE = Some(cb);
}

#[no_mangle]
pub extern "C" fn retro_init() {}

#[no_mangle]
pub extern "C" fn retro_deinit() {
    unsafe { CORE = None };
}

#[no_mangle]
pub unsafe extern "C" fn retro_get_system_info(info: *mut RetroSystemInfo) {
    if info.is_null() {
        return;
    }
    (*info).library_name = c"AnticRust".as_ptr();
    (*info).library_version = c"0.1.0".as_ptr();
    (*info).valid_extensions = c"a52|bin|rom".as_ptr();
    (*info).need_fullpath = false;
    (*info).block_extract = false;
}

/// The part of the frame the frontend is shown: colour clocks $30-$D0 (the
/// normal playfield) and 224 lines, centred in the 384x240 the core draws.
/// The same window as a5200. Games park players and missiles just outside
/// what a TV shows; the full wide-playfield frame showed them as stray blocks
/// at the right edge of Pole Position.
const VIEW_W: usize = 320;
const VIEW_H: usize = 224;
const VIEW_X: usize = (WIDTH - VIEW_W) / 2;
const VIEW_Y: usize = (HEIGHT - VIEW_H) / 2;

/// Copy the visible window of a frame of colour bytes out as XRGB8888.
fn view(frame: &[u8], pal: &[[u8; 3]; 256], out: &mut [u32]) {
    for (y, row) in out.chunks_exact_mut(VIEW_W).enumerate() {
        let src = &frame[(VIEW_Y + y) * WIDTH + VIEW_X..][..VIEW_W];
        for (o, &ci) in row.iter_mut().zip(src) {
            let [r, g, b] = pal[ci as usize];
            *o = (r as u32) << 16 | (g as u32) << 8 | b as u32;
        }
    }
}

/// 1.79 MHz over 114 cycles a line and 262 lines: 59.92 Hz.
fn fps() -> f64 {
    antic_core::CPU_HZ as f64 / (114.0 * 262.0)
}

#[no_mangle]
pub unsafe extern "C" fn retro_get_system_av_info(info: *mut RetroSystemAvInfo) {
    if info.is_null() {
        return;
    }
    (*info).geometry = RetroGameGeometry {
        base_width: VIEW_W as c_uint,
        base_height: VIEW_H as c_uint,
        max_width: VIEW_W as c_uint,
        max_height: VIEW_H as c_uint,
        // A hi-res pixel is 0.857 as wide as tall on NTSC (Altirra 4.2).
        aspect_ratio: (VIEW_W as f32 * 0.857) / VIEW_H as f32,
    };
    (*info).timing = RetroSystemTiming { fps: fps(), sample_rate: SAMPLE_RATE as f64 };
}

#[no_mangle]
pub extern "C" fn retro_set_controller_port_device(_port: c_uint, _device: c_uint) {}

/// The machine for a cartridge, always on the HLE.
fn machine_for(rom: &[u8]) -> Option<Box<Machine>> {
    let cart = Cart::identified(rom.to_vec()).ok()?;
    Some(Box::new(Machine::new(cart, None)))
}

#[no_mangle]
pub unsafe extern "C" fn retro_reset() {
    let Some(c) = core() else { return };
    // A power cycle: a new machine, so the RAM moves, so the map is sent again.
    if let Some(m) = machine_for(&c.rom) {
        c.machine = m;
        if let Some(env) = ENVIRON {
            set_memory_maps(env);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn retro_load_game(game: *const RetroGameInfo) -> bool {
    if game.is_null() || (*game).data.is_null() || (*game).size == 0 {
        return false;
    }
    let rom = std::slice::from_raw_parts((*game).data as *const u8, (*game).size).to_vec();
    let Some(machine) = machine_for(&rom) else { return false };

    if let Some(env) = ENVIRON {
        let mut fmt = RETRO_PIXEL_FORMAT_XRGB8888;
        if !env(RETRO_ENVIRONMENT_SET_PIXEL_FORMAT, &mut fmt as *mut _ as *mut c_void) {
            return false;
        }
    }
    CORE = Some(Core { machine, rom, pal: palette(), video: vec![0; VIEW_W * VIEW_H], stereo: Vec::new() });
    if let Some(env) = ENVIRON {
        set_input_descriptors(env);
        set_memory_maps(env);
        let mut yes = true;
        env(RETRO_ENVIRONMENT_SET_SUPPORT_ACHIEVEMENTS, &mut yes as *mut _ as *mut c_void);
    }
    true
}

unsafe fn set_input_descriptors(env: EnvironmentFn) {
    let mut d = Vec::new();
    for port in 0..4 {
        let buttons: [(c_uint, &'static std::ffi::CStr); 15] = [
            (JOY_UP, c"Stick up (full)"),
            (JOY_DOWN, c"Stick down (full)"),
            (JOY_LEFT, c"Stick left (full)"),
            (JOY_RIGHT, c"Stick right (full)"),
            (JOY_B, c"Bottom fire"),
            (JOY_A, c"Bottom fire"),
            (JOY_Y, c"Top fire"),
            (JOY_X, c"Top fire"),
            (JOY_START, c"Start"),
            (JOY_SELECT, c"Pause"),
            (JOY_L, c"Reset"),
            (JOY_R3, c"Keypad 5"),
            (JOY_L3, c"Keypad 0"),
            (JOY_L2, c"Keypad *"),
            (JOY_R2, c"Keypad #"),
        ];
        for (id, name) in buttons {
            d.push(RetroInputDescriptor { port, device: RETRO_DEVICE_JOYPAD, index: 0, id, description: name.as_ptr() });
        }
        for (axis, name) in [(AXIS_X, c"Stick X (analog)"), (AXIS_Y, c"Stick Y (analog)")] {
            d.push(RetroInputDescriptor { port, device: RETRO_DEVICE_ANALOG, index: ANALOG_LEFT, id: axis, description: name.as_ptr() });
        }
        for (axis, name) in [(AXIS_X, c"Keypad 4/6 (and 1/3/7/9)"), (AXIS_Y, c"Keypad 2/8 (and 1/3/7/9)")] {
            d.push(RetroInputDescriptor { port, device: RETRO_DEVICE_ANALOG, index: ANALOG_RIGHT, id: axis, description: name.as_ptr() });
        }
    }
    d.push(RetroInputDescriptor { port: 0, device: 0, index: 0, id: 0, description: std::ptr::null() });
    env(RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS, d.as_mut_ptr() as *mut c_void);
}

/// The memory map RetroAchievements reads: the 16K of RAM at $0000-$3FFF,
/// where the CPU sees it.
unsafe fn set_memory_maps(env: EnvironmentFn) {
    let Some(c) = core() else { return };
    let desc = [ram_descriptor(&mut c.machine)];
    let mut map = RetroMemoryMap { descriptors: desc.as_ptr(), num_descriptors: desc.len() as c_uint };
    env(RETRO_ENVIRONMENT_SET_MEMORY_MAPS, &mut map as *mut _ as *mut c_void);
}

fn ram_descriptor(m: &mut Machine) -> RetroMemoryDescriptor {
    RetroMemoryDescriptor {
        flags: RETRO_MEMDESC_SYSTEM_RAM,
        ptr: m.bus.ram.as_mut_ptr() as *mut c_void,
        offset: 0,
        start: 0x0000,
        // select 0: matched by start and len, no mirrors to describe.
        select: 0,
        disconnect: 0,
        len: antic_core::RAM,
        addrspace: std::ptr::null(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn retro_load_game_special(_ty: c_uint, _info: *const RetroGameInfo, _num: usize) -> bool {
    false
}

#[no_mangle]
pub extern "C" fn retro_unload_game() {
    unsafe { CORE = None };
}

#[no_mangle]
pub extern "C" fn retro_get_region() -> c_uint {
    RETRO_REGION_NTSC
}

/// A stick axis (-32768..32767) as a pot count, 1..227 with 114 at rest.
fn pot(v: i16) -> u8 {
    let p = POT_CENTRE as i32 + (v as i32 * 113) / 32767;
    p.clamp(1, 227) as u8
}

/// Which way a stick axis points: -1, 0 or 1, past half deflection.
fn sign(v: i16) -> i8 {
    if v as i32 > ANALOG_PRESSED as i32 {
        1
    } else if (v as i32) < -(ANALOG_PRESSED as i32) {
        -1
    } else {
        0
    }
}

/// The keypad key a right-stick position stands for, if any.
fn key_for_direction(x: i16, y: i16) -> Option<Key> {
    let d = (sign(x), sign(y));
    KEY_DIRECTIONS.iter().find(|&&(dir, _)| dir == d).map(|&(_, k)| k)
}

/// One controller as the frontend reports it, written into POKEY and GTIA.
unsafe fn read_controller(m: &mut Machine, state: InputStateFn, port: c_uint) {
    let held = |id| state(port, RETRO_DEVICE_JOYPAD, 0, id) != 0;
    let axis = |index, id| state(port, RETRO_DEVICE_ANALOG, index, id);
    let mut x = pot(axis(ANALOG_LEFT, AXIS_X));
    let mut y = pot(axis(ANALOG_LEFT, AXIS_Y));
    if held(JOY_LEFT) {
        x = 1;
    } else if held(JOY_RIGHT) {
        x = 227;
    }
    if held(JOY_UP) {
        y = 1;
    } else if held(JOY_DOWN) {
        y = 227;
    }
    let mut key = KEY_BUTTONS.iter().find(|&&(id, _)| held(id)).map(|&(_, k)| k);
    if key.is_none() {
        key = key_for_direction(axis(ANALOG_RIGHT, AXIS_X), axis(ANALOG_RIGHT, AXIS_Y));
    }
    if port == 0 && key.is_none() {
        key = KEYBOARD.iter().find(|&&(code, _)| state(0, RETRO_DEVICE_KEYBOARD, 0, code) != 0).map(|&(_, k)| k);
    }
    let c = &mut m.bus.pokey.controllers[port as usize];
    c.pot = [x, y];
    c.key = key;
    c.top = held(JOY_Y) || held(JOY_X);
    m.bus.gtia.trigger[port as usize] = held(JOY_B) || held(JOY_A);
}

#[no_mangle]
pub unsafe extern "C" fn retro_run() {
    if let Some(poll) = INPUT_POLL {
        poll();
    }
    let Some(c) = core() else { return };
    if let Some(state) = INPUT_STATE {
        for port in 0..4 {
            read_controller(&mut c.machine, state, port);
        }
    }
    c.machine.run_frame();
    view(&c.machine.bus.gtia.frame, &c.pal, &mut c.video);
    if let Some(video) = VIDEO {
        video(c.video.as_ptr() as *const c_void, VIEW_W as c_uint, VIEW_H as c_uint, VIEW_W * 4);
    }
    // POKEY is mono; the frontend takes stereo frames.
    let mono = c.machine.bus.pokey.take_samples();
    c.stereo.clear();
    c.stereo.extend(mono.iter().flat_map(|&s| [s, s]));
    if let Some(audio) = AUDIO_BATCH {
        if !c.stereo.is_empty() {
            audio(c.stereo.as_ptr(), c.stereo.len() / 2);
        }
    }
}

// ---- save states ----
//
// The in-house contract (TrophyHubResources specs/play/
// IN_HOUSE_CORE_SAVESTATE_SPEC.md): the same bytes on every platform, a
// fixed size for every cartridge, and a refusal, never a panic, for a state
// that is truncated, oversized, from a newer format, or from another kind of
// cartridge. Netplay's state-identity token is ("anticrust",
// STATE_VERSION, STATE_SIZE).

#[no_mangle]
pub extern "C" fn retro_serialize_size() -> usize {
    match core() {
        Some(_) => antic_core::machine::STATE_SIZE,
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn retro_serialize(data: *mut c_void, size: usize) -> bool {
    let Some(c) = core() else { return false };
    let state = c.machine.save_state();
    if data.is_null() || size < state.len() {
        return false;
    }
    std::ptr::copy_nonoverlapping(state.as_ptr(), data as *mut u8, state.len());
    true
}

#[no_mangle]
pub unsafe extern "C" fn retro_unserialize(data: *const c_void, size: usize) -> bool {
    let Some(c) = core() else { return false };
    if data.is_null() {
        return false;
    }
    // A frontend may hand over a buffer of the size it was told. Exactly the
    // state, no more: anything longer is refused by load_state as oversized.
    let bytes = std::slice::from_raw_parts(data as *const u8, size);
    c.machine.load_state(bytes).is_ok()
}

// ---- memory interface ----

#[no_mangle]
pub extern "C" fn retro_get_memory_data(id: c_uint) -> *mut c_void {
    match (core(), id) {
        (Some(c), RETRO_MEMORY_SYSTEM_RAM) => c.machine.bus.ram.as_mut_ptr() as *mut c_void,
        _ => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn retro_get_memory_size(id: c_uint) -> usize {
    match (core(), id) {
        (Some(_), RETRO_MEMORY_SYSTEM_RAM) => antic_core::RAM,
        _ => 0,
    }
}

#[no_mangle]
pub extern "C" fn retro_cheat_reset() {}

#[no_mangle]
pub unsafe extern "C" fn retro_cheat_set(_index: c_uint, _enabled: bool, _code: *const c_char) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The core is a process-wide singleton; tests touching it take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// What the environment callback saw, copied out of the core's buffers.
    #[derive(Default)]
    struct Seen {
        maps: Vec<(u64, usize, usize, usize, usize, usize)>,
        map_ptrs: Vec<usize>,
        achievements: Option<bool>,
        pixel_format: Option<c_uint>,
        commands: Vec<c_uint>,
    }

    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

    unsafe extern "C" fn env(cmd: c_uint, data: *mut c_void) -> bool {
        let mut g = SEEN.lock().unwrap();
        let s = g.get_or_insert_with(Seen::default);
        s.commands.push(cmd);
        match cmd {
            RETRO_ENVIRONMENT_SET_PIXEL_FORMAT => s.pixel_format = Some(*(data as *const c_uint)),
            0x10024 => {
                let map = &*(data as *const RetroMemoryMap);
                for k in 0..map.num_descriptors as usize {
                    let d = &*map.descriptors.add(k);
                    s.maps.push((d.flags, d.offset, d.start, d.select, d.disconnect, d.len));
                    s.map_ptrs.push(d.ptr as usize);
                }
            }
            0x1002a => s.achievements = Some(*(data as *const bool)),
            _ => {}
        }
        true
    }

    /// An 8K cart entered directly that writes $5A to $1234, then loops.
    fn cart() -> Vec<u8> {
        let mut rom = vec![0xea; 8192];
        rom[..8].copy_from_slice(&[0xa9, 0x5a, 0x8d, 0x34, 0x12, 0x4c, 0x05, 0x80]);
        rom[0x1ffd] = 0xff;
        rom[0x1ffe] = 0x00;
        rom[0x1fff] = 0x80;
        rom
    }

    unsafe fn load(rom: &[u8]) {
        *SEEN.lock().unwrap() = Some(Seen::default());
        retro_set_environment(env);
        let info = RetroGameInfo { path: std::ptr::null(), data: rom.as_ptr() as *const c_void, size: rom.len(), meta: std::ptr::null() };
        assert!(retro_load_game(&info));
    }

    #[test]
    fn the_memory_map_is_sent_with_its_experimental_id_and_describes_ram_at_zero() {
        let _g = SERIAL.lock().unwrap();
        unsafe {
            load(&cart());
            let seen = SEEN.lock().unwrap();
            let s = seen.as_ref().unwrap();
            assert!(s.commands.contains(&0x10024), "SET_MEMORY_MAPS must be 36|0x10000, saw {:x?}", s.commands);
            assert!(!s.commands.contains(&36) && !s.commands.contains(&35), "a bare 35 or 36 is a different command");
            assert_eq!(s.maps, vec![(RETRO_MEMDESC_SYSTEM_RAM, 0, 0x0000, 0, 0, 0x4000)]);
            assert_eq!(s.map_ptrs[0], retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM) as usize);
            assert_eq!(s.achievements, Some(true));
            assert_eq!(s.pixel_format, Some(1), "XRGB8888 is 1, not 2");
            retro_unload_game();
        }
    }

    #[test]
    fn system_ram_is_the_cpus_ram_so_ra_address_n_is_6502_address_n() {
        let _g = SERIAL.lock().unwrap();
        unsafe {
            load(&cart());
            assert_eq!(retro_get_memory_size(RETRO_MEMORY_SYSTEM_RAM), 0x4000);
            retro_run();
            let ram = retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM) as *const u8;
            // The cart stored $5A at $1234; RA's offset $1234 must see it.
            assert_eq!(*ram.add(0x1234), 0x5a);
            // And it is the machine's RAM, not a copy: a write shows through.
            core().unwrap().machine.bus.ram[0x0042] = 0x99;
            assert_eq!(*ram.add(0x0042), 0x99);
            assert_eq!(retro_get_memory_size(0), 0, "no save RAM");
            assert!(retro_get_memory_data(0).is_null());
            retro_unload_game();
        }
    }

    #[test]
    fn reset_rebuilds_the_machine_and_sends_the_moved_map_again() {
        let _g = SERIAL.lock().unwrap();
        unsafe {
            load(&cart());
            SEEN.lock().unwrap().as_mut().unwrap().maps.clear();
            SEEN.lock().unwrap().as_mut().unwrap().map_ptrs.clear();
            retro_reset();
            let seen = SEEN.lock().unwrap();
            let s = seen.as_ref().unwrap();
            assert_eq!(s.maps.len(), 1, "reset must republish the map");
            assert_eq!(s.map_ptrs[0], retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM) as usize);
            drop(seen);
            retro_unload_game();
        }
    }

    /// What the fake frontend reports: joypad ids held, and analog values.
    #[derive(Default, Clone, Copy)]
    struct Pad {
        buttons: u32,
        left: (i16, i16),
        right: (i16, i16),
    }
    static PAD: Mutex<Pad> = Mutex::new(Pad { buttons: 0, left: (0, 0), right: (0, 0) });

    unsafe extern "C" fn input(port: c_uint, device: c_uint, index: c_uint, id: c_uint) -> i16 {
        if port != 0 {
            return 0;
        }
        let p = *PAD.lock().unwrap();
        match device {
            RETRO_DEVICE_JOYPAD => ((p.buttons >> id) & 1) as i16,
            RETRO_DEVICE_ANALOG => {
                let (x, y) = if index == ANALOG_LEFT { p.left } else { p.right };
                if id == AXIS_X { x } else { y }
            }
            _ => 0,
        }
    }

    /// Run the mapping once for a pad state, return controller 1 as POKEY
    /// and GTIA see it.
    fn read(pad: Pad) -> (Option<Key>, [u8; 2], bool, bool) {
        *PAD.lock().unwrap() = pad;
        let mut m = Machine::new(Cart::identified(cart()).unwrap(), None);
        unsafe { read_controller(&mut m, input, 0) };
        let c = m.bus.pokey.controllers[0];
        (c.key, c.pot, c.top, m.bus.gtia.trigger[0])
    }

    fn held(ids: &[c_uint]) -> Pad {
        Pad { buttons: ids.iter().map(|&i| 1u32 << i).sum(), ..Pad::default() }
    }

    #[test]
    fn the_intellivision_panel_keypad_lands_on_the_5200_keys() {
        let _g = SERIAL.lock().unwrap();
        // The panel pulses the right stick to +-0x7F00 on each axis.
        const D: i16 = 0x7f00;
        let grid = [
            ((-D, -D), Key::Digit(1)),
            ((0, -D), Key::Digit(2)),
            ((D, -D), Key::Digit(3)),
            ((-D, 0), Key::Digit(4)),
            ((D, 0), Key::Digit(6)),
            ((-D, D), Key::Digit(7)),
            ((0, D), Key::Digit(8)),
            ((D, D), Key::Digit(9)),
        ];
        for (right, key) in grid {
            assert_eq!(read(Pad { right, ..Pad::default() }).0, Some(key), "right stick {right:?}");
        }
        assert_eq!(read(held(&[JOY_R3])).0, Some(Key::Digit(5)));
        assert_eq!(read(held(&[JOY_L3])).0, Some(Key::Digit(0)));
        assert_eq!(read(held(&[JOY_L2])).0, Some(Key::Star), "Clear is *");
        assert_eq!(read(held(&[JOY_R2])).0, Some(Key::Hash), "Enter is #");
        assert_eq!(read(Pad::default()).0, None, "a centred stick is no key");
    }

    #[test]
    fn start_pause_reset_and_both_fire_pairs() {
        let _g = SERIAL.lock().unwrap();
        assert_eq!(read(held(&[JOY_START])).0, Some(Key::Start));
        assert_eq!(read(held(&[JOY_SELECT])).0, Some(Key::Pause));
        assert_eq!(read(held(&[JOY_L])).0, Some(Key::Reset));
        for b in [JOY_B, JOY_A] {
            let (_, _, top, bottom) = read(held(&[b]));
            assert!(bottom && !top, "{b} is the bottom fire");
        }
        for b in [JOY_Y, JOY_X] {
            let (_, _, top, bottom) = read(held(&[b]));
            assert!(top && !bottom, "{b} is the top fire");
        }
    }

    #[test]
    fn the_left_stick_is_proportional_and_the_dpad_is_full_travel() {
        let _g = SERIAL.lock().unwrap();
        assert_eq!(read(Pad::default()).1, [114, 114]);
        assert_eq!(read(Pad { left: (16384, -16384), ..Pad::default() }).1, [pot(16384), pot(-16384)]);
        assert_eq!(read(held(&[JOY_LEFT, JOY_DOWN])).1, [1, 227]);
        // The stick does not leak into the keypad, nor the keypad into it.
        assert_eq!(read(Pad { left: (i16::MAX, i16::MAX), ..Pad::default() }).0, None);
        assert_eq!(read(Pad { right: (i16::MAX, 0), ..Pad::default() }).1, [114, 114]);
    }

    #[test]
    fn serialize_through_the_c_abi_restores_and_refuses_garbage() {
        let _g = SERIAL.lock().unwrap();
        unsafe {
            load(&cart());
            for _ in 0..5 {
                retro_run();
            }
            let n = retro_serialize_size();
            assert_eq!(n, 16_907, "a literal: the size is fixed for every cartridge");
            let mut buf = vec![0u8; n];
            assert!(retro_serialize(buf.as_mut_ptr() as *mut c_void, n));
            assert!(!retro_serialize(buf.as_mut_ptr() as *mut c_void, n - 1), "too small a buffer");
            let ram_then = core().unwrap().machine.bus.ram.to_vec();
            for _ in 0..3 {
                retro_run();
            }
            core().unwrap().machine.bus.ram[0x10] ^= 0xff;
            assert!(retro_unserialize(buf.as_ptr() as *const c_void, n));
            assert_eq!(core().unwrap().machine.bus.ram.to_vec(), ram_then);
            let mut junk = buf.clone();
            junk[0] = 0;
            assert!(!retro_unserialize(junk.as_ptr() as *const c_void, n), "wrong magic");
            assert!(!retro_unserialize(buf.as_ptr() as *const c_void, n - 1), "truncated");
            assert!(!retro_unserialize(std::ptr::null(), n));
            retro_unload_game();
            assert_eq!(retro_serialize_size(), 0, "no game, no state");
        }
    }

    #[test]
    fn the_frontend_sees_the_normal_playfield_and_not_the_parked_objects() {
        // Mark columns in the 384-wide frame and see which survive.
        let mut frame = vec![0u8; WIDTH * HEIGHT];
        let mut pal = [[0u8; 3]; 256];
        for c in 0..256 {
            pal[c] = [c as u8, 0, 0];
        }
        for y in 0..HEIGHT {
            frame[y * WIDTH + 31] = 1; // colour clock $2F: outside
            frame[y * WIDTH + 32] = 2; // $30: the first visible
            frame[y * WIDTH + 351] = 3; // $CF: the last visible
            frame[y * WIDTH + 376] = 4; // $DC: Pole Position's blocks
        }
        frame[7 * WIDTH + 100] = 5; // line 15: above
        frame[8 * WIDTH + 100] = 6; // line 16: the first visible
        frame[231 * WIDTH + 100] = 7; // line 239: the last visible
        frame[232 * WIDTH + 100] = 8; // line 240: below
        let mut out = vec![0u32; 320 * 224];
        view(&frame, &pal, &mut out);
        let seen: std::collections::BTreeSet<u32> = out.iter().map(|v| v >> 16).collect();
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), vec![0, 2, 3, 6, 7]);
        assert_eq!(out[0] >> 16, 2);
        assert_eq!(out[319] >> 16, 3);
        assert_eq!(out[68] >> 16, 6);
        assert_eq!(out[223 * 320 + 68] >> 16, 7);
    }

    #[test]
    fn a_stick_axis_maps_to_the_pot_range_with_114_at_rest() {
        assert_eq!(pot(0), 114);
        assert_eq!(pot(i16::MIN), 1);
        assert_eq!(pot(i16::MAX), 227);
        assert!(pot(16384) > 160 && pot(16384) < 175);
    }
}
