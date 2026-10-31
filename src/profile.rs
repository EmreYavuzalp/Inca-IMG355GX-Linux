//! User profile: load/save as INI and turn it into the mouse's memory image.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::protocol::{self as p, Image, Step};

const COLOR_NAMES: &[(&str, u32)] = &[
    ("red", 0xFF0000), ("green", 0x00FF00), ("blue", 0x0000FF), ("yellow", 0xFFFF00),
    ("cyan", 0x00FFFF), ("magenta", 0xFF00FF), ("pink", 0xFF00FF), ("white", 0xFFFFFF),
    ("orange", 0xFF8000), ("purple", 0x8000FF), ("black", 0x000000),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Byte stored for the button
    Hw(u8),
    /// Consumer usage the mouse sends (media keys)
    Media(u16),
}

/// What a button does; `text` is how it is written in the profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub kind: ActionKind,
    pub text: String,
}

fn find<T: Copy>(table: &[(&str, T)], name: &str) -> Option<T> {
    table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

pub fn parse_action(text: &str) -> Result<Action> {
    let t = text.trim().to_lowercase();
    let kind = if let Some(code) = find(p::HW_FUNCTIONS, &t) {
        ActionKind::Hw(code)
    } else if let Some(usage) = find(p::CONSUMER_USAGES, &t) {
        ActionKind::Media(usage)
    } else {
        bail!("unknown action {text:?} (see \"img355ctl actions\")")
    };
    Ok(Action { kind, text: t })
}

pub fn parse_color(text: &str) -> Result<u32> {
    let t = text.trim().to_lowercase();
    let t = t.trim_start_matches('#');
    if let Some(c) = find(COLOR_NAMES, t) {
        return Ok(c);
    }
    let hex: String = if t.len() == 3 { t.chars().flat_map(|c| [c, c]).collect() } else { t.to_string() };
    match (hex.len(), u32::from_str_radix(&hex, 16)) {
        (6, Ok(c)) => Ok(c),
        _ => bail!("bad colour {text:?}: use a name or RRGGBB"),
    }
}

pub fn parse_effect(text: &str) -> Result<u8> {
    let t = text.trim().to_lowercase();
    match find(p::LED_EFFECTS, &t) {
        Some(e) => Ok(e),
        None => {
            let names: Vec<&str> = p::LED_EFFECTS.iter().map(|(n, _)| *n).collect();
            bail!("unknown LED effect {text:?}; choose one of {}", names.join(", "))
        }
    }
}

pub fn effect_name(effect: u8) -> &'static str {
    p::LED_EFFECTS.iter().find(|(_, e)| *e == effect).map(|(n, _)| *n).unwrap_or("?")
}

/// Parse "1 3 5" into flags for items 1..=n.
pub fn parse_levels(text: &str, n: usize, what: &str) -> Result<Vec<bool>> {
    let mut flags = vec![false; n];
    for tok in text.replace(',', " ").split_whitespace() {
        match tok.parse::<usize>() {
            Ok(i) if (1..=n).contains(&i) => flags[i - 1] = true,
            _ => bail!("{what}: numbers must be 1-{n}"),
        }
    }
    Ok(flags)
}

fn format_levels(flags: &[bool]) -> String {
    flags.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| (i + 1).to_string()).collect::<Vec<_>>().join(" ")
}

fn mask(flags: &[bool]) -> u8 {
    flags.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| 1u8 << i).sum()
}

/// Defaults equal the vendor software's "Reset" configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Buttons 1-6 as numbered in the vendor software
    pub buttons: Vec<Action>,
    pub dpi: Vec<u16>,
    pub enabled: Vec<bool>,
    pub colors: Vec<u32>,
    /// 0-based level the mouse starts in
    pub active: usize,
    /// Index into protocol::LED_EFFECTS
    pub led_effect: u8,
    pub led_speed: u8,
    pub cycle_colors: Vec<bool>,
    /// Sent in 8 ms units (216 -> 27)
    pub fire_ms: u32,
}

