// Branch `pen-input` (PEN_INPUT_PLAN 7, ARCHITECTURE 8.8): Help > Tablet setup. Static help for drawing tablets and pen
// displays, the live pen line, and (part G) the "Tablet input" choice: Windows Ink (recommended) / WinTab.

use eframe::egui;

use crate::state::TabletInput;
use crate::view::ViewCx;

/// The paragraphs of the dialog (also checked by the tests and the scenario `pen`).
pub(crate) const TABLET_HELP: [&str; 7] = [
    "Works with any tablet whose driver has Windows Ink turned on (the default): Wacom Tablet Properties > Mapping > \
     \"Use Windows Ink\"; Huion > Pen settings > \"Enable Windows Ink\"; XP-Pen / Pentablet > \"Windows Ink\".",
    "If the stroke does not follow the pressure, turn that option on. Drivers allow it per application: add Wrapped \
     Texture Editor to the driver's application list. Or choose Tablet input: WinTab below.",
    "Pressure options: Tool tab > Brush options (and Eraser options) > Pen pressure. The pressure curve (soft or firm) is \
     set in the tablet driver.",
    "Turn the pen around to erase. Hold the lower pen button while the tip touches the tablet for the right mouse \
     button (orbit in 3D, pan in 2D, context menus). Other buttons do what the driver maps them to (middle click pans, \
     keys work as shortcuts).",
    "The status bar shows \"Pen N %\" while the pen touches the tablet (in a narrow window: the Pen line under Pen \
     pressure).",
    "On a pen display with touch, a resting hand acts as a mouse: turn touch off in the tablet driver, or turn on \
     \"Ignore touch input when I'm using my pen\" in the Windows settings under Pen & Windows Ink.",
    "An Android tablet can be the drawing tablet too: Help > Tablet companion shows this window on the tablet and takes \
     its pen (with pressure), fingers and keyboard. The tablet app \"Tablet Companion\" is at \
     github.com/Maviman/tablet-companion/releases.",
];

/// The line under the Tablet input choice.
pub(crate) const TABLET_INPUT_NOTE: &str = "Use WinTab only if your driver's Windows Ink option is off. Takes effect at once.";

/// Draws Help > Tablet setup.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let max = super::max_dialog_size(ctx);
    let pen_line = cx.state.pen.status_line();
    let wintab_line = cx.state.pen.wintab_line();
    let response = super::modal("tablet").show(ctx, |ui| {
        ui.set_width(max.x.min(480.0));
        ui.heading("Tablet setup");
        ui.separator();
        egui::ScrollArea::vertical().id_salt("wte_tablet_help").max_height((max.y - 120.0).max(120.0)).show(ui, |ui| {
            for text in TABLET_HELP {
                ui.add(egui::Label::new(text).wrap());
                ui.add_space(4.0);
            }
            ui.weak(pen_line.as_str());
            ui.add_space(6.0);
            // Part G: the pressure source (a preference; takes effect at once, `PenState::sync_tablet_input`).
            ui.horizontal(|ui| {
                ui.label("Tablet input:");
                let input = &mut cx.state.prefs.tablet_input;
                let text = |t: TabletInput| match t {
                    TabletInput::WindowsInk => "Windows Ink (recommended)",
                    TabletInput::WinTab => "WinTab",
                };
                egui::ComboBox::from_id_salt("wte_tablet_input").selected_text(text(*input)).show_ui(ui, |ui| {
                    for t in [TabletInput::WindowsInk, TabletInput::WinTab] {
                        ui.selectable_value(input, t, text(t));
                    }
                });
            });
            ui.weak(TABLET_INPUT_NOTE);
            ui.weak(wintab_line);
        });
        ui.separator();
        let mut close = false;
        super::button_row(ui, |ui| close = ui.button("Close").clicked());
        close
    });
    if response.inner || super::backdrop_clicked(&response) {
        super::close(cx);
    }
}
