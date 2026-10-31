//! img355ctl: configure the Inca IMG-355GX gaming mouse on Linux.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use img355ctl::device;
use img355ctl::profile::{self, Profile};
use img355ctl::protocol::{self as p, Step};

#[derive(Parser)]
#[command(version, about = "Configure the Inca IMG-355GX gaming mouse.")]
struct Cli {
    /// Profile file [default: ~/.config/img355ctl/profile.ini]
    #[arg(long, global = true, value_name = "FILE")]
    profile: Option<PathBuf>,
    /// hidraw node [default: auto-detect]
    #[arg(long, global = true, value_name = "PATH")]
    device: Option<PathBuf>,
    /// Print the reports instead of sending them (the profile is not saved)
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the detected mouse and the profile
    Status,
    /// Send the whole profile to the mouse
    Apply,
    /// Restore factory defaults (like the vendor "Reset")
    Reset,
    /// List what buttons can be bound to, and the LED effects
    Actions,
    /// Set a level's DPI, or which levels are enabled / active
    Dpi {
        /// DPI level 1-6
        #[arg(value_parser = clap::value_parser!(u8).range(1..=6))]
        level: u8,
        /// 200-2400 in steps of 200 (not 2200), or 3200-7200 in steps of 800
        value: Option<u16>,
        /// Enable the level
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Disable the level
        #[arg(long)]
        off: bool,
        /// Start in this level
        #[arg(long)]
        active: bool,
    },
    /// Set a DPI level's LED colour
    Color {
        /// DPI level 1-6, or "all"
        #[arg(value_parser = parse_level_or_all)]
        level: u8,
        /// RRGGBB or a name (red, orange, cyan, ...)
        color: String,
    },
    /// Bind a button (2-6, numbered as in the vendor tool; 1 is always left)
    Button {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=6))]
        button: u8,
        /// See "img355ctl actions"
        action: String,
    },
    /// LED effect
    Led {
        /// See "img355ctl actions"
        effect: String,
        /// Effect speed 1-6
        #[arg(long)]
        speed: Option<u8>,
        /// Which of the 7 cycle colours to use, e.g. "1 3 5"
        #[arg(long)]
        colors: Option<String>,
    },
    /// Repeat interval of the "fire" function
    Fire { ms: u32 },
}

/// "1".."6", or "all" (returned as 0).
fn parse_level_or_all(s: &str) -> Result<u8, String> {
    match s {
        "all" => Ok(0),
        _ => match s.parse::<u8>() {
            Ok(n) if (1..=6).contains(&n) => Ok(n),
            _ => Err("level must be 1-6 or \"all\"".into()),
        },
    }
}

fn show(prof: &Profile) {
    println!("DPI levels:");
    for i in 0..p::NUM_LEVELS {
        let mark = if i == prof.active { '*' } else { ' ' };
        let state = if prof.enabled[i] { "on " } else { "off" };
        println!(" {mark}{}: {:>4} dpi  {state}  #{:06x}", i + 1, prof.dpi[i], prof.colors[i]);
    }
    println!("Buttons:");
    for (i, a) in prof.buttons.iter().enumerate() {
        println!("  {}: {}", i + 1, a.text);
    }
    let cyc: Vec<String> = (0..p::NUM_CYCLE_COLORS).filter(|&i| prof.cycle_colors[i]).map(|i| (i + 1).to_string()).collect();
    println!("LED: {}, speed {}, cycle colours {}", profile::effect_name(prof.led_effect), prof.led_speed, cyc.join(" "));
    println!("Fire interval: {} ms", prof.fire_ms);
}

fn names<T>(table: &[(&str, T)]) -> String {
    table.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" ")
}

fn actions() {
    println!("Button functions (all handled by the mouse itself):");
    println!("  {}", names(p::HW_FUNCTIONS));
    println!("Media keys:");
    println!("  {}", names(p::CONSUMER_USAGES));
    println!("LED effects:");
    println!("  {}", names(p::LED_EFFECTS));
}

fn modify(cmd: Cmd, mut prof: Profile) -> Result<Profile> {
    match cmd {
        Cmd::Reset => return Ok(Profile::default()),
        Cmd::Dpi { level, value, on, off, active } => {
            let i = level as usize - 1;
            if value.is_none() && !(on || off || active) {
                bail!("give a DPI value or --on/--off/--active");
            }
            if let Some(v) = value {
                p::dpi_step(v)?;
                prof.dpi[i] = v;
            }
            if on || off {
                prof.enabled[i] = on;
            }
            if active {
                prof.active = i;
                prof.enabled[i] = true;
            }
        }
        Cmd::Color { level, color } => {
            let c = profile::parse_color(&color)?;
            match level {
                0 => prof.colors.iter_mut().for_each(|x| *x = c),
                n => prof.colors[n as usize - 1] = c,
            }
        }
        Cmd::Button { button, action } => {
            let a = profile::parse_action(&action)?;
            if button == 1 && a.text != "left" {
                bail!("button 1 must stay \"left\" (the mouse needs a left click)");
            }
            prof.buttons[button as usize - 1] = a;
        }
        Cmd::Led { effect, speed, colors } => {
            prof.led_effect = profile::parse_effect(&effect)?;
            if let Some(s) = speed {
                prof.led_speed = s;
            }
            if let Some(c) = colors {
                prof.cycle_colors = profile::parse_levels(&c, p::NUM_CYCLE_COLORS, "--colors")?;
            }
        }
        Cmd::Fire { ms } => prof.fire_ms = ms,
        Cmd::Status | Cmd::Apply | Cmd::Actions => {}
    }
    prof.validate()?;
    Ok(prof)
}

fn run(cli: Cli) -> Result<()> {
    let path = cli.profile.clone().unwrap_or_else(profile::default_path);
    let prof = profile::load(&path)?;
    match cli.cmd {
        Cmd::Actions => actions(),
        Cmd::Status => {
            let found: Vec<String> = device::find().iter().map(|p| p.display().to_string()).collect();
            println!("Mouse: {}", if found.is_empty() { "not found".to_string() } else { found.join(", ") });
            println!("Profile: {}", path.display());
            show(&prof);
        }
        cmd => {
            let saving = !matches!(cmd, Cmd::Apply);
            let prof = if saving { modify(cmd, prof)? } else { prof };
            let steps = prof.steps()?;
            if cli.dry_run {
                for s in &steps {
                    match s {
                        Step::Set(r) => println!("{}", r.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")),
                        Step::Get => println!("GET"),
                    }
                }
                if saving {
                    eprintln!("(dry run: profile not saved)");
                }
                return Ok(());
            }
            device::Mouse::open(cli.device.as_deref())?.run(&steps)?;
            if saving {
                profile::save(&prof, &path)?;
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    // Exit quietly when stdout is closed early (e.g. `img355ctl status | head`).
    // SAFETY: called before any other thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("img355ctl: {e:#}");
            ExitCode::FAILURE
        }
    }
}
