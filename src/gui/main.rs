//! img355ctl-gui: graphical settings for the Inca IMG-355GX gaming mouse.

mod i18n;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use i18n::{Strings, NAMES};
use img355ctl::device;
use img355ctl::profile::{self, Profile};
use img355ctl::protocol::{self as p, CONSUMER_USAGES, DPI_VALUES, HW_FUNCTIONS, LED_EFFECTS};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Dpi,
    Led,
    Buttons,
    Other,
}

enum Status {
    None,
    Ok(String),
    Err(String),
}

struct App {
    t: &'static Strings,
    turkish: bool,
    path: PathBuf,
    /// The profile being edited and the one last saved/applied.
    prof: Profile,
    saved: Profile,
    tab: Tab,
    status: Status,
    mouse: Vec<PathBuf>,
    last_poll: Instant,
}

const STATIC_EFFECT: u8 = 2;

fn rgb_array(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

fn rgb_u32(a: [u8; 3]) -> u32 {
    (a[0] as u32) << 16 | (a[1] as u32) << 8 | a[2] as u32
}

impl App {
    fn new() -> Self {
        let turkish = i18n::turkish();
        let path = profile::default_path();
        let (prof, status) = match profile::load(&path) {
            Ok(p) => (p, Status::None),
            Err(e) => (Profile::default(), Status::Err(format!("{e:#}"))),
        };
        App {
            t: if turkish { &i18n::TR } else { &i18n::EN },
            turkish,
            path,
            saved: prof.clone(),
            prof,
            tab: Tab::Dpi,
            status,
            mouse: Vec::new(),
            last_poll: Instant::now() - Duration::from_secs(10),
        }
    }

    fn label(&self, id: &str) -> String {
        match NAMES.iter().find(|(n, _, _)| *n == id) {
            Some((_, tr, en)) => (if self.turkish { tr } else { en }).to_string(),
            None => id.to_string(),
        }
    }

    fn poll(&mut self) {
        if self.last_poll.elapsed() >= Duration::from_secs(2) {
            self.last_poll = Instant::now();
            self.mouse = device::find();
        }
    }

    fn apply(&mut self) {
        let result = self.prof.steps().and_then(|steps| {
            device::Mouse::open(None)?.run(&steps)?;
            profile::save(&self.prof, &self.path)
        });
        self.status = match result {
            Ok(()) => {
                self.saved = self.prof.clone();
                Status::Ok(self.t.applied.to_string())
            }
            Err(e) => Status::Err(format!("{e:#}")),
        };
    }

    fn dpi_tab(&mut self, ui: &mut egui::Ui) {
        let t = self.t;
        egui::Grid::new("dpi").num_columns(5).spacing([24.0, 10.0]).striped(true).show(ui, |ui| {
            for h in [t.level, t.start, t.enabled, "DPI", t.color] {
                ui.strong(h);
            }
            ui.end_row();
            for i in 0..p::NUM_LEVELS {
                ui.label(format!("{}", i + 1));
                if ui.radio(self.prof.active == i, "").clicked() {
                    self.prof.active = i;
                    self.prof.enabled[i] = true;
                }
                let only_one = self.prof.enabled.iter().filter(|&&e| e).count() == 1 && self.prof.enabled[i];
                ui.add_enabled_ui(!only_one && self.prof.active != i, |ui| ui.checkbox(&mut self.prof.enabled[i], ""));
                egui::ComboBox::from_id_salt(("dpi", i))
                    .selected_text(self.prof.dpi[i].to_string())
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for v in DPI_VALUES {
                            ui.selectable_value(&mut self.prof.dpi[i], v, v.to_string());
                        }
                    });
                let mut rgb = rgb_array(self.prof.colors[i]);
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    self.prof.colors[i] = rgb_u32(rgb);
                }
                ui.end_row();
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new(t.color_note).weak());
    }

    fn led_tab(&mut self, ui: &mut egui::Ui) {
        let t = self.t;
        ui.horizontal(|ui| {
            ui.label(t.effect);
            let current = self.label(profile::effect_name(self.prof.led_effect));
            let labels: Vec<(u8, String)> = LED_EFFECTS.iter().map(|(name, e)| (*e, self.label(name))).collect();
            egui::ComboBox::from_id_salt("effect").selected_text(current).width(200.0).height(420.0).show_ui(ui, |ui| {
                for (e, label) in labels {
                    ui.selectable_value(&mut self.prof.led_effect, e, label);
                }
            });
        });
        ui.add_space(12.0);
        if self.prof.led_effect == STATIC_EFFECT {
            ui.horizontal(|ui| {
                ui.label(t.static_color);
                let mut rgb = rgb_array(self.prof.colors[self.prof.active]);
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    let c = rgb_u32(rgb);
                    self.prof.colors.iter_mut().for_each(|x| *x = c);
                }
            });
            ui.label(RichText::new(t.static_note).weak());
            ui.add_space(12.0);
        }
        let off = profile::effect_name(self.prof.led_effect) == "off";
        ui.add_enabled_ui(!off, |ui| {
            ui.horizontal(|ui| {
                ui.label(t.speed);
                ui.add(egui::Slider::new(&mut self.prof.led_speed, p::SPEED_MIN..=p::SPEED_MAX));
            });
        });
        ui.add_space(8.0);
        ui.label(t.cycle_colors);
        ui.horizontal(|ui| {
            for i in 0..p::NUM_CYCLE_COLORS {
                let only_one = self.prof.cycle_colors.iter().filter(|&&c| c).count() == 1 && self.prof.cycle_colors[i];
                ui.add_enabled_ui(!only_one, |ui| ui.checkbox(&mut self.prof.cycle_colors[i], format!("{}", i + 1)));
            }
        });
    }

    fn buttons_tab(&mut self, ui: &mut egui::Ui) {
        let t = self.t;
        egui::Grid::new("buttons").num_columns(2).spacing([16.0, 10.0]).striped(true).show(ui, |ui| {
            for i in 0..p::NUM_BUTTONS {
                ui.label(format!("{}  {}", i + 1, t.button_names[i]));
                if i == 0 {
                    ui.add_enabled(false, egui::Button::new(t.left_fixed).min_size(egui::vec2(220.0, 0.0)));
                    ui.end_row();
                    continue;
                }
                let mut chosen: Option<String> = None;
                egui::ComboBox::from_id_salt(("button", i))
                    .selected_text(self.label(&self.prof.buttons[i].text))
                    .width(220.0)
                    .height(420.0)
                    .show_ui(ui, |ui| {
                        let current = self.prof.buttons[i].text.clone();
                        let mut group = |ui: &mut egui::Ui, title: &str, ids: Vec<&str>| {
                            ui.label(RichText::new(title).small().weak());
                            for id in ids {
                                if ui.selectable_label(current == id, self.label(id)).clicked() {
                                    chosen = Some(id.to_string());
                                }
                            }
                            ui.separator();
                        };
                        group(ui, t.group_mouse, HW_FUNCTIONS.iter().map(|(n, _)| *n).collect());
                        group(ui, t.group_media, CONSUMER_USAGES.iter().map(|(n, _)| *n).collect());
                    });
                if let Some(id) = chosen {
                    if let Ok(a) = profile::parse_action(&id) {
                        self.prof.buttons[i] = a;
                    }
                }
                ui.end_row();
            }
        });
    }

    fn other_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(self.t.fire);
            let mut units = self.prof.fire_ms / 8;
            if ui.add(egui::Slider::new(&mut units, 1..=255).custom_formatter(|v, _| format!("{} ms", v as u32 * 8))).changed() {
                self.prof.fire_ms = units * 8;
            }
        });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.t;
        let dirty = self.prof != self.saved;
        let valid = self.prof.validate();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let (color, text) = if self.mouse.is_empty() {
                (Color32::from_rgb(220, 60, 60), t.mouse_missing)
            } else {
                (Color32::from_rgb(60, 170, 90), t.mouse_found)
            };
            status_dot(ui, color);
            ui.colored_label(color, text);
        });
        ui.horizontal(|ui| {
            match (&self.status, &valid) {
                (_, Err(e)) => ui.colored_label(Color32::from_rgb(220, 60, 60), format!("{}: {e:#}", t.invalid)),
                (Status::Err(e), _) => ui.colored_label(Color32::from_rgb(220, 60, 60), e),
                _ if dirty => ui.label(RichText::new(t.unsaved).italics()),
                (Status::Ok(m), _) => ui.colored_label(Color32::from_rgb(60, 170, 90), m),
                (Status::None, _) => ui.label(""),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(valid.is_ok() && !self.mouse.is_empty(), egui::Button::new(RichText::new(t.apply).strong())).clicked() {
                    self.apply();
                }
                if ui.add_enabled(dirty, egui::Button::new(t.revert)).clicked() {
                    self.prof = self.saved.clone();
                    self.status = Status::None;
                }
                if ui.button(t.defaults).clicked() {
                    self.prof = Profile::default();
                }
            });
        });
        ui.add_space(6.0);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        ctx.request_repaint_after(Duration::from_secs(2));
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let t = self.t;
                for (tab, name) in [(Tab::Dpi, t.tab_dpi), (Tab::Led, t.tab_led), (Tab::Buttons, t.tab_buttons), (Tab::Other, t.tab_other)] {
                    if ui.selectable_value(&mut self.tab, tab, RichText::new(name).size(15.0)).clicked() {
                        // The tab strip was already drawn with the old selection.
                        ctx.request_repaint();
                    }
                }
            });
            ui.add_space(6.0);
        });
        egui::TopBottomPanel::bottom("bar").show(ctx, |ui| self.bottom_bar(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                match self.tab {
                    Tab::Dpi => self.dpi_tab(ui),
                    Tab::Led => self.led_tab(ui),
                    Tab::Buttons => self.buttons_tab(ui),
                    Tab::Other => self.other_tab(ui),
                }
            });
        });
    }
}

