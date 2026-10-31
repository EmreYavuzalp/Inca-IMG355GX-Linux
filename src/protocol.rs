//! Wire protocol of the Inca IMG-355GX gaming mouse (USB 30fa:1040).
//!
//! The mouse keeps its settings in a small configuration memory, rewritten on
//! every "Apply" with 8-byte feature reports (report ID 7):
//!
//! ```text
//! 07 18 03 <i> <addr lo> <addr hi> <byte> <count>   write image[addr + i]
//! 07 18 09 00 <addr lo> <addr hi> 00 <count>        commit the block
//! 07 18 00 00 <addr lo> <addr hi> 00 00             end of block
//! ```
//!
//! The exact order of blocks and control reports is in `sequence.rs`. This
//! module edits the memory image.

use anyhow::{bail, Result};

use crate::factory;
use crate::sequence::{Op, APPLY};

pub const REPORT_ID: u8 = 7;
pub type Report = [u8; 8];

/// One step of an Apply: send a feature report, or read the status report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Set(Report),
    Get,
}

pub const NUM_BUTTONS: usize = 6;
pub const NUM_LEVELS: usize = 6;
pub const NUM_CYCLE_COLORS: usize = 7;

// Addresses in the configuration memory.
const BUTTONS: usize = 0x08; // one byte per button, in hardware order
const LEFT_MIRROR: usize = 0x6b; // function of the left button again (low 5 bits)
const DPI_HIGH_BITS: usize = 0x13; // bit n: bit 4 of level n's DPI step
const DPI_STEPS: usize = 0x15; // 4-bit DPI steps, two levels per byte, high nibble first
const ACTIVE_LEVEL: usize = 0x18; // 0x18 + level
const ENABLED_LEVELS: usize = 0x19; // bit n: level n is in the DPI button rotation
const FIRE: usize = 0x1a; // rapid-fire interval in 8 ms units
const LED_MODE: usize = 0x1c; // 0x80 | effect<<4 | speed
const CYCLE_COLORS: usize = 0x1d; // 0x80 | 7-bit mask
const DPI_COLORS: usize = 0x20; // 3 bytes per level (R, G, B), low nibble, active low
const LED_EFFECT_HIGH: usize = 0x33; // bit 7: bit 3 of the effect number
const MEDIA_SLOTS: usize = 0xa0; // 10 bytes per button (hardware order) for media keys

/// UI button (1-6, numbered as in the vendor software) -> hardware position.
const HW_POSITION: [usize; NUM_BUTTONS] = [0, 2, 1, 4, 3, 5];

/// Supported DPI values (2200 is skipped).
pub const DPI_VALUES: [u16; 17] = [
    200, 400, 600, 800, 1000, 1200, 1400, 1600, 1800, 2000, 2400, 3200, 4000, 4800, 5600, 6400, 7200,
];

/// Functions the firmware performs on its own. Values are the byte stored for the button.
pub const HW_FUNCTIONS: &[(&str, u8)] = &[
    ("left", 0x21),
    ("middle", 0x22),
    ("right", 0x23),
    ("back", 0x24),
    ("forward", 0x25),
    ("dpi-loop", 0x26),
    ("show-desktop", 0x27),
    ("double-click", 0x28),
    ("fire", 0x29),
    ("dpi+", 0x2b),
    ("dpi-", 0x2c),
    ("led-loop", 0x35),
    ("profile-1", 0x37),
    ("profile-2", 0x38),
    ("profile-3", 0x39),
    ("profile-4", 0x3a),
];
const MEDIA_FUNCTION: u8 = 0x2f;

/// Consumer-page usages for media keys; the mouse sends them itself.
pub const CONSUMER_USAGES: &[(&str, u16)] = &[
    ("browser", 0x223),
    ("player", 0x183),
    ("mail", 0x18A),
    ("volume-up", 0x0E9),
    ("volume-down", 0x0EA),
    ("previous", 0x0B6),
    ("next", 0x0B5),
    ("mute", 0x0E2),
    ("play-pause", 0x0CD),
    ("calculator", 0x192),
];

/// LED effects: (name, effect number).
pub const LED_EFFECTS: &[(&str, u8)] = &[
    ("dpi-breathing", 0),
    ("cycle-breathing", 1),
    ("static", 2),
    ("flowing-water", 3),
    ("mono-water", 4),
    ("comet", 5),
    ("neon", 6),
    ("ambilight", 7),
    ("flicker", 8),
    ("star-trek", 9),
    ("ripple", 10),
    ("enraptured", 11),
    ("button-response", 12),
    ("off", 13),
];
pub const SPEED_MIN: u8 = 1;
pub const SPEED_MAX: u8 = 6;

pub const FIRE_MIN_MS: u32 = 8;
pub const FIRE_MAX_MS: u32 = 255 * 8;

/// DPI -> 5-bit step: 200 DPI steps up to 2400, then 800 DPI steps.
pub fn dpi_step(dpi: u16) -> Result<u8> {
    match dpi {
        200..=2400 if dpi % 200 == 0 && dpi != 2200 => Ok((dpi / 200) as u8),
        3200..=7200 if dpi % 800 == 0 => Ok(12 + ((dpi - 2400) / 800) as u8),
        _ => bail!("unsupported DPI {dpi}; use 200-2400 in steps of 200, or 3200-7200 in steps of 800"),
    }
}

