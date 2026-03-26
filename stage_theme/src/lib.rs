//! Dark theme for stage lighting controller GUIs.
//!
//! Designed for use in dark environments where the screen should stay as dim as possible
//! while remaining legible. Features ultra-dark backgrounds, muted accent colors that
//! won't be confused with stage lighting, and large touch targets for chaotic environments.

use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle, Vec2};
use eframe::egui::style::{Selection, WidgetVisuals};
use eframe::epaint::Shadow;

// --- Color palette ---

// Backgrounds (true black base)
const BG_DARKEST: Color32 = Color32::BLACK;
const BG_PANEL: Color32 = Color32::from_rgb(6, 6, 8);
const BG_WIDGET: Color32 = Color32::from_rgb(22, 22, 28);
const BG_ELEVATED: Color32 = Color32::from_rgb(32, 32, 40);

// Text
const TEXT_PRIMARY: Color32 = Color32::from_rgb(210, 210, 220);
/// Muted text for secondary/disabled elements. Available for downstream use.
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(140, 140, 160);

// Accent (steel blue — muted, won't conflict with stage colors)
const ACCENT: Color32 = Color32::from_rgb(70, 130, 180);
const ACCENT_HOVER: Color32 = Color32::from_rgb(90, 150, 200);
const ACCENT_DIM: Color32 = Color32::from_rgb(40, 70, 100);

// Status
const WARN: Color32 = Color32::from_rgb(200, 160, 60);
const ERROR: Color32 = Color32::from_rgb(180, 60, 60);

// Borders (brighter so dividers are clearly visible against true black)
const BORDER_SUBTLE: Color32 = Color32::from_rgb(60, 60, 75);
const BORDER_VISIBLE: Color32 = Color32::from_rgb(80, 80, 100);

/// Apply the stage dark theme to an egui context.
///
/// Call this once at initialization time (e.g. in the `CreationContext` callback).
/// The style persists across all subsequent frames.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    // Ultra-dark backgrounds
    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = Color32::from_rgb(10, 10, 14);
    visuals.extreme_bg_color = BG_DARKEST;
    visuals.faint_bg_color = Color32::from_rgb(12, 12, 16);

    // Accent colors
    visuals.hyperlink_color = ACCENT;
    visuals.selection = Selection {
        bg_fill: ACCENT_DIM,
        stroke: Stroke::new(1.0, ACCENT_HOVER),
    };

    // Status colors
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = ERROR;

    // Widget states
    visuals.widgets.noninteractive = WidgetVisuals {
        bg_fill: BG_PANEL,
        weak_bg_fill: BG_PANEL,
        bg_stroke: Stroke::new(1.0, BORDER_SUBTLE),
        corner_radius: 4.0.into(),
        fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
        expansion: 0.0,
    };

    visuals.widgets.inactive = WidgetVisuals {
        bg_fill: BG_WIDGET,
        weak_bg_fill: BG_WIDGET,
        bg_stroke: Stroke::new(1.0, BORDER_SUBTLE),
        corner_radius: 4.0.into(),
        fg_stroke: Stroke::new(1.0, Color32::from_rgb(180, 180, 200)),
        expansion: 0.0,
    };

    visuals.widgets.hovered = WidgetVisuals {
        bg_fill: BG_ELEVATED,
        weak_bg_fill: BG_ELEVATED,
        bg_stroke: Stroke::new(1.0, ACCENT),
        corner_radius: 4.0.into(),
        fg_stroke: Stroke::new(1.0, Color32::from_rgb(220, 220, 230)),
        expansion: 1.0,
    };

    visuals.widgets.active = WidgetVisuals {
        bg_fill: ACCENT_DIM,
        weak_bg_fill: ACCENT_DIM,
        bg_stroke: Stroke::new(2.0, ACCENT_HOVER),
        corner_radius: 4.0.into(),
        fg_stroke: Stroke::new(1.0, Color32::WHITE),
        expansion: 1.0,
    };

    visuals.widgets.open = WidgetVisuals {
        bg_fill: BG_ELEVATED,
        weak_bg_fill: BG_ELEVATED,
        bg_stroke: Stroke::new(1.0, BORDER_VISIBLE),
        corner_radius: 4.0.into(),
        fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
        expansion: 0.0,
    };

    // Minimal shadows (barely visible on dark backgrounds)
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow {
        offset: [0, 2],
        blur: 4,
        spread: 0,
        color: Color32::from_black_alpha(60),
    };

    // Window styling
    visuals.window_corner_radius = 8.0.into();
    visuals.menu_corner_radius = 6.0.into();
    visuals.window_stroke = Stroke::new(1.0, BORDER_SUBTLE);

    // Slider trailing fill for clear value indication
    visuals.slider_trailing_fill = true;

    // Muted text for disabled/secondary elements
    visuals.override_text_color = None;

    ctx.set_visuals(visuals);

    // Spacing and sizing for large touch targets
    ctx.all_styles_mut(|style| {
        style.spacing.interact_size = Vec2::new(60.0, 36.0);
        style.spacing.button_padding = Vec2::new(12.0, 8.0);
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.icon_width = 22.0;
        style.spacing.icon_width_inner = 16.0;

        // Larger text for readability in low light
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(24.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(16.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(15.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(14.0, FontFamily::Monospace),
        );
    });
}