impl Default for Profile {
    fn default() -> Self {
        let img = Image::default();
        let buttons = ["left", "right", "middle", "forward", "back", "dpi-loop"];
        Profile {
            buttons: buttons.iter().map(|b| parse_action(b).unwrap()).collect(),
            dpi: (0..p::NUM_LEVELS).map(|l| img.dpi(l)).collect(),
            enabled: vec![true; p::NUM_LEVELS],
            colors: (0..p::NUM_LEVELS).map(|l| img.dpi_color(l)).collect(),
            active: 0,
            led_effect: 7,
            led_speed: 4,
            cycle_colors: vec![true; p::NUM_CYCLE_COLORS],
            fire_ms: 216,
        }
    }
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        if self.buttons.len() != p::NUM_BUTTONS || self.dpi.len() != p::NUM_LEVELS || self.colors.len() != p::NUM_LEVELS {
            bail!("profile needs {} buttons and {} DPI levels", p::NUM_BUTTONS, p::NUM_LEVELS);
        }
        if self.buttons[0].text != "left" {
            bail!("button 1 must stay \"left\" (the mouse needs a left click)");
        }
        for &d in &self.dpi {
            p::dpi_step(d)?;
        }
        if !self.enabled.iter().any(|&e| e) {
            bail!("at least one DPI level must stay enabled");
        }
        if self.active >= p::NUM_LEVELS {
            bail!("active level must be 1-6");
        }
        if !self.cycle_colors.iter().any(|&c| c) {
            bail!("at least one cycle colour must stay enabled");
        }
        self.image()?;
        Ok(())
    }

    pub fn image(&self) -> Result<Image> {
        let mut img = Image::default();
        for (i, a) in self.buttons.iter().enumerate() {
            match a.kind {
                ActionKind::Hw(code) => img.set_button(i, code),
                ActionKind::Media(usage) => img.set_media_button(i, usage),
            }
        }
        for level in 0..p::NUM_LEVELS {
            img.set_dpi(level, self.dpi[level])?;
            img.set_dpi_color(level, self.colors[level]);
        }
        img.set_enabled_levels(mask(&self.enabled));
        img.set_active_level(self.active);
        img.set_led(self.led_effect, self.led_speed)?;
        img.set_cycle_colors(mask(&self.cycle_colors));
        img.set_fire(self.fire_ms)?;
        Ok(img)
    }

    pub fn steps(&self) -> Result<Vec<Step>> {
        self.validate()?;
        Ok(self.image()?.steps())
    }
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("img355ctl").join("profile.ini")
}

type Ini = BTreeMap<String, Vec<(String, String)>>;

fn parse_ini(text: &str) -> Result<Ini> {
    let mut ini = Ini::new();
    let mut section: Option<String> = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = Some(name.trim().to_string());
            ini.entry(name.trim().to_string()).or_default();
        } else if let (Some(sec), Some((k, v))) = (&section, line.split_once('=')) {
            ini.get_mut(sec).unwrap().push((k.trim().to_string(), v.trim().to_string()));
        } else {
            bail!("line {}: expected [section] or key = value", n + 1);
        }
    }
    Ok(ini)
}

fn get<'a>(ini: &'a Ini, section: &str, key: &str) -> Option<&'a str> {
    ini.get(section)?.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn num<T: std::str::FromStr>(value: &str, what: &str) -> Result<T> {
    value.parse().map_err(|_| anyhow::anyhow!("{what}: bad number {value:?}"))
}

