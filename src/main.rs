use anyhow::{bail, Context, Result};
use hidapi::{HidApi, HidDevice};
use once_cell::sync::Lazy;
use rusb::{Direction, RequestType, TransferType};
use rusb::UsbContext;
use std::collections::HashMap;
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant };
use std::time;
use uinput::event::controller;
use uinput::event::absolute::Position;
use uinput::event::absolute;
use uinput::event::Event;
use uinput::device::{Builder, Device};

const VENDOR_ID: u16 = 0x057E; // Nintendo
const PRODUCT_ID: u16 = 0x2069; // "Pro Controller 2" (Switch 2 generation)
const USB_INTERFACE: u8 = 1;    // same as Pro Con 1

// ────────────────── Handshake payloads lifted from original HTML page ────────
// NOTE: the MAC‑address and LTK placeholders (0xFF) are left as‑is; the
// controller accepts them when not paired.
static INIT_COMMAND_0X03: &[u8] = &[0x03, 0x91, 0x00, 0x0d, 0x00, 0x08,
    0x00, 0x00, 0x01, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
static UNKNOWN_COMMAND_0X07: &[u8] = &[0x07, 0x91, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
static UNKNOWN_COMMAND_0X16: &[u8] = &[0x16, 0x91, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
static REQUEST_CONTROLLER_MAC: &[u8] = &[0x15, 0x91, 0x00, 0x01, 0x00, 0x0e,
    0x00, 0x00, 0x00, 0x02,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff];
static LTK_REQUEST: &[u8] = &[0x15, 0x91, 0x00, 0x02, 0x00, 0x11,
    0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
static UNKNOWN_COMMAND_0X15_ARG3: &[u8] = &[0x15, 0x91, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00];
static UNKNOWN_COMMAND_0X09: &[u8] = &[0x09, 0x91, 0x00, 0x07, 0x00, 0x08,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
static IMU_COMMAND_0X02: &[u8] = &[0x0c, 0x91, 0x00, 0x02, 0x00, 0x04,
    0x00, 0x00, 0x27, 0x00, 0x00, 0x00];
static OUT_UNKNOWN_COMMAND_0X11: &[u8] = &[0x11, 0x91, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00];
static UNKNOWN_COMMAND_0X0A: &[u8] = &[0x0a, 0x91, 0x00, 0x08, 0x00, 0x14,
    0x00, 0x00, 0x01,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x35, 0x00, 0x46,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
static IMU_COMMAND_0X04: &[u8] = &[0x0c, 0x91, 0x00, 0x04, 0x00, 0x04,
    0x00, 0x00, 0x27, 0x00, 0x00, 0x00];
static ENABLE_HAPTICS: &[u8] = &[0x03, 0x91, 0x00, 0x0a, 0x00, 0x04,
    0x00, 0x00, 0x09, 0x00, 0x00, 0x00];
static OUT_UNKNOWN_COMMAND_0X10: &[u8] = &[0x10, 0x91, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
static OUT_UNKNOWN_COMMAND_0X01: &[u8] = &[0x01, 0x91, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00];
static OUT_UNKNOWN_COMMAND_0X03: &[u8] = &[0x03, 0x91, 0x00, 0x01, 0x00, 0x00, 0x00];
static OUT_UNKNOWN_COMMAND_0X0A_ALT: &[u8] = &[0x0a, 0x91, 0x00, 0x02, 0x00, 0x04,
    0x00, 0x00, 0x03, 0x00, 0x00];
static SET_PLAYER_LED: &[u8] = &[0x09, 0x91, 0x00, 0x07, 0x00, 0x08,
    0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

static HANDSHAKE_SEQUENCE: &[&[u8]] = &[
    INIT_COMMAND_0X03,
    UNKNOWN_COMMAND_0X07,
    UNKNOWN_COMMAND_0X16,
    REQUEST_CONTROLLER_MAC,
    LTK_REQUEST,
    UNKNOWN_COMMAND_0X15_ARG3,
    UNKNOWN_COMMAND_0X09,
    IMU_COMMAND_0X02,
    OUT_UNKNOWN_COMMAND_0X11,
    UNKNOWN_COMMAND_0X0A,
    IMU_COMMAND_0X04,
    ENABLE_HAPTICS,
    OUT_UNKNOWN_COMMAND_0X10,
    OUT_UNKNOWN_COMMAND_0X01,
    OUT_UNKNOWN_COMMAND_0X03,
    OUT_UNKNOWN_COMMAND_0X0A_ALT,
    SET_PLAYER_LED,
];

// ─────────────────────────────────────────────────────────────────────────────
// Input mapping helpers
#[derive(Default, Clone, Copy, Debug)]
struct State { buttons: u32, lx: i16, ly: i16, rx: i16, ry: i16 }


// bit masks – identical to the Linux driver constants
mod bit {
    pub const A:        u32 = 1 << 0;   // 0x0000_0001
    pub const B:        u32 = 1 << 1;   // 0x0000_0002
    pub const Y:        u32 = 1 << 2;   // 0x0000_0004
    pub const X:        u32 = 1 << 3;   // 0x0000_0008
    pub const R:        u32 = 1 << 4;   // 0x0000_0010
    pub const ZR:       u32 = 1 << 5;   // 0x0000_0020
    pub const PLUS:     u32 = 1 << 6;   // 0x0000_0040
    pub const R_STICK:  u32 = 1 << 7;   // 0x0000_0080

    pub const DOWN:     u32 = 1 << 8;   // 0x0000_0100
    pub const RIGHT:    u32 = 1 << 9;   // 0x0000_0200
    pub const LEFT:     u32 = 1 << 10;  // 0x0000_0400
    pub const UP:       u32 = 1 << 11;  // 0x0000_0800
    pub const L:        u32 = 1 << 12;  // 0x0000_1000
    pub const ZL:       u32 = 1 << 13;  // 0x0000_2000
    pub const MINUS:    u32 = 1 << 14;  // 0x0000_4000
    pub const L_STICK:  u32 = 1 << 15;  // 0x0000_8000

    pub const HOME:     u32 = 1 << 16;  // 0x0001_0000
    pub const CAPTURE:  u32 = 1 << 17;  // 0x0002_0000
    // bits 18-20 exist on your pad (GR/GL/CHAT) — keep free for later
}
// Virtual device & mapping table
struct Mapper {
    dev: Device,
    prev: State,
}

impl Mapper {
    fn new() -> Result<Self> {
        let dev = Builder::default()?
            .name("ProCon2 (virt)")?
            // Buttons
            .event(controller::GamePad::South)? // B
            .event(controller::GamePad::East)?  // A
            .event(controller::GamePad::West)?  // Y
            .event(controller::GamePad::North)? // X
            .event(controller::GamePad::TL)?    // L
            .event(controller::GamePad::TR)?    // R
            .event(controller::GamePad::TL2)?   // ZL
            .event(controller::GamePad::TR2)?   // ZR
            .event(controller::GamePad::Select)?// Minus
            .event(controller::GamePad::Start)? // Plus
            .event(controller::GamePad::Mode)?  // Home
            .event(controller::GamePad::ThumbL)?
            .event(controller::GamePad::ThumbR)?
            .event(controller::GamePad::C)?
            // D‑pad
            .event(controller::DPad::Left)?
            .event(controller::DPad::Right)?
            .event(controller::DPad::Up)?
            .event(controller::DPad::Down)?
            // Axes (–32767..32767)
            .event(absolute::Position::X)? // LX
            .event(absolute::Position::Y)? // LY
            .event(absolute::Position::RX)? // RX
            .event(absolute::Position::RY)? // RY
            .create()?;
        Ok(Self { dev, prev: State::default() })
    }
        /// Helper: set button state on underlying uinput device
    fn set_button(&mut self, pressed: bool, btn: &controller::GamePad) -> Result<()> {
        if pressed {
            self.dev.press(btn)?;
        } else {
            self.dev.release(btn)?;
        }
        Ok(())
    }

    /// Helper: set d‑pad direction on underlying uinput device
    fn set_hat(&mut self, pressed: bool, dir: &controller::DPad) -> Result<()> {
        if pressed {
            self.dev.press(dir)?;
        } else {
            self.dev.release(dir)?;
        }
        Ok(())
    }
    fn emit(&mut self, new: State) -> Result<()> {
        // println!("[DEBUG] emit: prev state = {:?}, new state = {:?}", self.prev, new);
        let mut emit = |cond: bool, event: Event, value: i32| -> Result<()> {
            if cond {
                self.dev.send(event, value)?;
            }
            Ok(())
        };
        // Buttons
        macro_rules! cmp_btn {
            ($mask:ident, $btn:expr) => {
                if (self.prev.buttons ^ new.buttons) & bit::$mask != 0 {
                    self.set_button(new.buttons & bit::$mask != 0, &$btn)?;
                }
            };
        }
        use controller::GamePad::*;
        cmp_btn!(B, South);
        cmp_btn!(A, East);
        cmp_btn!(Y, West);
        cmp_btn!(X, North);
        cmp_btn!(L, TL);
        cmp_btn!(R, TR);
        cmp_btn!(ZL, TL2);
        cmp_btn!(ZR, TR2);
        cmp_btn!(MINUS, Select);
        cmp_btn!(PLUS, Start);
        cmp_btn!(HOME, Mode);
        cmp_btn!(L_STICK, ThumbL);
        cmp_btn!(R_STICK, ThumbR);
        cmp_btn!(CAPTURE, C);
        // D‑pad
        macro_rules! cmp_hat {
            ($mask:ident, $dir:expr) => {
                if (self.prev.buttons ^ new.buttons) & bit::$mask != 0 {
                    self.set_hat(new.buttons & bit::$mask != 0, &$dir)?;
                }
            };
        }
        cmp_hat!(LEFT, controller::DPad::Left);
        cmp_hat!(RIGHT, controller::DPad::Right);
        cmp_hat!(UP, controller::DPad::Up);
        cmp_hat!(DOWN, controller::DPad::Down);
        // Axes – only emit if changed by ≥ 32 to avoid spam
        if (new.lx - self.prev.lx).abs() > 32 {
            self.dev.send(Position::X, new.lx as i32)?;
        }
        if (new.ly - self.prev.ly).abs() > 32 {
            self.dev.send(Position::Y, new.ly as i32)?;
        }
        if (new.rx - self.prev.rx).abs() > 32 {
            self.dev.send(Position::RX, new.rx as i32)?;
        }
        if (new.ry - self.prev.ry).abs() > 32 {
            self.dev.send(Position::RY, new.ry as i32)?;
        }
        self.dev.synchronize()?;
        self.prev = new;
        Ok(())
    }
}

// ────────────────── USB initialisation sequence via libusb ───────────────────
fn run_handshake() -> Result<()> {
    let ctx = rusb::Context::new()?;
    let devices = ctx.devices()?;
    let device = devices
        .iter()
        .find(|d| {
            let desc = d.device_descriptor().ok().unwrap();
            desc.vendor_id() == VENDOR_ID && desc.product_id() == PRODUCT_ID
        })
        .context("ProCon2 USB device not found")?;

    let handle = device.open()?;
    handle.claim_interface(USB_INTERFACE.into())?;
    if handle.kernel_driver_active(USB_INTERFACE.into())? {
        println!("detatch controller from kernel...");
        handle.detach_kernel_driver(USB_INTERFACE.into())?;
    }
    let config_descriptor = device.config_descriptor(0)?;
    let ep = config_descriptor
        .interfaces()
        .find(|i| i.number() == USB_INTERFACE)
        .and_then(|i| i.descriptors().next())
        .and_then(|alt| {
            alt.endpoint_descriptors()
                .find(|e| e.transfer_type() == TransferType::Bulk && e.direction() == Direction::Out)
        })
        .context("bulk‑out endpoint not found")?;
    let addr = ep.address();

    for packet in HANDSHAKE_SEQUENCE {
        handle.write_bulk(addr, packet, Duration::from_millis(5))?;
        thread::sleep(Duration::from_millis(3));
    }

    eprintln!("[init] USB handshake finished");
    Ok(())
}

// ─────────────── HID reading & translation to virtual device ─────────────────
fn open_hid() -> Result<HidDevice> {
    let api = HidApi::new()?;
    let dev = api
        .device_list()
        .find(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID)
        .context("hid device not found (plug in via USB‑C)")?
        .open_device(&api)?;
    dev.set_blocking_mode(false)?;
    Ok(dev)
}

// ---------------------------------------------------------------------------
fn parse_report(buf: &[u8]) -> Option<State> {
    // println!("[DEBUG] parse_report: raw_buffer = {:?}", buf);
    match buf.first()? {
        0x30 => parse_full_30(buf), // BT full report
        0x3F => parse_simple_3f(buf),
        0x09 => parse_full_09(buf), // Switch‑2 USB full report (new)
        _ => None,
    }
}

// ---------------- 0x09: new USB full report ----------------
fn parse_full_09(b: &[u8]) -> Option<State> {
    if b.len() < 12 { return None; }
    // layout: 0:id 1‑2:timer 3‑5:buttons 6‑11:sticks …
    let btn = 3;
    let mut st = State::default();
    st.buttons = b[btn] as u32 | (b[btn+1] as u32) << 8 | (b[btn+2] as u32) << 16;
    decode_sticks(&b[btn+3..btn+9], &mut st);
    Some(st)
}

// ---------------- 0x30: classic full report ----------------
fn parse_full_30(b: &[u8]) -> Option<State> {
    if b.len() < 13 { return None; }
    // layout: 0:id 1‑2:timer 3:status 4‑6:buttons 7‑12:sticks …
    let btn = 4;
    let mut st = State::default();
    st.buttons = b[btn] as u32 | (b[btn+1] as u32) << 8 | (b[btn+2] as u32) << 16;
    decode_sticks(&b[btn+3..btn+9], &mut st);
    Some(st)
}

// ---------------- 0x3F: simple report ----------------
fn parse_simple_3f(b: &[u8]) -> Option<State> {
    if b.len() < 12 { return None; }
    let mut st = State::default();
    st.buttons = b[1] as u32 | (b[2] as u32) << 8 | ((hat_bits(b[3]) as u32) << 16);
    st.lx = i16::from_le_bytes([b[4], b[5]]);
    st.ly = i16::from_le_bytes([b[6], b[7]]);
    st.rx = i16::from_le_bytes([b[8], b[9]]);
    st.ry = i16::from_le_bytes([b[10], b[11]]);
    Some(st)
}

// -------- helpers -----------------------------------------------------------
fn hat_bits(h: u8) -> u8 { // up down left right bits (1,2,3,0 order)
    match h & 0x0F { 0 => 0b0010, 1 => 0b00110, 2 => 0b00100, 3 => 0b01100,
        4 => 0b01000, 5 => 0b01001, 6 => 0b00001, 7 => 0b00011, _ => 0 }
}

fn decode_sticks(src: &[u8], st: &mut State) {
    // src[0..5] = LX(12) LY(12) RX(12) RY(12
    let lx_raw = ((src[0] as u16) | (((src[1] & 0x0F) as u16) << 8)) as i32;
    let ly_raw = (((src[1] as u16) >> 4) |  ((src[2] as u16) << 4))  as i32;
    let rx_raw = ((src[3] as u16) | (((src[4] & 0x0F) as u16) << 8)) as i32;
    let ry_raw = (((src[4] as u16) >> 4) |  ((src[5] as u16) << 4))  as i32;

    let map = |v: i32| {
        let c = v - 2048;                      // centre
        if c.abs() < 200 { 0 }
        else { ((c * 32767) / 2048)
               .clamp(-32767, 32767) as i16 }
    };

    st.lx =  map(lx_raw);
    st.ly =  -map(ly_raw);
    st.rx =  map(rx_raw);
    st.ry =  -map(ry_raw);
}

// ───────────────────────────── Main loop ────────────────────────────────────
fn main() -> Result<()> {
    env_logger::init();

    loop {
        if let Err(e) = run_handshake() {
            eprintln!("[error] USB init failed: {e}");
            thread::sleep(std::time::Duration::new(5, 0));
            continue;
        }

        let hid = open_hid()?;
        let mut mapper = Mapper::new()?;
        let mut buf = [0u8; 64];

        loop {
            match hid.read_timeout(&mut buf, 20) {
                Ok(n) if n > 0 => {
                    if let Some(state) = parse_report(&buf[..n]) {
                        if let Err(e) = mapper.emit(state) {
                            eprintln!("[uinput] emit error: {e}");
                        }
                    }
                }
                Ok(_) => { /* timeout – nothing */ }
                Err(e) => {
                    eprintln!("[hid] read error: {e}");
                    break;
                }
            }
        }
    }
}
