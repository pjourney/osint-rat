//! Cockpit colour roles. Widgets ask for a role (`theme.ai`), never a raw colour.
//!
//! Every `Color::` constructor in the crate lives in this file. CI greps the rest
//! of `src/` and fails if one leaks out.

use ratatui::style::{Color, Modifier, Style};

use crate::config::Config;

use super::glyphs::Glyphs;

/// Named colour roles. Heat is four gradient stops; everything else is a single colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Bg,
    Panel,
    Fg,
    Muted,
    Rule,
    Accent,
    Ok,
    Warn,
    Err,
    Col,
    Ext,
    Anl,
    Ai,
    Heat0,
    Heat1,
    Heat2,
    Heat3,
}

impl Role {
    pub const ALL: [Role; 17] = [
        Role::Bg,
        Role::Panel,
        Role::Fg,
        Role::Muted,
        Role::Rule,
        Role::Accent,
        Role::Ok,
        Role::Warn,
        Role::Err,
        Role::Col,
        Role::Ext,
        Role::Anl,
        Role::Ai,
        Role::Heat0,
        Role::Heat1,
        Role::Heat2,
        Role::Heat3,
    ];

    /// Roles that carry body or label text and must meet WCAG AA (4.5:1).
    pub const TEXT: [Role; 10] = [
        Role::Fg,
        Role::Muted,
        Role::Accent,
        Role::Ok,
        Role::Warn,
        Role::Err,
        Role::Col,
        Role::Ext,
        Role::Anl,
        Role::Ai,
    ];

    /// Semantic pairs checked under colour-blind simulation.
    pub const SEMANTIC: [Role; 7] = [
        Role::Ok,
        Role::Warn,
        Role::Err,
        Role::Col,
        Role::Ext,
        Role::Ai,
        Role::Accent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Bg => "bg",
            Role::Panel => "panel",
            Role::Fg => "fg",
            Role::Muted => "muted",
            Role::Rule => "rule",
            Role::Accent => "accent",
            Role::Ok => "ok",
            Role::Warn => "warn",
            Role::Err => "err",
            Role::Col => "col",
            Role::Ext => "ext",
            Role::Anl => "anl",
            Role::Ai => "ai",
            Role::Heat0 => "heat0",
            Role::Heat1 => "heat1",
            Role::Heat2 => "heat2",
            Role::Heat3 => "heat3",
        }
    }
}

/// Built-in cockpit themes. `auto` is a config value, not a palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeName {
    Dark,
    Light,
    Mono,
    Phosphor,
    Amber,
}

impl ThemeName {
    pub const ALL: [ThemeName; 5] = [
        ThemeName::Dark,
        ThemeName::Light,
        ThemeName::Mono,
        ThemeName::Phosphor,
        ThemeName::Amber,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" | "cockpit-dark" | "cockpit_dark" => Some(Self::Dark),
            "light" | "cockpit-light" | "cockpit_light" => Some(Self::Light),
            "mono" | "monochrome" => Some(Self::Mono),
            "phosphor" => Some(Self::Phosphor),
            "amber" => Some(Self::Amber),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Mono => "mono",
            Self::Phosphor => "phosphor",
            Self::Amber => "amber",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Cockpit Dark",
            Self::Light => "Cockpit Light",
            Self::Mono => "Mono",
            Self::Phosphor => "Phosphor",
            Self::Amber => "Amber",
        }
    }

    pub fn paints_background(self) -> bool {
        matches!(self, Self::Phosphor | Self::Amber)
    }

    pub fn is_signature(self) -> bool {
        matches!(self, Self::Phosphor | Self::Amber)
    }
}

/// Terminal colour depth after detection and `ui.color` override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Ansi16,
    None,
}

impl ColorDepth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrueColor => "truecolor",
            Self::Ansi256 => "256",
            Self::Ansi16 => "16",
            Self::None => "none",
        }
    }
}

/// `ui.color` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    TrueColor,
    Ansi256,
    Ansi16,
    None,
}

impl ColorMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "truecolor" | "truecolour" | "24bit" => Some(Self::TrueColor),
            "256" | "256color" | "256colour" => Some(Self::Ansi256),
            "16" | "16color" | "16colour" | "ansi" => Some(Self::Ansi16),
            "none" | "off" | "mono" => Some(Self::None),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::TrueColor => "truecolor",
            Self::Ansi256 => "256",
            Self::Ansi16 => "16",
            Self::None => "none",
        }
    }
}

/// Environment used for colour detection. Tests pass a struct; the TUI reads the process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalEnv {
    pub colorterm: Option<String>,
    pub term: Option<String>,
    pub no_color: bool,
    pub colorfgbg: Option<String>,
}

impl TerminalEnv {
    pub fn from_process() -> Self {
        Self {
            colorterm: std::env::var("COLORTERM").ok(),
            term: std::env::var("TERM").ok(),
            no_color: std::env::var_os("NO_COLOR").is_some(),
            colorfgbg: std::env::var("COLORFGBG").ok(),
        }
    }
}

/// Result of walking COLORTERM, then TERM, then NO_COLOR, then `ui.color`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedDepth {
    pub depth: ColorDepth,
    pub reason: String,
}

/// sRGB triple. Stored for contrast, CVD, and the preview command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    pub fn from_hex(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xff) as u8,
            g: ((hex >> 8) & 0xff) as u8,
            b: (hex & 0xff) as u8,
        }
    }

    pub fn format_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Clone, Copy, Debug)]
struct RoleSpec {
    rgb: Option<Rgb>,
    index256: u8,
    ansi16: Color,
    modifiers: Modifier,
    ansi16_modifiers: Modifier,
}