/// A small filled circle; drawn rather than typed because the bundled fonts lack "●".
fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

/// Prints log records to stderr when IMG355_LOG is set (error, warn, info, debug or trace),
/// so graphics driver problems can be diagnosed without a debugger.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

fn init_logging() {
    let Ok(level) = std::env::var("IMG355_LOG") else { return };
    let level = level.parse().unwrap_or(log::LevelFilter::Debug);
    if log::set_logger(&StderrLogger).is_ok() {
        log::set_max_level(level);
    }
}

/// Re-executes this program with Mesa's software OpenGL; returns only if that is not possible.
fn restart_with_software_gl() {
    if std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    log::warn!("only OpenGL ES 2.0 is available; restarting with software rendering");
    let err = Command::new(exe).args(std::env::args_os().skip(1)).env("LIBGL_ALWAYS_SOFTWARE", "1").exec();
    log::warn!("could not restart with software rendering: {err}");
}

fn main() -> eframe::Result {
    init_logging();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(if i18n::turkish() { i18n::TR.title } else { i18n::EN.title })
            .with_app_id("img355ctl-gui")
            .with_inner_size([600.0, 470.0])
            .with_min_inner_size([540.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "img355ctl-gui",
        options,
        Box::new(|cc| {
            if let Some(gl) = &cc.gl {
                use eframe::glow::HasContext;
                let v = gl.version();
                log::debug!("GL {}.{} embedded={} {}", v.major, v.minor, v.is_embedded, v.vendor_info);
                // Old GPUs (e.g. Intel Ironlake) only offer OpenGL ES 2.0 here, where egui draws a
                // black window. Mesa's software renderer works fine for a small settings window.
                if v.is_embedded && v.major < 3 {
                    restart_with_software_gl();
                }
            }
            Ok(Box::new(App::new()))
        }),
    )
}
