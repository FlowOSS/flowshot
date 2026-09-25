//! Design tokens for `FlowShot`.
//!
//! Central, platform-free source of truth for visual constants: colors,
//! spacing, corner radii, shadows, typography, and animation easing curves.
//! UI crates should derive their styling from [`DesignTokens`] instead of
//! hard-coding values.

use serde::{Deserialize, Serialize};

/// Core color palette.
///
/// Colors are CSS-style hex strings (`#RRGGBB`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    /// Primary accent color used for highlights and active states.
    pub accent: String,
    /// High-contrast backdrop color (e.g. the dim overlay base).
    pub contrast: String,
    /// Opacity (0-255) applied when dimming the background.
    pub dim_opacity: u8,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            // Final FlowShot brand accent (Indigo 500).
            accent: "#6366F1".to_owned(),
            // Final FlowShot brand contrast (Slate 900).
            contrast: "#0F172A".to_owned(),
            dim_opacity: 190,
        }
    }
}

/// Spacing scale in logical pixels, built on a 4px grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spacing {
    /// Base unit; all other steps are multiples of it.
    pub base: u32,
    /// Tight spacing (1x base) for inline elements.
    pub small: u32,
    /// Default spacing (2x base) between related elements.
    pub medium: u32,
    /// Generous spacing (4x base) between sections.
    pub large: u32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            base: 4,
            small: 4,
            medium: 8,
            large: 16,
        }
    }
}

/// Corner radius scale in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Radii {
    /// Subtle rounding for chips, inputs, and small controls.
    pub small: u32,
    /// Default rounding for buttons and cards.
    pub medium: u32,
    /// Pronounced rounding for dialogs and panels.
    pub large: u32,
}

impl Default for Radii {
    fn default() -> Self {
        Self {
            small: 2,
            medium: 4,
            large: 8,
        }
    }
}

/// A single drop shadow definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    /// Blur radius in logical pixels.
    pub blur: u32,
    /// `[x, y]` offset in logical pixels.
    pub offset: [f32; 2],
    /// Shadow color as `#RRGGBBAA` (alpha in the last byte).
    pub color: String,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            blur: 4,
            offset: [0.0, 1.0],
            color: "#0000001F".to_owned(),
        }
    }
}

/// Elevation scale: three shadow presets for increasing depth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadows {
    /// Low elevation (tooltips, toolbar buttons).
    pub small: Shadow,
    /// Medium elevation (popovers, HUD).
    pub medium: Shadow,
    /// High elevation (dialogs, floating panels).
    pub large: Shadow,
}

impl Default for Shadows {
    fn default() -> Self {
        Self {
            small: Shadow {
                blur: 4,
                offset: [0.0, 1.0],
                color: "#0000001F".to_owned(),
            },
            medium: Shadow {
                blur: 8,
                offset: [0.0, 2.0],
                color: "#00000029".to_owned(),
            },
            large: Shadow {
                blur: 16,
                offset: [0.0, 4.0],
                color: "#00000033".to_owned(),
            },
        }
    }
}

/// Typography defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Typography {
    /// Preferred font family name (resolved by the platform font stack).
    pub family: String,
    /// Base font size in logical pixels.
    pub base_size: u32,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            family: "Noto Sans".to_owned(),
            base_size: 14,
        }
    }
}

/// A named cubic-bezier easing curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Easing {
    /// Curve identifier, e.g. `"standard"`.
    pub name: String,
    /// Cubic-bezier control points `[x1, y1, x2, y2]`.
    pub curve: [f32; 4],
}

impl Default for Easing {
    fn default() -> Self {
        Self::standard()
    }
}

impl Easing {
    /// General-purpose motion curve (Material "standard").
    #[must_use]
    pub fn standard() -> Self {
        Self {
            name: "standard".to_owned(),
            curve: [0.4, 0.0, 0.2, 1.0],
        }
    }

    /// Enter/decelerate curve: elements arriving on screen.
    #[must_use]
    pub fn decelerate() -> Self {
        Self {
            name: "decelerate".to_owned(),
            curve: [0.0, 0.0, 0.2, 1.0],
        }
    }

    /// Exit/accelerate curve: elements leaving the screen.
    #[must_use]
    pub fn accelerate() -> Self {
        Self {
            name: "accelerate".to_owned(),
            curve: [0.4, 0.0, 1.0, 1.0],
        }
    }

    /// Sharp curve for small, quick transitions.
    #[must_use]
    pub fn sharp() -> Self {
        Self {
            name: "sharp".to_owned(),
            curve: [0.4, 0.0, 0.6, 1.0],
        }
    }
}

/// The complete design token set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignTokens {
    /// Color palette.
    pub palette: Palette,
    /// Spacing scale.
    pub spacing: Spacing,
    /// Corner radius scale.
    pub radii: Radii,
    /// Shadow/elevation presets.
    pub shadows: Shadows,
    /// Typography defaults.
    pub typography: Typography,
    /// Named easing curves (at least three).
    pub easings: Vec<Easing>,
}

impl Default for DesignTokens {
    fn default() -> Self {
        Self {
            palette: Palette::default(),
            spacing: Spacing::default(),
            radii: Radii::default(),
            shadows: Shadows::default(),
            typography: Typography::default(),
            easings: vec![
                Easing::standard(),
                Easing::decelerate(),
                Easing::accelerate(),
                Easing::sharp(),
            ],
        }
    }
}

impl DesignTokens {
    /// Look up an easing curve by name.
    #[must_use]
    pub fn easing(&self, name: &str) -> Option<&Easing> {
        self.easings.iter().find(|e| e.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_matches_spec() {
        let palette = Palette::default();
        assert_eq!(palette.accent, "#6366F1");
        assert_eq!(palette.contrast, "#0F172A");
        assert_eq!(palette.dim_opacity, 190);
    }

    #[test]
    fn default_spacing_is_four_pixel_grid() {
        let spacing = Spacing::default();
        assert_eq!(spacing.base, 4);
        assert_eq!(spacing.small, spacing.base);
        assert_eq!(spacing.medium, spacing.base * 2);
        assert_eq!(spacing.large, spacing.base * 4);
    }

    #[test]
    fn default_tokens_expose_at_least_three_easings() {
        let tokens = DesignTokens::default();
        assert!(tokens.easings.len() >= 3);
        assert!(tokens.easing("standard").is_some());
        assert!(tokens.easing("decelerate").is_some());
        assert!(tokens.easing("accelerate").is_some());
        assert!(tokens.easing("nonexistent").is_none());
    }

    #[test]
    fn easing_curves_are_valid_bezier_control_points() {
        for easing in &DesignTokens::default().easings {
            let [x1, _, x2, _] = easing.curve;
            assert!(
                (0.0..=1.0).contains(&x1),
                "{}: x1 out of range",
                easing.name
            );
            assert!(
                (0.0..=1.0).contains(&x2),
                "{}: x2 out of range",
                easing.name
            );
        }
    }

    #[test]
    fn tokens_survive_toml_roundtrip() {
        let tokens = DesignTokens::default();
        let serialized = toml::to_string(&tokens).unwrap_or_else(|e| panic!("{e}"));
        let restored: DesignTokens = toml::from_str(&serialized).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(tokens, restored);
    }
}