#[derive(Clone, Copy, Debug)]
struct Palette {
    roles: [RoleSpec; 17],
}

/// Resolved styles for the active theme and colour depth.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: ThemeName,
    pub depth: ColorDepth,
    pub depth_reason: String,
    pub ascii: bool,
    pub paints_background: bool,
    pub degrade_hint: Option<&'static str>,
    pub glyphs: Glyphs,
    pub bg: Style,
    pub panel: Style,
    pub fg: Style,
    pub muted: Style,
    pub rule: Style,
    pub accent: Style,
    pub ok: Style,
    pub warn: Style,
    pub err: Style,
    pub col: Style,
    pub ext: Style,
    pub anl: Style,
    pub ai: Style,
    pub heat0: Style,
    pub heat1: Style,
    pub heat2: Style,
    pub heat3: Style,
    /// Bold body text (titles). Derived from `fg`.
    pub title: Style,
    /// Alias of `accent` for existing screens.
    pub header: Style,
    /// Alias of `accent` (focused pane rule).
    pub focused_border: Style,
    /// Alias of `rule` (unfocused pane rule).
    pub unfocused_border: Style,
    /// Selected row: `panel` background, or REVERSED on Mono / 16-colour Dark/Light.
    pub selected: Style,
    /// Alias of `fg`.
    pub status: Style,
    /// Alias of `err`.
    pub error: Style,
    /// Alias of `ok`.
    pub success: Style,
    /// Alias of `warn`.
    pub warning: Style,
    /// Alias of `ai`.
    pub agent: Style,
    pub badge_ai: Style,
    pub badge_col: Style,
    pub badge_anl: Style,
    pub badge_ext: Style,
    palette: Palette,
}

impl Theme {
    pub fn from_config(config: &Config) -> Self {
        Self::resolve(config, &TerminalEnv::from_process())
    }

    pub fn resolve(config: &Config, env: &TerminalEnv) -> Self {
        let color_mode = ColorMode::parse(&config.ui.color).unwrap_or(ColorMode::Auto);
        let detected = detect_color_depth(env, color_mode);
        let requested = resolve_theme_name(&config.ui.theme, env, detected.depth);
        Self::build(requested, detected, config.ui.ascii)
    }

    pub fn build(requested: ThemeName, detected: DetectedDepth, ascii: bool) -> Self {
        let mut degrade_hint = None;
        let mut name = requested;
        let mut depth = detected.depth;
        let mut reason = detected.reason;
        if detected.depth == ColorDepth::None {
            name = ThemeName::Mono;
            depth = ColorDepth::None;
        } else if requested.is_signature() && detected.depth == ColorDepth::Ansi16 {
            name = ThemeName::Mono;
            depth = ColorDepth::None;
            degrade_hint = Some(match requested {
                ThemeName::Phosphor => "phosphor needs 256 colours",
                ThemeName::Amber => "amber needs 256 colours",
                _ => "theme needs 256 colours",
            });
        } else if requested == ThemeName::Mono {
            depth = ColorDepth::None;
            reason = "mono theme".into();
        }
        let palette = palette_for(name);
        let glyphs = Glyphs::from_ascii_flag(ascii);
        let paints_background = name.paints_background() && depth != ColorDepth::None;
        let styles = role_styles(&palette, depth, paints_background);
        let selected = selected_style(&palette, depth, paints_background, styles[idx(Role::Fg)]);
        let fg = styles[idx(Role::Fg)];
        let accent = styles[idx(Role::Accent)];
        let rule = styles[idx(Role::Rule)];
        let ok = styles[idx(Role::Ok)];
        let warn = styles[idx(Role::Warn)];
        let err = styles[idx(Role::Err)];
        let col = styles[idx(Role::Col)];
        let ext = styles[idx(Role::Ext)];
        let anl = styles[idx(Role::Anl)];
        let ai = styles[idx(Role::Ai)];
        Self {
            name,
            depth,
            depth_reason: reason,
            ascii,
            paints_background,
            degrade_hint,
            glyphs,
            bg: styles[idx(Role::Bg)],
            panel: styles[idx(Role::Panel)],
            fg,
            muted: styles[idx(Role::Muted)],
            rule,
            accent,
            ok,
            warn,
            err,
            col,
            ext,
            anl,
            ai,
            heat0: styles[idx(Role::Heat0)],
            heat1: styles[idx(Role::Heat1)],
            heat2: styles[idx(Role::Heat2)],
            heat3: styles[idx(Role::Heat3)],
            title: fg.add_modifier(Modifier::BOLD),
            header: accent,
            focused_border: accent,
            unfocused_border: rule,
            selected,
            status: fg,
            error: err,
            success: ok,
            warning: warn,
            agent: ai,
            badge_ai: ai,
            badge_col: col,
            badge_anl: anl,
            badge_ext: ext,
            palette,
        }
    }

    pub fn style(&self, role: Role) -> Style {
        match role {
            Role::Bg => self.bg,
            Role::Panel => self.panel,
            Role::Fg => self.fg,
            Role::Muted => self.muted,
            Role::Rule => self.rule,
            Role::Accent => self.accent,
            Role::Ok => self.ok,
            Role::Warn => self.warn,
            Role::Err => self.err,
            Role::Col => self.col,
            Role::Ext => self.ext,
            Role::Anl => self.anl,
            Role::Ai => self.ai,
            Role::Heat0 => self.heat0,
            Role::Heat1 => self.heat1,
            Role::Heat2 => self.heat2,
            Role::Heat3 => self.heat3,
        }
    }