pub fn load(path: &Path) -> Result<Profile> {
    let mut prof = Profile::default();
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(prof),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let ini = parse_ini(&text).with_context(|| format!("{}", path.display()))?;
    (|| -> Result<()> {
        if let Some(v) = get(&ini, "dpi", "levels") {
            prof.dpi = v.replace(',', " ").split_whitespace().map(|d| num(d, "dpi.levels")).collect::<Result<_>>()?;
        }
        if let Some(v) = get(&ini, "dpi", "enabled") {
            prof.enabled = parse_levels(v, p::NUM_LEVELS, "dpi.enabled")?;
        }
        if let Some(v) = get(&ini, "dpi", "colors") {
            prof.colors = v.replace(',', " ").split_whitespace().map(parse_color).collect::<Result<_>>()?;
        }
        if let Some(v) = get(&ini, "dpi", "active") {
            prof.active = num::<usize>(v, "dpi.active")?.wrapping_sub(1);
        }
        for (k, v) in ini.get("buttons").into_iter().flatten() {
            match k.parse::<usize>() {
                Ok(i) if (1..=p::NUM_BUTTONS).contains(&i) => prof.buttons[i - 1] = parse_action(v)?,
                _ => bail!("button numbers are 1-{}", p::NUM_BUTTONS),
            }
        }
        if let Some(v) = get(&ini, "led", "effect") {
            prof.led_effect = parse_effect(v)?;
        }
        if let Some(v) = get(&ini, "led", "speed") {
            prof.led_speed = num(v, "led.speed")?;
        }
        if let Some(v) = get(&ini, "led", "cycle_colors") {
            prof.cycle_colors = parse_levels(v, p::NUM_CYCLE_COLORS, "led.cycle_colors")?;
        }
        if let Some(v) = get(&ini, "misc", "fire_interval_ms") {
            prof.fire_ms = num(v, "misc.fire_interval_ms")?;
        }
        prof.validate()
    })()
    .with_context(|| format!("{}", path.display()))?;
    Ok(prof)
}

