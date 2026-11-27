//! Conversion from toolkit-agnostic [`scires_ui::Theme`] tokens to [`eframe::egui::Visuals`].

use eframe::egui::{Color32, Stroke, Visuals};
use scires_ui::{Color, Theme};

/// Selectable built-in theme mode for the workbench spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// Default dark theme ([`Theme::dark`]).
    Dark,
    /// Default light theme ([`Theme::light`]).
    Light,
}

impl ThemeMode {
    /// Materialize the corresponding [`scires_ui::Theme`].
    #[must_use]
    pub fn theme(self) -> Theme {
        match self {
            Self::Dark => Theme::dark(),
            Self::Light => Theme::light(),
        }
    }

    /// Flip between dark and light modes.
    pub fn toggle(&mut self) {
        *self = match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        };
    }

    /// Short human-readable label for toolbar controls.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

/// Convert a [`scires_ui::Color`] token into an opaque [`Color32`].
#[must_use]
pub const fn color_to_egui(color: Color) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}

/// Build [`Visuals`] directly from [`scires_ui::Theme`] design tokens.
///
/// Dark vs. light mode is inferred from the background colour luminance so custom
/// palettes map consistently without hard-coding theme names.
#[must_use]
pub fn theme_to_visuals(theme: &Theme) -> Visuals {
    let p = &theme.palette;
    let is_dark = u16::from(p.background.r) + u16::from(p.background.g) + u16::from(p.background.b)
        < 384;
    let mut visuals = if is_dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    let bg = color_to_egui(p.background);
    let surface = color_to_egui(p.surface);
    let surface_raised = color_to_egui(p.surface_raised);
    let border = color_to_egui(p.border);
    let text = color_to_egui(p.text);
    let accent = color_to_egui(p.accent);
    let warning = color_to_egui(p.warning);
    let danger = color_to_egui(p.danger);

    visuals.dark_mode = is_dark;
    visuals.override_text_color = Some(text);
    visuals.panel_fill = surface;
    visuals.window_fill = surface;
    visuals.extreme_bg_color = bg;
    visuals.faint_bg_color = surface_raised;
    visuals.code_bg_color = bg;
    visuals.window_stroke = Stroke::new(1.0, border);
    visuals.hyperlink_color = accent;
    visuals.warn_fg_color = warning;
    visuals.error_fg_color = danger;

    visuals.selection.bg_fill = accent;
    visuals.selection.stroke = Stroke::new(1.0, text);

    visuals.widgets.noninteractive.bg_fill = surface;
    visuals.widgets.noninteractive.weak_bg_fill = surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);

    visuals.widgets.inactive.bg_fill = surface_raised;
    visuals.widgets.inactive.weak_bg_fill = surface_raised;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);

    visuals.widgets.hovered.bg_fill = surface_raised;
    visuals.widgets.hovered.weak_bg_fill = surface_raised;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text);

    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.weak_bg_fill = accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, text);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, text);

    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_dark_and_light_theme_tokens_to_visuals() {
        let dark = Theme::dark();
        let dark_visuals = theme_to_visuals(&dark);
        assert!(dark_visuals.dark_mode);
        assert_eq!(dark_visuals.panel_fill, color_to_egui(dark.palette.surface));
        assert_eq!(
            dark_visuals.extreme_bg_color,
            color_to_egui(dark.palette.background)
        );
        assert_eq!(
            dark_visuals.selection.bg_fill,
            color_to_egui(dark.palette.accent)
        );
        assert_eq!(
            dark_visuals.error_fg_color,
            color_to_egui(dark.palette.danger)
        );
        assert_eq!(
            dark_visuals.warn_fg_color,
            color_to_egui(dark.palette.warning)
        );

        let light = Theme::light();
        let light_visuals = theme_to_visuals(&light);
        assert!(!light_visuals.dark_mode);
        assert_eq!(
            light_visuals.panel_fill,
            color_to_egui(light.palette.surface)
        );
        assert_eq!(
            light_visuals.extreme_bg_color,
            color_to_egui(light.palette.background)
        );
        assert_eq!(
            light_visuals.override_text_color,
            Some(color_to_egui(light.palette.text))
        );
        assert_ne!(dark_visuals.panel_fill, light_visuals.panel_fill);
    }

    #[test]
    fn theme_mode_toggles_between_dark_and_light() {
        let mut mode = ThemeMode::Dark;
        assert_eq!(mode.label(), "Dark");
        mode.toggle();
        assert_eq!(mode, ThemeMode::Light);
        assert_eq!(mode.label(), "Light");
    }
}