    pub fn spec(&self, role: Role) -> RolePreview {
        let spec = self.palette.roles[idx(role)];
        RolePreview {
            role,
            rgb: spec.rgb,
            index256: spec.index256,
            ansi16_name: ansi16_name(spec.ansi16),
        }
    }
}

/// One row of `os-int theme preview`.
#[derive(Clone, Copy, Debug)]
pub struct RolePreview {
    pub role: Role,
    pub rgb: Option<Rgb>,
    pub index256: u8,
    pub ansi16_name: &'static str,
}

pub fn detect_color_depth(env: &TerminalEnv, color_override: ColorMode) -> DetectedDepth {
    let colorterm = env
        .colorterm
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let term = env
        .term
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut depth;
    let mut reason;
    if let Some(value) = colorterm.filter(|value| is_truecolor_token(value)) {
        depth = ColorDepth::TrueColor;
        reason = format!("COLORTERM={value}");
    } else if let Some(value) = term.filter(|value| value.to_ascii_lowercase().contains("256color"))
    {
        depth = ColorDepth::Ansi256;
        reason = format!("TERM={value}");
    } else {
        depth = ColorDepth::Ansi16;
        reason = match term {
            Some(value) => format!("TERM={value}"),
            None => "TERM unset".into(),
        };
    }
    if env.no_color {
        depth = ColorDepth::None;
        reason = "NO_COLOR".into();
    }
    match color_override {
        ColorMode::Auto => {}
        ColorMode::TrueColor => {
            depth = ColorDepth::TrueColor;
            reason = "ui.color=truecolor".into();
        }
        ColorMode::Ansi256 => {
            depth = ColorDepth::Ansi256;
            reason = "ui.color=256".into();
        }
        ColorMode::Ansi16 => {
            depth = ColorDepth::Ansi16;
            reason = "ui.color=16".into();
        }
        ColorMode::None => {
            depth = ColorDepth::None;
            reason = "ui.color=none".into();
        }
    }
    DetectedDepth { depth, reason }
}

fn is_truecolor_token(value: &str) -> bool {
    value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
}

pub fn resolve_theme_name(configured: &str, env: &TerminalEnv, depth: ColorDepth) -> ThemeName {
    if depth == ColorDepth::None {
        return ThemeName::Mono;
    }
    let trimmed = configured.trim();
    if trimmed.eq_ignore_ascii_case("auto") || trimmed.is_empty() {
        return auto_theme_from_colorfgbg(env.colorfgbg.as_deref());
    }
    ThemeName::parse(trimmed).unwrap_or_else(|| auto_theme_from_colorfgbg(env.colorfgbg.as_deref()))
}

fn auto_theme_from_colorfgbg(colorfgbg: Option<&str>) -> ThemeName {
    let Some(value) = colorfgbg else {
        return ThemeName::Dark;
    };
    let Some(background) = value.split(';').nth(1) else {
        return ThemeName::Dark;
    };
    match background.trim().parse::<u8>() {
        Ok(7 | 9..=15) => ThemeName::Light,
        _ => ThemeName::Dark,
    }
}

fn idx(role: Role) -> usize {
    match role {
        Role::Bg => 0,
        Role::Panel => 1,
        Role::Fg => 2,
        Role::Muted => 3,
        Role::Rule => 4,
        Role::Accent => 5,
        Role::Ok => 6,
        Role::Warn => 7,
        Role::Err => 8,
        Role::Col => 9,
        Role::Ext => 10,
        Role::Anl => 11,
        Role::Ai => 12,
        Role::Heat0 => 13,
        Role::Heat1 => 14,
        Role::Heat2 => 15,
        Role::Heat3 => 16,
    }
}

fn spec(
    hex: u32,
    index256: u8,
    ansi16: Color,
    modifiers: Modifier,
    ansi16_modifiers: Modifier,
) -> RoleSpec {
    RoleSpec {
        rgb: Some(Rgb::from_hex(hex)),
        index256,
        ansi16,
        modifiers,
        ansi16_modifiers,
    }
}

fn palette_for(name: ThemeName) -> Palette {
    match name {
        ThemeName::Dark => dark_palette(),
        ThemeName::Light => light_palette(),
        ThemeName::Mono => mono_palette(),
        ThemeName::Phosphor => phosphor_palette(),
        ThemeName::Amber => amber_palette(),
    }
}

fn dark_palette() -> Palette {
    let none = Modifier::empty();
    Palette {
        roles: [
            spec(0x14171C, 234, Color::Black, none, none),
            spec(0x1B1F26, 235, Color::Black, none, none),
            spec(0xD8DEE6, 253, Color::White, none, none),
            spec(0x8B95A1, 246, Color::Gray, none, none),
            spec(0x2C323B, 236, Color::DarkGray, none, none),
            spec(0x4FB3D9, 74, Color::Cyan, none, none),
            spec(0x8FD694, 114, Color::Green, none, none),
            spec(0xE5B567, 179, Color::Yellow, none, none),
            spec(0xE0607E, 168, Color::LightRed, none, none),
            spec(0x56C2A6, 73, Color::Cyan, none, none),
            spec(0xA9B1D6, 146, Color::Blue, none, none),
            spec(0xD8DEE6, 253, Color::White, Modifier::BOLD, Modifier::BOLD),
            spec(
                0xC792EA,
                176,
                Color::Magenta,
                Modifier::ITALIC,
                Modifier::ITALIC,
            ),
            spec(0x2C4A5A, nearest_256(0x2C4A5A), Color::DarkGray, none, none),
            spec(0x4FB3D9, 74, Color::Cyan, none, none),
            spec(0xE5B567, 179, Color::Yellow, none, none),
            spec(0xE0607E, 168, Color::LightRed, none, none),
        ],
    }
}