pub fn step_dpi(step: u8) -> u16 {
    if step <= 12 {
        step as u16 * 200
    } else {
        2400 + (step as u16 - 12) * 800
    }
}

/// The memory image, starting from the factory settings.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image(pub [u8; factory::SIZE]);

impl Default for Image {
    fn default() -> Self {
        Image(factory::IMAGE)
    }
}

impl Image {
    /// Button 1 must stay "left": the vendor software requires a left click and only
    /// the left-click value of its mirror byte (0x6b) is known.
    pub fn set_button(&mut self, ui_button: usize, function: u8) {
        let pos = HW_POSITION[ui_button];
        self.0[BUTTONS + pos] = function;
        if pos == 0 {
            self.0[LEFT_MIRROR] = function & 0x1F;
        }
        let slot = MEDIA_SLOTS + 10 * pos;
        self.0[slot..slot + 10].fill(0);
    }

    pub fn set_media_button(&mut self, ui_button: usize, usage: u16) {
        self.set_button(ui_button, MEDIA_FUNCTION);
        let slot = MEDIA_SLOTS + 10 * HW_POSITION[ui_button];
        self.0[slot..slot + 7].copy_from_slice(&[0x07, 0x00, 0xFF, usage as u8, (usage >> 8) as u8, 0x00, 0x04]);
    }

    pub fn set_dpi(&mut self, level: usize, dpi: u16) -> Result<()> {
        let step = dpi_step(dpi)?;
        let hi = &mut self.0[DPI_HIGH_BITS];
        *hi = (*hi & !(1 << level)) | ((step >> 4) << level);
        let byte = &mut self.0[DPI_STEPS + level / 2];
        *byte = if level % 2 == 0 { (*byte & 0x0F) | (step & 0xF) << 4 } else { (*byte & 0xF0) | (step & 0xF) };
        Ok(())
    }

    pub fn dpi(&self, level: usize) -> u16 {
        let byte = self.0[DPI_STEPS + level / 2];
        let low = if level % 2 == 0 { byte >> 4 } else { byte & 0xF };
        step_dpi(low | ((self.0[DPI_HIGH_BITS] >> level) & 1) << 4)
    }

    pub fn set_active_level(&mut self, level: usize) {
        self.0[ACTIVE_LEVEL] = 0x18 + level as u8;
    }

    pub fn set_enabled_levels(&mut self, mask: u8) {
        self.0[ENABLED_LEVELS] = mask & 0x3F;
    }

    pub fn set_fire(&mut self, ms: u32) -> Result<()> {
        if !(FIRE_MIN_MS..=FIRE_MAX_MS).contains(&ms) {
            bail!("fire interval must be {FIRE_MIN_MS}-{FIRE_MAX_MS} ms");
        }
        self.0[FIRE] = (ms / 8) as u8;
        Ok(())
    }

    pub fn set_led(&mut self, effect: u8, speed: u8) -> Result<()> {
        if !(SPEED_MIN..=SPEED_MAX).contains(&speed) {
            bail!("LED speed must be {SPEED_MIN}-{SPEED_MAX}");
        }
        let (mode, high) = match effect {
            0..=11 => (0x80 | (effect & 7) << 4 | speed, effect >= 8),
            12 => (0x98 | speed, false), // button response: cycle breathing + reaction flag
            13 => (0x87, false),         // off
            _ => bail!("unknown LED effect {effect}"),
        };
        self.0[LED_MODE] = mode;
        let h = &mut self.0[LED_EFFECT_HIGH];
        *h = (*h & 0x7F) | (high as u8) << 7;
        Ok(())
    }

    pub fn set_cycle_colors(&mut self, mask: u8) {
        self.0[CYCLE_COLORS] = 0x80 | (mask & 0x7F);
    }

    /// 24-bit RGB; stored as 4 bits per channel, active low, in the low nibbles.
    pub fn set_dpi_color(&mut self, level: usize, rgb: u32) {
        for (ch, shift) in [16, 8, 0].into_iter().enumerate() {
            let c = (rgb >> shift) & 0xFF;
            let nibble = 15 - ((c * 15 + 127) / 255) as u8;
            let b = &mut self.0[DPI_COLORS + 3 * level + ch];
            *b = (*b & 0xF0) | nibble;
        }
    }

    pub fn dpi_color(&self, level: usize) -> u32 {
        (0..3).fold(0, |rgb, ch| {
            let n = 15 - (self.0[DPI_COLORS + 3 * level + ch] & 0xF) as u32;
            rgb << 8 | (n * 17)
        })
    }

    /// The complete report sequence of an Apply.
    pub fn steps(&self) -> Vec<Step> {
        let mut out = Vec::with_capacity(900);
        for op in APPLY {
            match op {
                Op::Get => out.push(Step::Get),
                Op::Raw(r) => out.push(Step::Set(r)),
                Op::Write { addr, len, count, fixed, fixed_mask } => {
                    for i in 0..len {
                        let byte = if fixed_mask >> i & 1 == 1 { fixed[i as usize] } else { self.0[addr as usize + i as usize] };
                        out.push(Step::Set([REPORT_ID, 0x18, 0x03, i, addr as u8, (addr >> 8) as u8, byte, count]));
                    }
                }
            }
        }
        out
    }
}

/// Status report read during an Apply: `07 <00|41> 80 06 1d 2c ...`.
pub fn get_request() -> Report {
    [REPORT_ID, 0, 0, 0, 0, 0, 0, 0]
}
