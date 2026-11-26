use eframe::egui::{self, Color32, Stroke};

/// Colors shared with the light and dark Bluefin app themes.
struct Palette {
    background: Color32,
    panel: Color32,
    hover: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    control_border: Color32,
    divider: Color32,
    primary: Color32,
    on_primary: Color32,
}

impl Palette {
    fn for_theme(theme: egui::Theme) -> Self {
        let navy = Color32::from_rgb(0x06, 0x17, 0x27);
        let mist = Color32::from_rgb(0xA3, 0xAC, 0xB1);
        let moon = Color32::from_rgb(0xCC, 0xD0, 0xD3);
        let lavender = Color32::from_rgb(0x82, 0x89, 0xA5);
        match theme {
            egui::Theme::Light => Self {
                background: mist,
                panel: moon,
                hover: Color32::from_rgb(0xBD, 0xC5, 0xC9),
                text: navy,
                muted: Color32::from_rgb(0x2B, 0x40, 0x50),
                accent: Color32::from_rgb(0x33, 0x3B, 0x5A),
                control_border: Color32::from_rgb(0x46, 0x59, 0x66),
                divider: Color32::from_rgb(0x8A, 0x98, 0x9F),
                primary: navy,
                on_primary: moon,
            },
            egui::Theme::Dark => Self {
                background: navy,
                panel: Color32::from_rgb(0x10, 0x24, 0x35),
                hover: Color32::from_rgb(0x18, 0x2D, 0x3D),
                text: moon,
                muted: mist,
                accent: Color32::from_rgb(0xAD, 0xB5, 0xD1),
                control_border: Color32::from_rgb(0x63, 0x78, 0x89),
                divider: Color32::from_rgb(0x2D, 0x43, 0x54),
                primary: lavender,
                on_primary: navy,
            },
        }
    }
}

pub fn configure(ctx: &egui::Context) {
    // Register both palettes without overriding the system or saved preference.
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let palette = Palette::for_theme(theme);
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = match theme {
            egui::Theme::Light => egui::Visuals::light(),
            egui::Theme::Dark => egui::Visuals::dark(),
        };
        let visuals = &mut style.visuals;
        visuals.panel_fill = palette.background;
        visuals.window_fill = palette.panel;
        visuals.window_stroke = Stroke::new(1.0, palette.divider);
        visuals.extreme_bg_color = palette.panel;
        visuals.text_edit_bg_color = Some(palette.panel);
        visuals.faint_bg_color = palette.hover;
        visuals.code_bg_color = palette.hover;
        visuals.hyperlink_color = palette.accent;
        visuals.weak_text_color = Some(palette.muted);
        // Let selected controls use their own contrasting foreground.
        visuals.override_text_color = None;
        visuals.selection.bg_fill = palette.primary;
        visuals.selection.stroke = Stroke::new(2.0, palette.on_primary);
        visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = palette.panel;
        widgets.noninteractive.weak_bg_fill = palette.panel;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.divider);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
        for widget in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            widget.bg_fill = palette.hover;
            widget.weak_bg_fill = palette.hover;
            widget.bg_stroke = Stroke::new(1.0, palette.control_border);
            widget.fg_stroke = Stroke::new(1.0, palette.text);
        }
        widgets.hovered.bg_stroke = Stroke::new(1.5, palette.accent);
        widgets.active.bg_stroke = Stroke::new(2.0, palette.accent);
        widgets.open.bg_stroke = Stroke::new(1.5, palette.accent);

        style.wrap_mode = Some(egui::TextWrapMode::Wrap);
        style.spacing.interact_size.y = 44.0;
        style.spacing.text_edit_width = 320.0;
        style.spacing.item_spacing = egui::vec2(12.0, 10.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        for (text_style, size) in [
            (egui::TextStyle::Body, 16.0),
            (egui::TextStyle::Button, 17.0),
            (egui::TextStyle::Heading, 30.0),
            (egui::TextStyle::Small, 13.0),
        ] {
            style
                .text_styles
                .insert(text_style, egui::FontId::proportional(size));
        }
        ctx.set_style_of(theme, style);
    }
}

pub(super) fn primary_button(ui: &egui::Ui, label: &str) -> egui::Button<'static> {
    let selection = ui.visuals().selection;
    egui::Button::new(egui::RichText::new(label.to_owned()).color(selection.stroke.color))
        .fill(selection.bg_fill)
}