fn light_palette() -> Palette {
    let none = Modifier::empty();
    Palette {
        roles: [
            spec(0xF7F7F5, 255, Color::White, none, none),
            spec(0xEEEEEA, 254, Color::White, none, none),
            spec(0x1D232B, 235, Color::Black, none, none),
            spec(0x5C6670, 241, Color::DarkGray, none, none),
            spec(0xC9CDD2, nearest_256(0xC9CDD2), Color::Gray, none, none),
            spec(0x0B6E99, 24, Color::Blue, none, none),
            spec(0x1E5B2A, 22, Color::Green, none, none),
            spec(0x8A5A00, 94, Color::Yellow, none, Modifier::BOLD),
            spec(0xC2185B, 125, Color::Red, none, none),
            spec(0x17705A, 23, Color::Cyan, none, none),
            spec(0x4A5394, 60, Color::Blue, none, none),
            spec(0x1D232B, 235, Color::Black, Modifier::BOLD, Modifier::BOLD),
            spec(
                0x7B3FA0,
                90,
                Color::Magenta,
                Modifier::ITALIC,
                Modifier::ITALIC,
            ),
            spec(0xD6E4EC, nearest_256(0xD6E4EC), Color::Gray, none, none),
            spec(0x0B6E99, 24, Color::Blue, none, none),
            spec(0x8A5A00, 94, Color::Yellow, none, none),
            spec(0xC2185B, 125, Color::Red, none, none),
        ],
    }
}

fn phosphor_palette() -> Palette {
    let none = Modifier::empty();
    Palette {
        roles: [
            hex_nearest(0x050A06, Color::Black, none),
            hex_nearest(0x0C1A10, Color::Black, none),
            hex_nearest(0x7CFF9B, Color::Green, none),
            hex_nearest(0x3FAE5F, Color::Green, none),
            hex_nearest(0x163822, Color::DarkGray, none),
            hex_nearest(0xC8FFD4, Color::LightGreen, none),
            hex_nearest(0x7CFF9B, Color::Green, none),
            hex_nearest(0xE8FF7A, Color::Yellow, none),
            hex_nearest(0xFF6B5E, Color::LightRed, none),
            hex_nearest(0x4DE680, Color::Green, none),
            hex_nearest(0x9FD9B0, Color::LightGreen, none),
            hex_nearest_mod(0x7CFF9B, Color::Green, Modifier::BOLD, Modifier::BOLD),
            hex_nearest_mod(
                0xB8FFC9,
                Color::LightGreen,
                Modifier::ITALIC,
                Modifier::ITALIC,
            ),
            hex_nearest(0x0C3B1A, Color::DarkGray, none),
            hex_nearest(0x1F9E44, Color::Green, none),
            hex_nearest(0x7CFF9B, Color::Green, none),
            hex_nearest(0xE8FF7A, Color::Yellow, none),
        ],
    }
}

fn amber_palette() -> Palette {
    let none = Modifier::empty();
    Palette {
        roles: [
            hex_nearest(0x0E0900, Color::Black, none),
            hex_nearest(0x1C1300, Color::Black, none),
            hex_nearest(0xFFB000, Color::Yellow, none),
            hex_nearest(0xB88220, Color::Yellow, none),
            hex_nearest(0x2E2200, Color::DarkGray, none),
            hex_nearest(0xFFD580, Color::LightYellow, none),
            hex_nearest(0xFFC94D, Color::Yellow, none),
            hex_nearest(0xFFE9B0, Color::LightYellow, none),
            hex_nearest(0xFF6A3D, Color::LightRed, none),
            hex_nearest(0xFFA31A, Color::Yellow, none),
            hex_nearest(0xE0B870, Color::Yellow, none),
            hex_nearest_mod(0xFFB000, Color::Yellow, Modifier::BOLD, Modifier::BOLD),
            hex_nearest_mod(
                0xFFE0A3,
                Color::LightYellow,
                Modifier::ITALIC,
                Modifier::ITALIC,
            ),
            hex_nearest(0x3A2600, Color::DarkGray, none),
            hex_nearest(0xB88220, Color::Yellow, none),
            hex_nearest(0xFFB000, Color::Yellow, none),
            hex_nearest(0xFF6A3D, Color::LightRed, none),
        ],
    }
}

fn mono_palette() -> Palette {
    let reset = RoleSpec {
        rgb: None,
        index256: 0,
        ansi16: Color::Reset,
        modifiers: Modifier::empty(),
        ansi16_modifiers: Modifier::empty(),
    };
    let mut roles = [reset; 17];
    roles[idx(Role::Accent)] = RoleSpec {
        rgb: None,
        index256: 0,
        ansi16: Color::Reset,
        modifiers: Modifier::BOLD,
        ansi16_modifiers: Modifier::BOLD,
    };
    roles[idx(Role::Anl)] = RoleSpec {
        rgb: None,
        index256: 0,
        ansi16: Color::Reset,
        modifiers: Modifier::BOLD,
        ansi16_modifiers: Modifier::BOLD,
    };
    roles[idx(Role::Ai)] = RoleSpec {
        rgb: None,
        index256: 0,
        ansi16: Color::Reset,
        modifiers: Modifier::ITALIC,
        ansi16_modifiers: Modifier::ITALIC,
    };
    Palette { roles }
}

fn hex_nearest(hex: u32, ansi16: Color, modifiers: Modifier) -> RoleSpec {
    hex_nearest_mod(hex, ansi16, modifiers, modifiers)
}

