use serde::{Deserialize, Serialize};

/// An opaque sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Color {
    /// Build a colour from `0xRRGGBB`.
    #[must_use]
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xFF) as u8,
            g: ((rgb >> 8) & 0xFF) as u8,
            b: (rgb & 0xFF) as u8,
        }
    }

    /// Format as `#RRGGBB`.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Semantic colour tokens shared by the front-end and the Figma prototype.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    /// Window background.
    pub background: Color,
    /// Panel / sidebar surface.
    pub surface: Color,
    /// Raised surface (tabs, toolbars).
    pub surface_raised: Color,
    /// Separators and outlines.
    pub border: Color,
    /// Primary text.
    pub text: Color,
    /// Secondary text.
    pub text_muted: Color,
    /// Brand / selection accent.
    pub accent: Color,
    /// Success (kernel idle, test passed).
    pub success: Color,
    /// Warning (kernel busy).
    pub warning: Color,
    /// Error / breakpoint.
    pub danger: Color,
}

/// A named theme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    /// Display name.
    pub name: String,
    /// Colour tokens.
    pub palette: Palette,
}

impl Theme {
    /// Default dark theme (matches the Figma prototype).
    #[must_use]
    pub fn dark() -> Self {
        Self {
            name: "SciRes Dark".into(),
            palette: Palette {
                background: Color::hex(0x0F1117),
                surface: Color::hex(0x161922),
                surface_raised: Color::hex(0x1E2230),
                border: Color::hex(0x2A2F3E),
                text: Color::hex(0xE6E8EF),
                text_muted: Color::hex(0x8B91A5),
                accent: Color::hex(0x4F8CFF),
                success: Color::hex(0x3FB950),
                warning: Color::hex(0xD29922),
                danger: Color::hex(0xF85149),
            },
        }
    }

    /// Default light theme.
    #[must_use]
    pub fn light() -> Self {
        Self {
            name: "SciRes Light".into(),
            palette: Palette {
                background: Color::hex(0xFFFFFF),
                surface: Color::hex(0xF6F7F9),
                surface_raised: Color::hex(0xECEEF2),
                border: Color::hex(0xD8DBE2),
                text: Color::hex(0x1B1F2A),
                text_muted: Color::hex(0x5D6475),
                accent: Color::hex(0x2F6FEB),
                success: Color::hex(0x1A7F37),
                warning: Color::hex(0x9A6700),
                danger: Color::hex(0xCF222E),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(Color::hex(0x0A_B1_FF).to_hex(), "#0AB1FF");
    }

    #[test]
    fn themes_have_distinct_backgrounds() {
        assert_ne!(Theme::dark().palette.background, Theme::light().palette.background);
    }
}
