//! What the settings window shows, and what changing a row does to the config.
//!
//! Kept free of Wayland and of drawing: every row is a [`Field`] with a getter
//! and a setter over [`Config`], so the list that is drawn, the list that is
//! clicked and the list that is written are the same list.

use std::path::PathBuf;

use spectre_config::{Config, PanelPosition, Profile, WallpaperMode, WorkspaceTransition};
use spectre_theme::PatternKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Resolution,
    OutputScale,
    Profile,
    Wallpaper,
    WallpaperMode,
    CornerRadius,
    TitlebarHeight,
    BorderWidth,

    PatternKind,
    PatternAnimated,
    PatternSpeed,
    ColorCycle,
    ColorSpeed,
    PatternIntensity,

    Transition,
    Blur,
    Shadows,
    RoundedCorners,
    WindowAnimations,
    AnimationSpeed,

    PanelEnabled,
    PanelPosition,
    PanelFloating,
    PanelOpacity,
}

/// How a row is drawn and what changing it means.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    Toggle(bool),
    /// One of a fixed list, cycled left and right.
    Choice { index: usize, options: Vec<String> },
    /// A 0..1 knob, shown as a bar.
    Slider { value: f32, label: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub field: Field,
    pub label: &'static str,
    pub help: &'static str,
    pub control: Control,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

pub struct Settings {
    pub config: Config,
    /// Wallpapers found on this machine, with "None" as the first choice.
    pub wallpapers: Vec<Option<PathBuf>>,
    /// Resolutions the display reports, with "Auto" first. Empty until the
    /// compositor has answered, which is why "Auto" is always a valid choice.
    pub resolutions: Vec<String>,
}

impl Settings {
    pub fn new(config: Config) -> Self {
        let mut wallpapers: Vec<Option<PathBuf>> = vec![None];
        wallpapers.extend(spectre_config::desktop::find_wallpapers().into_iter().map(Some));
        // A wallpaper set by hand may live outside the search path.
        if let Some(current) = config.desktop.wallpaper.clone() {
            if !wallpapers.iter().any(|w| w.as_ref() == Some(&current)) {
                wallpapers.push(Some(current));
            }
        }
        Self { config, wallpapers, resolutions: vec![AUTO.to_owned()] }
    }

    /// The ends of a slider field's range, in the field's own units.
    /// `None` for anything that is not a slider.
    pub fn slider_range(&self, field: Field) -> Option<(f32, f32)> {
        match field {
            Field::OutputScale => Some((MIN_SCALE as f32, MAX_SCALE as f32)),

            Field::CornerRadius => Some((0.0, MAX_RADIUS as f32)),
            Field::TitlebarHeight => Some((0.0, MAX_TITLEBAR as f32)),
            Field::BorderWidth => Some((0.0, MAX_BORDER as f32)),

            Field::PatternSpeed => Some((0.0, 1.0)),
            Field::ColorSpeed => Some((0.0, 1.0)),
            Field::PatternIntensity => Some((0.0, 1.0)),

            Field::AnimationSpeed => Some((0.0, MAX_SPEED)),

            Field::PanelOpacity => Some((0.0, 1.0)),

            _ => None,
        }
    }
    /// Set a slider field straight to `value`, given as 0..1 of its range.
    ///
    /// Assigning rather than stepping is what makes dragging one smooth; the
    /// range still comes from one place, so a slider and the arrow keys cannot
    /// disagree about where the ends are.
    pub fn set_slider(&mut self, field: Field, value: f32) -> bool {
        let Some((min, max)) = self.slider_range(field) else {
            return false;
        };
        let before = self.config.clone();
        // `f32::clamp` hands NaN straight back, and a slider whose control is
        // zero pixels wide produces one; treat it as the low end.
        let t = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
        let scaled = min + t * (max - min);
        let pixels = |v: f32| v.round().clamp(0.0, u32::MAX as f32) as u32;

        match field {
            // Two decimals: finer is below what the output can be driven at.
            Field::OutputScale => {
                self.config.display.scale = ((scaled as f64) * 100.0).round() / 100.0
            }
            Field::CornerRadius => self.config.theme.metrics.corner_radius = pixels(scaled),
            Field::TitlebarHeight => self.config.theme.metrics.titlebar_height = pixels(scaled),
            Field::BorderWidth => self.config.theme.metrics.border_width = pixels(scaled),

            Field::PatternSpeed => self.set_patterns(|p| p.speed = scaled),
            Field::ColorSpeed => self.set_patterns(|p| p.color_speed = scaled),
            Field::PatternIntensity => self.set_patterns(|p| p.intensity = scaled),

            Field::AnimationSpeed => {
                self.custom();
                self.config.effects.animation_speed = scaled.max(0.1);
            }
            Field::PanelOpacity => self.config.panel.opacity = scaled.max(0.1),

            // slider_range answered, so every slider is covered above.
            _ => return false,
        }
        before != self.config
    }

    /// Take the modes the compositor reported for its first output.
    pub fn set_modes(&mut self, modes: &[spectre_ipc::Mode]) {
        let mut list = vec![AUTO.to_owned()];
        list.extend(modes.iter().map(|m| m.label()));
        // A resolution written by hand stays selectable even if the display
        // stops offering it, so the row never shows something that is not there.
        let current = self.config.display.resolution.clone();
        if !current.eq_ignore_ascii_case(AUTO) && !list.contains(&current) {
            list.push(current);
        }
        self.resolutions = list;
    }

    pub fn sections(&self) -> Vec<Section> {
        vec![
            Section {
                title: "Display",
                rows: vec![
                    self.row(Field::Resolution, "Resolution", "Mode the output is driven at"),
                    self.row(Field::OutputScale, "Scale", "Logical pixels per device pixel"),
                ],
            },
            Section {
                title: "Appearance",
                rows: vec![
                    self.row(Field::Profile, "Profile", "Preset over every effect"),
                    self.row(Field::Wallpaper, "Wallpaper", "Image behind the desktop"),
                    self.row(Field::WallpaperMode, "Wallpaper fit", "How it covers the screen"),
                    self.row(Field::CornerRadius, "Corner radius", "Roundness of windows"),
                    self.row(Field::TitlebarHeight, "Title bar height", "Server-side decorations"),
                    self.row(Field::BorderWidth, "Border width", "Hairline around a window"),
                ],
            },
            Section {
                title: "Spectre Pattern",
                rows: vec![
                    self.row(Field::PatternKind, "Pattern", "Contour texture"),
                    self.row(Field::PatternAnimated, "Move the lines", "Scroll the contour field"),
                    self.row(Field::PatternSpeed, "Line speed", "How fast the field scrolls"),
                    self.row(Field::ColorCycle, "Animate the RGB", "Colours travel, lines stay put"),
                    self.row(Field::ColorSpeed, "RGB speed", "How fast the colours travel"),
                    self.row(Field::PatternIntensity, "Intensity", "How strong the lines read"),
                ],
            },
            Section {
                title: "Effects",
                rows: vec![
                    self.row(Field::Transition, "Workspace switch", "Animation between desktops"),
                    self.row(Field::WindowAnimations, "Window animations", "Open, close and move"),
                    self.row(Field::AnimationSpeed, "Animation speed", "Higher is faster"),
                    self.row(Field::RoundedCorners, "Rounded corners", "Clip windows to a radius"),
                    self.row(Field::Shadows, "Shadows", "Drop shadow under windows"),
                    self.row(Field::Blur, "Blur", "Costs a full offscreen pass"),
                ],
            },
            Section {
                title: "Panel",
                rows: vec![
                    self.row(Field::PanelEnabled, "Panel", "The Spectre taskbar"),
                    self.row(Field::PanelPosition, "Position", "Screen edge it sits on"),
                    self.row(Field::PanelFloating, "Floating", "Leave a margin around it"),
                    self.row(Field::PanelOpacity, "Opacity", "Background transparency"),
                ],
            },
        ]
    }

    fn row(&self, field: Field, label: &'static str, help: &'static str) -> Row {
        Row { field, label, help, control: self.control(field) }
    }

    fn control(&self, field: Field) -> Control {
        let cfg = &self.config;
        let pattern = &cfg.theme.window_pattern;
        match field {
            Field::Resolution => choice(self.resolutions.clone(), self.resolution_index()),
            Field::OutputScale => Control::Slider {
                value: ((cfg.display.output_scale() - MIN_SCALE) / (MAX_SCALE - MIN_SCALE)) as f32,
                label: format!("{:.2}x", cfg.display.output_scale()),
            },
            Field::Profile => choice(
                Profile::ALL.iter().map(|p| p.label().to_owned()).collect(),
                Profile::ALL.iter().position(|p| *p == cfg.general.profile).unwrap_or(0),
            ),
            Field::Wallpaper => choice(
                self.wallpapers.iter().map(|w| wallpaper_label(w.as_deref())).collect(),
                self.wallpaper_index(),
            ),
            Field::WallpaperMode => choice(
                MODES.iter().map(|(_, name)| (*name).to_owned()).collect(),
                MODES.iter().position(|(m, _)| *m == cfg.desktop.wallpaper_mode).unwrap_or(0),
            ),
            Field::CornerRadius => pixels(cfg.theme.metrics.corner_radius, MAX_RADIUS),
            Field::TitlebarHeight => pixels(cfg.theme.metrics.titlebar_height, MAX_TITLEBAR),
            Field::BorderWidth => pixels(cfg.theme.metrics.border_width, MAX_BORDER),

            Field::PatternKind => choice(
                KINDS.iter().map(|(_, name)| (*name).to_owned()).collect(),
                KINDS.iter().position(|(k, _)| *k == pattern.kind).unwrap_or(0),
            ),
            Field::PatternAnimated => Control::Toggle(pattern.animated),
            Field::PatternSpeed => percent(pattern.speed),
            Field::ColorCycle => Control::Toggle(pattern.color_cycle),
            Field::ColorSpeed => percent(pattern.color_speed),
            Field::PatternIntensity => percent(pattern.intensity),

            Field::Transition => choice(
                TRANSITIONS.iter().map(|(_, name)| (*name).to_owned()).collect(),
                TRANSITIONS
                    .iter()
                    .position(|(t, _)| *t == cfg.effects.workspace_transition)
                    .unwrap_or(0),
            ),
            Field::Blur => Control::Toggle(cfg.effects.blur),
            Field::Shadows => Control::Toggle(cfg.effects.shadows),
            Field::RoundedCorners => Control::Toggle(cfg.effects.rounded_corners),
            Field::WindowAnimations => Control::Toggle(cfg.effects.window_animations),
            Field::AnimationSpeed => percent(cfg.effects.animation_speed / MAX_SPEED),

            Field::PanelEnabled => Control::Toggle(cfg.panel.enabled),
            Field::PanelPosition => choice(
                POSITIONS.iter().map(|(_, name)| (*name).to_owned()).collect(),
                POSITIONS.iter().position(|(p, _)| *p == cfg.panel.position).unwrap_or(0),
            ),
            Field::PanelFloating => Control::Toggle(cfg.panel.floating),
            Field::PanelOpacity => percent(cfg.panel.opacity),
        }
    }

    fn resolution_index(&self) -> usize {
        self.resolutions
            .iter()
            .position(|r| r.eq_ignore_ascii_case(&self.config.display.resolution))
            .unwrap_or(0)
    }

    fn wallpaper_index(&self) -> usize {
        self.wallpapers
            .iter()
            .position(|w| w.as_deref() == self.config.desktop.wallpaper.as_deref())
            .unwrap_or(0)
    }

    /// Step a row: `delta` is -1 or 1, and a toggle flips either way.
    ///
    /// Returns true when something actually changed, so the caller only writes
    /// the file when there is something to write.
    pub fn step(&mut self, field: Field, delta: i32) -> bool {
        let before = self.config.clone();
        match field {
            Field::Resolution => {
                let index = wrap(self.resolutions.len(), self.resolution_index(), delta);
                self.config.display.resolution = self.resolutions[index].clone();
            }
            Field::OutputScale => {
                // Quarter steps: anything finer is invisible and makes the row
                // impossible to land on with an arrow key.
                let scale = (self.config.display.output_scale() + delta as f64 * 0.25)
                    .clamp(MIN_SCALE, MAX_SCALE);
                self.config.display.scale = (scale * 100.0).round() / 100.0;
            }
            Field::Profile => {
                let index = cycle(&Profile::ALL, self.control_index(field), delta);
                self.config.general.profile = Profile::ALL[index];
                // Anything but Custom overwrites the effect switches, and the
                // user has to see that immediately or the rows below lie.
                if let Some(effects) = self.config.general.profile.effects() {
                    self.config.effects = effects;
                }
            }
            Field::Wallpaper => {
                let index = wrap(self.wallpapers.len(), self.wallpaper_index(), delta);
                self.config.desktop.wallpaper = self.wallpapers[index].clone();
            }
            Field::WallpaperMode => {
                let index = cycle(&MODES, self.control_index(field), delta);
                self.config.desktop.wallpaper_mode = MODES[index].0;
            }
            Field::CornerRadius => {
                self.config.theme.metrics.corner_radius =
                    nudge(self.config.theme.metrics.corner_radius, delta, MAX_RADIUS)
            }
            Field::TitlebarHeight => {
                self.config.theme.metrics.titlebar_height =
                    nudge(self.config.theme.metrics.titlebar_height, delta * 2, MAX_TITLEBAR)
            }
            Field::BorderWidth => {
                self.config.theme.metrics.border_width =
                    nudge(self.config.theme.metrics.border_width, delta, MAX_BORDER)
            }

            Field::PatternKind => {
                let index = cycle(&KINDS, self.control_index(field), delta);
                self.set_patterns(|p| p.kind = KINDS[index].0);
            }
            Field::PatternAnimated => {
                let on = !self.config.theme.window_pattern.animated;
                self.set_patterns(|p| p.animated = on);
            }
            Field::PatternSpeed => {
                let value = step_unit(self.config.theme.window_pattern.speed, delta);
                self.set_patterns(|p| p.speed = value);
            }
            Field::ColorCycle => {
                let on = !self.config.theme.window_pattern.color_cycle;
                self.set_patterns(|p| p.color_cycle = on);
            }
            Field::ColorSpeed => {
                let value = step_unit(self.config.theme.window_pattern.color_speed, delta);
                self.set_patterns(|p| p.color_speed = value);
            }
            Field::PatternIntensity => {
                let value = step_unit(self.config.theme.window_pattern.intensity, delta);
                self.set_patterns(|p| p.intensity = value);
            }

            Field::Transition => {
                let index = cycle(&TRANSITIONS, self.control_index(field), delta);
                self.custom();
                self.config.effects.workspace_transition = TRANSITIONS[index].0;
            }
            Field::Blur => {
                self.custom();
                self.config.effects.blur = !self.config.effects.blur;
            }
            Field::Shadows => {
                self.custom();
                self.config.effects.shadows = !self.config.effects.shadows;
            }
            Field::RoundedCorners => {
                self.custom();
                self.config.effects.rounded_corners = !self.config.effects.rounded_corners;
            }
            Field::WindowAnimations => {
                self.custom();
                self.config.effects.window_animations = !self.config.effects.window_animations;
            }
            Field::AnimationSpeed => {
                self.custom();
                let value = step_unit(self.config.effects.animation_speed / MAX_SPEED, delta);
                self.config.effects.animation_speed = (value * MAX_SPEED).max(0.1);
            }

            Field::PanelEnabled => self.config.panel.enabled = !self.config.panel.enabled,
            Field::PanelPosition => {
                let index = cycle(&POSITIONS, self.control_index(field), delta);
                self.config.panel.position = POSITIONS[index].0;
            }
            Field::PanelFloating => self.config.panel.floating = !self.config.panel.floating,
            Field::PanelOpacity => {
                self.config.panel.opacity = step_unit(self.config.panel.opacity, delta).max(0.1)
            }
        }
        before != self.config
    }

    /// Editing an effect switch only means something under the Custom profile,
    /// so touching one moves the profile there rather than silently doing
    /// nothing on the next reload.
    fn custom(&mut self) {
        self.config.general.profile = Profile::Custom;
    }

    /// Patterns are edited together: one Spectre Pattern, drawn in three places.
    fn set_patterns(&mut self, edit: impl Fn(&mut spectre_theme::Pattern)) {
        edit(&mut self.config.theme.window_pattern);
        edit(&mut self.config.theme.panel_pattern);
        edit(&mut self.config.theme.desktop_pattern.0);
    }

    fn control_index(&self, field: Field) -> usize {
        match self.control(field) {
            Control::Choice { index, .. } => index,
            _ => 0,
        }
    }

    /// Write the config to the file the session is running from.
    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Config::active_path().ok_or("no configuration directory")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = self.config.to_toml().map_err(|e| format!("{e}"))?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

/// The string that means "whatever the display prefers".
pub const AUTO: &str = "Auto";
const MIN_SCALE: f64 = 0.5;
const MAX_SCALE: f64 = 3.0;

const MAX_RADIUS: u32 = 24;
const MAX_TITLEBAR: u32 = 48;
const MAX_BORDER: u32 = 6;
/// The top of the animation speed slider.
const MAX_SPEED: f32 = 2.0;

const MODES: [(WallpaperMode, &str); 4] = [
    (WallpaperMode::Fill, "Fill"),
    (WallpaperMode::Fit, "Fit"),
    (WallpaperMode::Stretch, "Stretch"),
    (WallpaperMode::Center, "Centre"),
];

const KINDS: [(PatternKind, &str); 3] = [
    (PatternKind::Topographic, "Topographic"),
    (PatternKind::Grid, "Grid"),
    (PatternKind::None, "None"),
];

const TRANSITIONS: [(WorkspaceTransition, &str); 6] = [
    (WorkspaceTransition::None, "None"),
    (WorkspaceTransition::Fade, "Fade"),
    (WorkspaceTransition::Slide, "Slide"),
    (WorkspaceTransition::Depth, "Depth"),
    (WorkspaceTransition::Cube, "Cube"),
    (WorkspaceTransition::Coverflow, "Coverflow"),
];

const POSITIONS: [(PanelPosition, &str); 4] = [
    (PanelPosition::Bottom, "Bottom"),
    (PanelPosition::Top, "Top"),
    (PanelPosition::Left, "Left"),
    (PanelPosition::Right, "Right"),
];

fn choice(options: Vec<String>, index: usize) -> Control {
    Control::Choice { index, options }
}

fn percent(value: f32) -> Control {
    let value = value.clamp(0.0, 1.0);
    Control::Slider { value, label: format!("{}%", (value * 100.0).round() as i32) }
}

fn pixels(value: u32, max: u32) -> Control {
    Control::Slider { value: value as f32 / max as f32, label: format!("{value} px") }
}

fn wallpaper_label(path: Option<&std::path::Path>) -> String {
    match path {
        None => String::from("None"),
        Some(path) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    }
}

fn cycle<T>(options: &[T], index: usize, delta: i32) -> usize {
    wrap(options.len(), index, delta)
}

fn wrap(len: usize, index: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (index as i64 + delta as i64).rem_euclid(len as i64) as usize
}

fn step_unit(value: f32, delta: i32) -> f32 {
    (value + delta as f32 * 0.05).clamp(0.0, 1.0)
}

fn nudge(value: u32, delta: i32, max: u32) -> u32 {
    (value as i64 + delta as i64).clamp(0, max as i64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            config: Config::default(),
            wallpapers: vec![None],
            resolutions: vec![AUTO.to_owned()],
        }
    }

    fn mode(width: i32, height: i32, refresh: u32) -> spectre_ipc::Mode {
        spectre_ipc::Mode { width, height, refresh }
    }

    #[test]
    fn every_section_has_rows_and_every_row_a_control() {
        let s = settings();
        let sections = s.sections();
        assert!(sections.len() >= 4);
        for section in sections {
            assert!(!section.rows.is_empty(), "{} is empty", section.title);
        }
    }

    #[test]
    fn toggling_a_row_flips_it_back_and_forth() {
        let mut s = settings();
        let before = s.config.theme.window_pattern.color_cycle;
        assert!(s.step(Field::ColorCycle, 1));
        assert_ne!(s.config.theme.window_pattern.color_cycle, before);
        assert!(s.step(Field::ColorCycle, 1));
        assert_eq!(s.config.theme.window_pattern.color_cycle, before);
    }

    #[test]
    fn the_pattern_is_edited_everywhere_it_is_drawn() {
        let mut s = settings();
        s.step(Field::PatternIntensity, 1);
        let window = s.config.theme.window_pattern.intensity;
        assert_eq!(s.config.theme.panel_pattern.intensity, window);
        assert_eq!(s.config.theme.desktop_pattern.0.intensity, window);
    }

    #[test]
    fn choices_wrap_in_both_directions() {
        let mut s = settings();
        s.config.general.profile = Profile::ALL[0];
        s.step(Field::Profile, -1);
        assert_eq!(s.config.general.profile, Profile::ALL[Profile::ALL.len() - 1]);
        s.step(Field::Profile, 1);
        assert_eq!(s.config.general.profile, Profile::ALL[0]);
    }

    #[test]
    fn sliders_stay_inside_their_range() {
        let mut s = settings();
        for _ in 0..40 {
            s.step(Field::PatternIntensity, 1);
        }
        assert_eq!(s.config.theme.window_pattern.intensity, 1.0);
        for _ in 0..40 {
            s.step(Field::PatternIntensity, -1);
        }
        assert_eq!(s.config.theme.window_pattern.intensity, 0.0);
    }

    #[test]
    fn editing_an_effect_moves_the_profile_to_custom() {
        let mut s = settings();
        s.config.general.profile = Profile::Balanced;
        s.step(Field::Blur, 1);
        assert_eq!(s.config.general.profile, Profile::Custom);
    }

    #[test]
    fn picking_a_profile_updates_the_effect_rows_it_prescribes() {
        let mut s = settings();
        s.config.general.profile = Profile::Custom;
        s.config.effects.blur = true;
        while s.config.general.profile != Profile::Performance {
            s.step(Field::Profile, 1);
        }
        assert!(!s.config.effects.blur, "Performance must win over the old switch");
    }

    #[test]
    fn a_step_that_changes_nothing_reports_nothing() {
        let mut s = settings();
        s.config.theme.window_pattern.intensity = 1.0;
        s.config.theme.panel_pattern.intensity = 1.0;
        s.config.theme.desktop_pattern.0.intensity = 1.0;
        assert!(!s.step(Field::PatternIntensity, 1), "already at the top");
    }

    #[test]
    fn metrics_never_go_negative() {
        let mut s = settings();
        for _ in 0..50 {
            s.step(Field::BorderWidth, -1);
        }
        assert_eq!(s.config.theme.metrics.border_width, 0);
    }

    #[test]
    fn the_resolution_list_always_offers_auto_first() {
        let mut s = settings();
        s.set_modes(&[mode(1920, 1080, 60), mode(1280, 720, 60)]);
        assert_eq!(s.resolutions[0], AUTO);
        assert!(s.resolutions.contains(&String::from("1920x1080@60")));
    }

    #[test]
    fn a_resolution_the_display_no_longer_offers_stays_selectable() {
        let mut s = settings();
        s.config.display.resolution = String::from("3840x2160@60");
        s.set_modes(&[mode(1920, 1080, 60)]);
        assert!(s.resolutions.contains(&String::from("3840x2160@60")));
        assert!(s.resolution_index() > 0);
    }

    #[test]
    fn stepping_the_resolution_writes_something_the_config_can_parse() {
        let mut s = settings();
        s.set_modes(&[mode(1920, 1080, 60)]);
        s.step(Field::Resolution, 1);
        assert_eq!(s.config.display.resolution, "1920x1080@60");
        let wanted = s.config.display.wanted_mode().unwrap();
        assert_eq!((wanted.width, wanted.height, wanted.refresh), (1920, 1080, Some(60)));
        s.step(Field::Resolution, -1);
        assert!(s.config.display.wanted_mode().is_none(), "back to Auto");
    }

    #[test]
    fn the_scale_moves_in_quarter_steps_and_stays_drivable() {
        let mut s = settings();
        s.step(Field::OutputScale, 1);
        assert_eq!(s.config.display.scale, 1.25);
        for _ in 0..40 {
            s.step(Field::OutputScale, -1);
        }
        assert_eq!(s.config.display.scale, 0.5);
    }

    #[test]
    fn a_slider_can_be_set_straight_to_a_value() {
        let mut s = settings();
        assert!(s.set_slider(Field::PatternIntensity, 0.5));
        assert_eq!(s.config.theme.window_pattern.intensity, 0.5);
    }

    #[test]
    fn a_value_outside_the_range_is_clamped_rather_than_wrapped() {
        let mut s = settings();
        s.set_slider(Field::PatternIntensity, 4.0);
        assert_eq!(s.config.theme.window_pattern.intensity, 1.0);
        s.set_slider(Field::PatternIntensity, -4.0);
        assert_eq!(s.config.theme.window_pattern.intensity, 0.0);
        s.set_slider(Field::PatternIntensity, f32::NAN);
        assert!(s.config.theme.window_pattern.intensity.is_finite(), "NaN must not get through");
    }

    #[test]
    fn setting_a_slider_that_is_not_one_changes_nothing() {
        let mut s = settings();
        let before = s.config.clone();
        assert!(!s.set_slider(Field::Blur, 1.0));
        assert!(!s.set_slider(Field::Profile, 1.0));
        assert_eq!(s.config, before);
    }

    #[test]
    fn setting_the_same_value_twice_reports_a_change_only_once() {
        let mut s = settings();
        assert!(s.set_slider(Field::PanelOpacity, 0.5));
        assert!(!s.set_slider(Field::PanelOpacity, 0.5));
    }

    #[test]
    fn a_slider_in_pixels_lands_on_whole_pixels() {
        let mut s = settings();
        s.set_slider(Field::CornerRadius, 0.5);
        let (_, max) = s.slider_range(Field::CornerRadius).unwrap();
        assert_eq!(s.config.theme.metrics.corner_radius, (max * 0.5).round() as u32);
    }

    #[test]
    fn every_slider_has_a_range_and_nothing_else_does() {
        let s = settings();
        for section in s.sections() {
            for row in section.rows {
                let is_slider = matches!(row.control, Control::Slider { .. });
                assert_eq!(
                    s.slider_range(row.field).is_some(),
                    is_slider,
                    "{} disagrees about being a slider",
                    row.label
                );
            }
        }
    }

    #[test]
    fn dragging_a_slider_reaches_both_ends() {
        let mut s = settings();
        s.set_slider(Field::PatternSpeed, 0.0);
        assert_eq!(s.config.theme.window_pattern.speed, 0.0);
        s.set_slider(Field::PatternSpeed, 1.0);
        assert_eq!(s.config.theme.window_pattern.speed, 1.0);
    }

    #[test]
    fn the_wallpaper_list_always_offers_none() {
        let s = Settings::new(Config::default());
        assert_eq!(s.wallpapers.first(), Some(&None));
    }

    #[test]
    fn a_hand_written_wallpaper_stays_selectable() {
        let mut config = Config::default();
        config.desktop.wallpaper = Some(PathBuf::from("/tmp/mine.png"));
        let s = Settings::new(config);
        assert!(s.wallpapers.contains(&Some(PathBuf::from("/tmp/mine.png"))));
        assert!(s.wallpaper_index() > 0);
    }
}