fn hex_nearest_mod(
    hex: u32,
    ansi16: Color,
    modifiers: Modifier,
    ansi16_modifiers: Modifier,
) -> RoleSpec {
    spec(hex, nearest_256(hex), ansi16, modifiers, ansi16_modifiers)
}

fn role_styles(palette: &Palette, depth: ColorDepth, paints_background: bool) -> [Style; 17] {
    let mut styles = [Style::default(); 17];
    for role in Role::ALL {
        let spec = palette.roles[idx(role)];
        styles[idx(role)] = style_for(role, spec, depth, paints_background);
    }
    styles
}

fn style_for(role: Role, spec: RoleSpec, depth: ColorDepth, paints_background: bool) -> Style {
    let modifiers = match depth {
        ColorDepth::Ansi16 => spec.modifiers.union(spec.ansi16_modifiers),
        _ => spec.modifiers,
    };
    if depth == ColorDepth::None {
        return Style::default().add_modifier(modifiers);
    }
    let color = match depth {
        ColorDepth::TrueColor => spec
            .rgb
            .map(|rgb| Color::Rgb(rgb.r, rgb.g, rgb.b))
            .unwrap_or(Color::Reset),
        ColorDepth::Ansi256 => Color::Indexed(spec.index256),
        ColorDepth::Ansi16 => spec.ansi16,
        ColorDepth::None => Color::Reset,
    };
    if role == Role::Bg {
        if paints_background {
            Style::default().bg(color)
        } else {
            Style::default()
        }
    } else if role == Role::Panel {
        if paints_background {
            Style::default().bg(color).add_modifier(modifiers)
        } else {
            Style::default().fg(color).add_modifier(modifiers)
        }
    } else {
        Style::default().fg(color).add_modifier(modifiers)
    }
}

fn selected_style(
    palette: &Palette,
    depth: ColorDepth,
    paints_background: bool,
    fg: Style,
) -> Style {
    match depth {
        ColorDepth::None | ColorDepth::Ansi16 => Style::default().add_modifier(Modifier::REVERSED),
        ColorDepth::TrueColor | ColorDepth::Ansi256 => {
            let panel = palette.roles[idx(Role::Panel)];
            let bg = match depth {
                ColorDepth::TrueColor => panel
                    .rgb
                    .map(|rgb| Color::Rgb(rgb.r, rgb.g, rgb.b))
                    .unwrap_or(Color::Reset),
                ColorDepth::Ansi256 => Color::Indexed(panel.index256),
                ColorDepth::Ansi16 | ColorDepth::None => Color::Reset,
            };
            let _ = paints_background;
            fg.bg(bg)
        }
    }
}

fn ansi16_name(color: Color) -> &'static str {
    match color {
        Color::Reset => "default",
        Color::Black => "Black",
        Color::Red => "Red",
        Color::Green => "Green",
        Color::Yellow => "Yellow",
        Color::Blue => "Blue",
        Color::Magenta => "Magenta",
        Color::Cyan => "Cyan",
        Color::Gray => "Gray",
        Color::DarkGray => "DarkGray",
        Color::LightRed => "LightRed",
        Color::LightGreen => "LightGreen",
        Color::LightYellow => "LightYellow",
        Color::LightBlue => "LightBlue",
        Color::LightMagenta => "LightMagenta",
        Color::LightCyan => "LightCyan",
        Color::White => "White",
        Color::Indexed(_) => "indexed",
        Color::Rgb(_, _, _) => "rgb",
    }
}

/// Nearest xterm-256 index. Used for Phosphor/Amber and unspecified Dark/Light stops.
pub fn nearest_256(hex: u32) -> u8 {
    let rgb = Rgb::from_hex(hex);
    nearest_256_rgb(rgb)
}

fn nearest_256_rgb(rgb: Rgb) -> u8 {
    let cube_index = |value: u8| -> u8 {
        if value < 48 {
            0
        } else if value < 115 {
            1
        } else {
            ((u16::from(value) - 35) / 40).min(5) as u8
        }
    };
    let cube_level = |index: u8| -> u8 { if index == 0 { 0 } else { 40 * index + 55 } };
    let ri = cube_index(rgb.r);
    let gi = cube_index(rgb.g);
    let bi = cube_index(rgb.b);
    let cube = 16 + 36 * ri + 6 * gi + bi;
    let cube_rgb = Rgb::new(cube_level(ri), cube_level(gi), cube_level(bi));
    let gray_index = {
        let avg = (u16::from(rgb.r) + u16::from(rgb.g) + u16::from(rgb.b)) / 3;
        if avg < 8 {
            16
        } else if avg > 238 {
            231
        } else {
            232 + ((avg - 8) / 10) as u8
        }
    };
    let gray_value = if (232..=255).contains(&gray_index) {
        8 + (gray_index - 232) * 10
    } else if gray_index == 16 {
        0
    } else {
        255
    };
    let gray_rgb = Rgb::new(gray_value, gray_value, gray_value);
    if distance_sq(rgb, gray_rgb) < distance_sq(rgb, cube_rgb) {
        gray_index
    } else {
        cube
    }
}

fn distance_sq(a: Rgb, b: Rgb) -> i32 {
    let dr = i32::from(a.r) - i32::from(b.r);
    let dg = i32::from(a.g) - i32::from(b.g);
    let db = i32::from(a.b) - i32::from(b.b);
    dr * dr + dg * dg + db * db
}

