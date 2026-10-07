//! Colour palettes. Themes stay understated: dense, readable, no decorative noise.

use ratatui::style::{Color, Modifier, Style};

use crate::config::Config;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub title: Style,
    pub header: Style,
    pub focused_border: Style,
    pub unfocused_border: Style,
    pub selected: Style,
    pub muted: Style,
    pub status: Style,
    pub error: Style,
    pub success: Style,
    pub warning: Style,
    pub agent: Style,
    pub badge_ai: Style,
    pub badge_col: Style,
    pub badge_anl: Style,
}

impl Theme {
    pub fn from_config(config: &Config) -> Self {
        match config.ui.theme.as_str() {
            "high-contrast" => Self::high_contrast(),
            "monochrome" => Self::monochrome(),
            _ => Self::default_theme(),
        }
    }

    fn default_theme() -> Self {
        Self {
            title: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            header: Style::default().fg(Color::Cyan),
            focused_border: Style::default().fg(Color::Cyan),
            unfocused_border: Style::default().fg(Color::DarkGray),
            selected: Style::default().fg(Color::Black).bg(Color::Yellow),
            muted: Style::default().fg(Color::DarkGray),
            status: Style::default().fg(Color::Gray),
            error: Style::default().fg(Color::Red),
            success: Style::default().fg(Color::Green),
            warning: Style::default().fg(Color::Yellow),
            agent: Style::default().fg(Color::Magenta),
            badge_ai: Style::default().fg(Color::Magenta),
            badge_col: Style::default().fg(Color::Green),
            badge_anl: Style::default().fg(Color::Blue),
        }
    }

    fn high_contrast() -> Self {
        Self {
            title: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            header: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            focused_border: Style::default().fg(Color::White),
            unfocused_border: Style::default().fg(Color::Gray),
            selected: Style::default().fg(Color::Black).bg(Color::White),
            muted: Style::default().fg(Color::Gray),
            status: Style::default().fg(Color::White),
            error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            success: Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            warning: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            agent: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::UNDERLINED),
            badge_ai: Style::default().fg(Color::Yellow),
            badge_col: Style::default().fg(Color::White),
            badge_anl: Style::default().fg(Color::White),
        }
    }

    fn monochrome() -> Self {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let reversed = Style::default().add_modifier(Modifier::REVERSED);
        let dim = Style::default().add_modifier(Modifier::DIM);
        Self {
            title: bold,
            header: bold,
            focused_border: bold,
            unfocused_border: dim,
            selected: reversed,
            muted: dim,
            status: Style::default(),
            error: bold,
            success: Style::default(),
            warning: bold,
            agent: Style::default().add_modifier(Modifier::UNDERLINED),
            badge_ai: bold,
            badge_col: Style::default(),
            badge_anl: Style::default(),
        }
    }
}