pub fn save(prof: &Profile, path: &Path) -> Result<()> {
    prof.validate()?;
    let mut s = String::new();
    let section = |s: &mut String, name: &str, items: &[(String, String)]| {
        s.push_str(&format!("[{name}]\n"));
        for (k, v) in items {
            s.push_str(&format!("{k} = {v}\n"));
        }
        s.push('\n');
    };
    let kv = |k: &str, v: String| (k.to_string(), v);
    section(&mut s, "dpi", &[
        kv("levels", prof.dpi.iter().map(u16::to_string).collect::<Vec<_>>().join(" ")),
        kv("enabled", format_levels(&prof.enabled)),
        kv("active", (prof.active + 1).to_string()),
        kv("colors", prof.colors.iter().map(|c| format!("{c:06x}")).collect::<Vec<_>>().join(" ")),
    ]);
    let buttons: Vec<_> = prof.buttons.iter().enumerate().map(|(i, a)| ((i + 1).to_string(), a.text.clone())).collect();
    section(&mut s, "buttons", &buttons);
    section(&mut s, "led", &[
        kv("effect", effect_name(prof.led_effect).to_string()),
        kv("speed", prof.led_speed.to_string()),
        kv("cycle_colors", format_levels(&prof.cycle_colors)),
    ]);
    section(&mut s, "misc", &[kv("fire_interval_ms", prof.fire_ms.to_string())]);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, s).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Expected data for the report sequence.
    use super::*;
    use crate::factory;

    fn expected_steps() -> Vec<Step> {
        include_str!("../tests/data/factory-apply.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| {
                let (kind, hex) = l.split_once(' ').unwrap();
                if kind == "GET" {
                    return Step::Get;
                }
                let b: Vec<u8> = hex.split_whitespace().map(|x| u8::from_str_radix(x, 16).unwrap()).collect();
                Step::Set(b.try_into().unwrap())
            })
            .collect()
    }

    #[test]
    fn default_profile_is_the_factory_image() {
        assert_eq!(Profile::default().image().unwrap().0, factory::IMAGE);
    }

    #[test]
    fn reset_matches_expected_reports() {
        assert_eq!(Profile::default().steps().unwrap(), expected_steps());
    }

    #[test]
    fn factory_dpi_and_colours() {
        let prof = Profile::default();
        assert_eq!(prof.dpi, [1200, 1600, 2400, 3200, 4800, 7200]);
        assert_eq!(prof.colors, [0xFFFF00, 0x0000FF, 0xFF00FF, 0x00FF00, 0xFF0000, 0x00FFFF]);
    }

    /// Each case: one change and the bytes that change.
    #[test]
    fn single_changes_match_expected_images() {
        let cases: Vec<(Box<dyn Fn(&mut Profile)>, &[(usize, u8)])> = vec![
            (Box::new(|p| p.enabled[5] = false), &[(0x19, 0x1f)]),
            (Box::new(|p| p.dpi[0] = 1400), &[(0x15, 0x78)]),
            (Box::new(|p| p.dpi[0] = 7200), &[(0x13, 0x21), (0x15, 0x28)]),
            (Box::new(|p| p.dpi[1] = 200), &[(0x15, 0x61)]),
            (Box::new(|p| p.dpi[5] = 200), &[(0x13, 0x00), (0x17, 0xf1)]),
            (Box::new(|p| p.active = 5), &[(0x18, 0x1d)]),
            (Box::new(|p| p.fire_ms = 100), &[(0x1a, 0x0c)]),
            (Box::new(|p| p.led_effect = 0), &[(0x1c, 0x84)]),
            (Box::new(|p| p.led_effect = 9), &[(0x1c, 0x94), (0x33, 0xbf)]),
            (Box::new(|p| p.led_effect = 12), &[(0x1c, 0x9c)]),
            (Box::new(|p| p.led_effect = 13), &[(0x1c, 0x87)]),
            (Box::new(|p| p.led_speed = 1), &[(0x1c, 0xf1)]),
            (Box::new(|p| p.cycle_colors = vec![true, false, true, false, true, false, true]), &[(0x1d, 0xd5)]),
            (Box::new(|p| p.buttons[1] = parse_action("right").unwrap()), &[]),
            (Box::new(|p| p.buttons[5] = parse_action("dpi-").unwrap()), &[(0x0d, 0x2c)]),
            (Box::new(|p| p.buttons[5] = parse_action("led-loop").unwrap()), &[(0x0d, 0x35)]),
            (
                Box::new(|p| p.buttons[3] = parse_action("volume-up").unwrap()),
                &[(0x0c, 0x2f), (0xc8, 0x07), (0xca, 0xff), (0xcb, 0xe9), (0xce, 0x04)],
            ),
            (
                Box::new(|p| p.buttons[5] = parse_action("calculator").unwrap()),
                &[(0x0d, 0x2f), (0xd2, 0x07), (0xd4, 0xff), (0xd5, 0x92), (0xd6, 0x01), (0xd8, 0x04)],
            ),
        ];
        for (i, (change, expected)) in cases.iter().enumerate() {
            let mut prof = Profile::default();
            change(&mut prof);
            let img = prof.image().unwrap();
            let diff: Vec<(usize, u8)> =
                (0..factory::SIZE).filter(|&a| img.0[a] != factory::IMAGE[a]).map(|a| (a, img.0[a])).collect();
            assert_eq!(diff, *expected, "case {i}");
        }
    }

    #[test]
    fn colour_encoding() {
        let mut prof = Profile::default();
        prof.colors[0] = 0x00FFFF; // vendor INI DPI0 R=15 G=0 B=0 -> low nibbles f 0 0
        let img = prof.image().unwrap();
        assert_eq!([img.0[0x20] & 0xF, img.0[0x21] & 0xF, img.0[0x22] & 0xF], [0xF, 0x0, 0x0]);
        assert_eq!(img.0[0x21] >> 4, factory::IMAGE[0x21] >> 4); // user colour nibble kept
    }

    #[test]
    fn left_button_is_required() {
        let mut prof = Profile::default();
        prof.buttons[0] = parse_action("dpi+").unwrap();
        assert!(prof.validate().is_err());
    }

    #[test]
    fn profile_roundtrip() {
        let mut prof = Profile::default();
        prof.dpi[2] = 800;
        prof.colors[4] = 0x112233;
        prof.buttons[4] = parse_action("mute").unwrap();
        prof.led_effect = 9;
        let dir = std::env::temp_dir().join(format!("img355ctl-test-{}", std::process::id()));
        let path = dir.join("profile.ini");
        save(&prof, &path).unwrap();
        let loaded = load(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded, prof);
    }
}