pub fn contrast_ratio(fg: Rgb, bg: Rgb) -> f64 {
    let lighter = relative_luminance(fg).max(relative_luminance(bg));
    let darker = relative_luminance(fg).min(relative_luminance(bg));
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(rgb: Rgb) -> f64 {
    let linear = |channel: u8| {
        let s = f64::from(channel) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CvdKind {
    Deutan,
    Protan,
    Tritan,
}

impl CvdKind {
    pub const ALL: [CvdKind; 3] = [CvdKind::Deutan, CvdKind::Protan, CvdKind::Tritan];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deutan => "deutan",
            Self::Protan => "protan",
            Self::Tritan => "tritan",
        }
    }
}

/// Machado 2009 simulation (severity 1.0) then CIE76 ΔE in CIELAB.
pub fn delta_e_cvd(a: Rgb, b: Rgb, kind: CvdKind) -> f64 {
    delta_e_lab(to_lab(simulate_cvd(a, kind)), to_lab(simulate_cvd(b, kind)))
}

fn simulate_cvd(rgb: Rgb, kind: CvdKind) -> Rgb {
    let matrix = match kind {
        CvdKind::Protan => [
            [0.152_286, 1.052_583, -0.204_868],
            [0.114_503, 0.786_281, 0.099_216],
            [-0.003_882, -0.048_116, 1.051_998],
        ],
        CvdKind::Deutan => [
            [0.367_322, 0.860_646, -0.227_968],
            [0.280_085, 0.672_501, 0.047_413],
            [-0.011_820, 0.042_940, 0.968_881],
        ],
        CvdKind::Tritan => [
            [1.255_528, -0.076_749, -0.178_779],
            [-0.078_411, 0.930_809, 0.147_602],
            [0.004_733, 0.691_367, 0.303_900],
        ],
    };
    let linear = srgb_to_linear(rgb);
    let mut out = [0.0; 3];
    for (i, row) in matrix.iter().enumerate() {
        out[i] = row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2];
    }
    linear_to_srgb(out)
}

fn srgb_to_linear(rgb: Rgb) -> [f64; 3] {
    let channel = |value: u8| {
        let s = f64::from(value) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [channel(rgb.r), channel(rgb.g), channel(rgb.b)]
}

fn linear_to_srgb(linear: [f64; 3]) -> Rgb {
    let channel = |value: f64| {
        let clamped = value.clamp(0.0, 1.0);
        let s = if clamped <= 0.003_130_8 {
            12.92 * clamped
        } else {
            1.055 * clamped.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Rgb::new(channel(linear[0]), channel(linear[1]), channel(linear[2]))
}

struct Lab {
    l: f64,
    a: f64,
    b: f64,
}

fn to_lab(rgb: Rgb) -> Lab {
    let lin = srgb_to_linear(rgb);
    let x = 0.412_456_4 * lin[0] + 0.357_576_1 * lin[1] + 0.180_437_5 * lin[2];
    let y = 0.212_672_9 * lin[0] + 0.715_152_2 * lin[1] + 0.072_175_0 * lin[2];
    let z = 0.019_333_9 * lin[0] + 0.119_192_0 * lin[1] + 0.950_304_1 * lin[2];
    let (xn, yn, zn) = (0.950_47, 1.0, 1.088_83);
    let fx = lab_f(x / xn);
    let fy = lab_f(y / yn);
    let fz = lab_f(z / zn);
    Lab {
        l: 116.0 * fy - 16.0,
        a: 500.0 * (fx - fy),
        b: 200.0 * (fy - fz),
    }
}

fn lab_f(t: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if t > DELTA.powi(3) {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

fn delta_e_lab(a: Lab, b: Lab) -> f64 {
    let dl = a.l - b.l;
    let da = a.a - b.a;
    let db = a.b - b.b;
    (dl * dl + da * da + db * db).sqrt()
}

pub fn palette_rgb(name: ThemeName, role: Role) -> Option<Rgb> {
    palette_for(name).roles[idx(role)].rgb
}

#[cfg(test)]
pub fn palette_index256(name: ThemeName, role: Role) -> u8 {
    palette_for(name).roles[idx(role)].index256
}

pub struct ClosePair {
    pub left: Role,
    pub right: Role,
    pub delta: f64,
}

pub fn semantic_delta_e(name: ThemeName, kind: CvdKind) -> Vec<(Role, Role, f64)> {
    let mut pairs = Vec::new();
    let roles = Role::SEMANTIC;
    for (i, left) in roles.iter().enumerate() {
        let Some(left_rgb) = palette_rgb(name, *left) else {
            continue;
        };
        for right in roles.iter().skip(i + 1) {
            let Some(right_rgb) = palette_rgb(name, *right) else {
                continue;
            };
            pairs.push((*left, *right, delta_e_cvd(left_rgb, right_rgb, kind)));
        }
    }
    pairs
}

pub fn min_semantic_delta(name: ThemeName, kind: CvdKind) -> Option<(f64, Role, Role)> {
    semantic_delta_e(name, kind)
        .into_iter()
        .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(left, right, delta)| (delta, left, right))
}

pub fn close_pairs(name: ThemeName, kind: CvdKind, threshold: f64) -> Vec<ClosePair> {
    semantic_delta_e(name, kind)
        .into_iter()
        .filter(|(_, _, delta)| *delta < threshold)
        .map(|(left, right, delta)| ClosePair { left, right, delta })
        .collect()
}

pub fn critical_delta(name: ThemeName, left: Role, right: Role, kind: CvdKind) -> f64 {
    let a = palette_rgb(name, left).expect("critical roles have colour");
    let b = palette_rgb(name, right).expect("critical roles have colour");
    delta_e_cvd(a, b, kind)
}

#[cfg(test)]
pub const WCAG_AA: f64 = 4.5;
#[cfg(test)]
pub const CRITICAL_DELTA_FLOOR: f64 = 15.0;
pub const CLOSE_DELTA: f64 = 12.0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn truecolor() -> DetectedDepth {
        DetectedDepth {
            depth: ColorDepth::TrueColor,
            reason: "COLORTERM=truecolor".into(),
        }
    }

    fn config_with(theme: &str, color: &str, ascii: bool) -> Config {
        let mut config = Config::default();
        config.ui.theme = theme.into();
        config.ui.color = color.into();
        config.ui.ascii = ascii;
        config
    }

    #[test]
    fn detection_walks_colorterm_then_term_then_no_color() {
        let env = TerminalEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            no_color: false,
            colorfgbg: None,
        };
        let detected = detect_color_depth(&env, ColorMode::Auto);
        assert_eq!(detected.depth, ColorDepth::TrueColor);
        assert_eq!(detected.reason, "COLORTERM=truecolor");
        let env = TerminalEnv {
            colorterm: None,
            term: Some("xterm-256color".into()),
            no_color: false,
            colorfgbg: None,
        };
        let detected = detect_color_depth(&env, ColorMode::Auto);
        assert_eq!(detected.depth, ColorDepth::Ansi256);
        assert!(detected.reason.contains("256color"));
        let env = TerminalEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            no_color: true,
            colorfgbg: None,
        };
        let detected = detect_color_depth(&env, ColorMode::Auto);
        assert_eq!(detected.depth, ColorDepth::None);
        assert_eq!(detected.reason, "NO_COLOR");
        let env = TerminalEnv {
            colorterm: None,
            term: Some("xterm".into()),
            no_color: false,
            colorfgbg: None,
        };
        let detected = detect_color_depth(&env, ColorMode::Auto);
        assert_eq!(detected.depth, ColorDepth::Ansi16);
        let env = TerminalEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            no_color: true,
            colorfgbg: None,
        };
        let detected = detect_color_depth(&env, ColorMode::Ansi256);
        assert_eq!(detected.depth, ColorDepth::Ansi256);
        assert_eq!(detected.reason, "ui.color=256");
        let env = TerminalEnv {
            colorterm: None,
            term: Some("XTERM-256COLOR".into()),
            no_color: false,
            colorfgbg: None,
        };
        let detected = detect_color_depth(&env, ColorMode::Auto);
        assert_eq!(detected.depth, ColorDepth::Ansi256);
        assert!(detected.reason.to_ascii_lowercase().contains("256color"));
    }

    #[test]
    fn old_theme_names_warn_and_fall_back() {
        for name in ["default", "high-contrast"] {
            let config = config_with(name, "auto", false);
            assert!(
                config
                    .warnings()
                    .iter()
                    .any(|warning| warning.contains("unknown theme")),
                "{name}"
            );
            let theme = Theme::resolve(&config, &TerminalEnv::default());
            assert_eq!(theme.name, ThemeName::Dark, "{name}");
        }
    }

    #[test]
    fn auto_theme_reads_colorfgbg_and_never_picks_signature_themes() {
        let dark_env = TerminalEnv {
            colorfgbg: Some("15;0".into()),
            ..TerminalEnv::default()
        };
        assert_eq!(
            resolve_theme_name("auto", &dark_env, ColorDepth::TrueColor),
            ThemeName::Dark
        );
        let light_env = TerminalEnv {
            colorfgbg: Some("0;15".into()),
            ..TerminalEnv::default()
        };
        assert_eq!(
            resolve_theme_name("auto", &light_env, ColorDepth::TrueColor),
            ThemeName::Light
        );
        assert_eq!(
            resolve_theme_name("auto", &TerminalEnv::default(), ColorDepth::TrueColor),
            ThemeName::Dark
        );
    }

    #[test]
    fn no_color_and_mono_force_the_mono_palette() {
        let theme = Theme::resolve(
            &config_with("dark", "auto", false),
            &TerminalEnv {
                no_color: true,
                colorterm: Some("truecolor".into()),
                ..TerminalEnv::default()
            },
        );
        assert_eq!(theme.name, ThemeName::Mono);
        assert_eq!(theme.depth, ColorDepth::None);
        assert!(!theme.paints_background);
        let theme = Theme::build(ThemeName::Mono, truecolor(), false);
        assert_eq!(theme.depth, ColorDepth::None);
        assert_eq!(theme.muted, Style::default());
        assert!(
            theme.accent.add_modifier.contains(Modifier::BOLD) || theme.accent != Style::default()
        );
    }

    #[test]
    fn phosphor_and_amber_degrade_to_mono_at_16_colours() {
        let sixteen = DetectedDepth {
            depth: ColorDepth::Ansi16,
            reason: "TERM=xterm".into(),
        };
        let phosphor = Theme::build(ThemeName::Phosphor, sixteen.clone(), false);
        assert_eq!(phosphor.name, ThemeName::Mono);
        assert_eq!(phosphor.degrade_hint, Some("phosphor needs 256 colours"));
        let amber = Theme::build(ThemeName::Amber, sixteen, false);
        assert_eq!(amber.name, ThemeName::Mono);
        assert_eq!(amber.degrade_hint, Some("amber needs 256 colours"));
        let phosphor_256 = Theme::build(
            ThemeName::Phosphor,
            DetectedDepth {
                depth: ColorDepth::Ansi256,
                reason: "TERM=xterm-256color".into(),
            },
            false,
        );
        assert_eq!(phosphor_256.name, ThemeName::Phosphor);
        assert!(phosphor_256.paints_background);
        assert!(phosphor_256.degrade_hint.is_none());
    }

    #[test]
    fn documented_256_indices_match_the_spec() {
        assert_eq!(palette_index256(ThemeName::Dark, Role::Bg), 234);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Panel), 235);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Fg), 253);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Muted), 246);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Accent), 74);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Ok), 114);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Warn), 179);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Err), 168);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Col), 73);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Ext), 146);
        assert_eq!(palette_index256(ThemeName::Dark, Role::Ai), 176);
        assert_eq!(palette_index256(ThemeName::Light, Role::Bg), 255);
        assert_eq!(palette_index256(ThemeName::Light, Role::Panel), 254);
        assert_eq!(palette_index256(ThemeName::Light, Role::Fg), 235);
        assert_eq!(palette_index256(ThemeName::Light, Role::Muted), 241);
        assert_eq!(palette_index256(ThemeName::Light, Role::Accent), 24);
        assert_eq!(palette_index256(ThemeName::Light, Role::Ok), 22);
        assert_eq!(palette_index256(ThemeName::Light, Role::Warn), 94);
        assert_eq!(palette_index256(ThemeName::Light, Role::Err), 125);
        assert_eq!(palette_index256(ThemeName::Light, Role::Col), 23);
        assert_eq!(palette_index256(ThemeName::Light, Role::Ext), 60);
        assert_eq!(palette_index256(ThemeName::Light, Role::Ai), 90);
        assert_ne!(
            palette_index256(ThemeName::Light, Role::Ok),
            nearest_256(0x1E5B2A),
            "Light ok keeps index 22 rather than the grey nearest-256 match"
        );
        assert_ne!(
            palette_index256(ThemeName::Light, Role::Ai),
            nearest_256(0x7B3FA0),
            "Light ai keeps index 90 rather than the low-contrast nearest match"
        );
    }

    #[test]
    fn wcag_text_roles_meet_aa_on_bg_and_panel() {
        for name in [
            ThemeName::Dark,
            ThemeName::Light,
            ThemeName::Phosphor,
            ThemeName::Amber,
        ] {
            let bg = palette_rgb(name, Role::Bg).expect("bg");
            let panel = palette_rgb(name, Role::Panel).expect("panel");
            for role in Role::TEXT {
                let fg = palette_rgb(name, role).expect("text role");
                let vs_bg = contrast_ratio(fg, bg);
                let vs_panel = contrast_ratio(fg, panel);
                assert!(
                    vs_bg >= WCAG_AA,
                    "{name:?} {} vs bg = {vs_bg:.2}",
                    role.as_str()
                );
                assert!(
                    vs_panel >= WCAG_AA,
                    "{name:?} {} vs panel = {vs_panel:.2}",
                    role.as_str()
                );
            }
        }
    }

    #[test]
    fn colour_blind_critical_pairs_stay_above_floor() {
        for name in [
            ThemeName::Dark,
            ThemeName::Light,
            ThemeName::Phosphor,
            ThemeName::Amber,
        ] {
            for kind in CvdKind::ALL {
                let ok_err = critical_delta(name, Role::Ok, Role::Err, kind);
                let warn_err = critical_delta(name, Role::Warn, Role::Err, kind);
                assert!(
                    ok_err >= CRITICAL_DELTA_FLOOR,
                    "{name:?} ok/err {kind:?} ΔE {ok_err:.1}"
                );
                assert!(
                    warn_err >= CRITICAL_DELTA_FLOOR,
                    "{name:?} warn/err {kind:?} ΔE {warn_err:.1}"
                );
            }
        }
    }

    #[test]
    fn light_close_pairs_are_ext_ai_and_ok_col() {
        let deutan = min_semantic_delta(ThemeName::Light, CvdKind::Deutan).expect("deutan");
        assert_eq!(deutan.1, Role::Ext, "{deutan:?}");
        assert_eq!(deutan.2, Role::Ai, "{deutan:?}");
        let protan = min_semantic_delta(ThemeName::Light, CvdKind::Protan).expect("protan");
        assert!(
            matches!(
                (protan.1, protan.2),
                (Role::Ext, Role::Ai) | (Role::Ai, Role::Ext)
            ),
            "{protan:?}"
        );
        let tritan_close: Vec<(Role, Role)> =
            close_pairs(ThemeName::Light, CvdKind::Tritan, CLOSE_DELTA)
                .into_iter()
                .map(|pair| (pair.left, pair.right))
                .collect();
        assert!(
            tritan_close.contains(&(Role::Ok, Role::Col))
                || tritan_close.contains(&(Role::Col, Role::Ok))
                || tritan_close.iter().any(|pair| {
                    matches!(pair, (Role::Col, Role::Accent) | (Role::Accent, Role::Col))
                }),
            "{tritan_close:?}"
        );
    }

    #[test]
    fn color_constructors_live_only_in_theme_rs() {
        let mut src = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        src.push("src");
        let mut leaks = Vec::new();
        visit_rs(&src, &mut leaks);
        assert!(
            leaks.is_empty(),
            "Color:: must live only in src/tui/theme.rs:\n{}",
            leaks.join("\n")
        );
    }

    fn visit_rs(dir: &std::path::Path, leaks: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("read src") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.is_dir() {
                visit_rs(&path, leaks);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            if path.ends_with("tui/theme.rs") {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read rust");
            for (index, line) in text.lines().enumerate() {
                if line.contains("Color::") {
                    leaks.push(format!("{}:{}:{line}", path.display(), index + 1));
                }
            }
        }
    }
}
