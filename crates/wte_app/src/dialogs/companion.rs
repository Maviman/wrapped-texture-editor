// Branch `companion` (COMPANION_PLAN 3, ARCHITECTURE 8.8 / 8.14): Help > Tablet companion. The three links (USB cable,
// Wi-Fi, Bluetooth) with their status lines, the pairing code or the Allow / Deny question, the paired tablets, the
// options and the fixed security text. Link switches, Allow / Deny, Forget, Disconnect and Fit go through
// `AppAction::Companion`; the options (resize, fingers) are preferences the companion reads every frame.

use eframe::egui;

use crate::action::AppAction;
use crate::companion::{CompanionAction, LinkKind, TABLET_APP_NAME, TABLET_APP_RELEASES, adb, crypto, link_name};
use crate::state::FingerMode;
use crate::view::ViewCx;

/// The fixed security text (C9; also in Help and the README).
pub(crate) const SECURITY_TEXT: &str = "The connection is checked with a pairing key, but the picture and the input are not \
     encrypted. Use Wi-Fi only on a network you trust; USB does not use the network.";

/// The first paragraph: what this is and where the tablet app comes from.
pub(crate) fn intro() -> String {
    format!(
        "Draw on an Android tablet: the tablet shows this window live and sends the pen, your fingers and the keyboard \
         back. Install the {TABLET_APP_NAME} app on the tablet from {TABLET_APP_RELEASES} (the .apk file)."
    )
}

/// Draws Help > Tablet companion.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let max = super::max_dialog_size(ctx);
    let mut actions: Vec<AppAction> = Vec::new();
    let status = cx.state.companion.clone();
    let selftest = cx.state.selftest;
    let response = super::modal("companion").show(ctx, |ui| {
        ui.set_width(max.x.min(520.0));
        ui.heading("Tablet companion");
        ui.separator();
        egui::ScrollArea::vertical().id_salt("wte_companion").max_height((max.y - 120.0).max(160.0)).show(ui, |ui| {
            ui.add(egui::Label::new(intro()).wrap());
            ui.add_space(6.0);
            if let Some((name, link)) = &status.session {
                ui.horizontal(|ui| {
                    ui.strong(format!("Connected: {name} ({})", link_name(*link)));
                    if ui.button("Disconnect").clicked() {
                        actions.push(AppAction::Companion(CompanionAction::Disconnect));
                    }
                    if status.fitted {
                        if ui.button("Restore window size").clicked() {
                            actions.push(AppAction::Companion(CompanionAction::Fit(false)));
                        }
                    } else if ui.add_enabled(cx.state.prefs.companion.allow_resize, egui::Button::new("Fit to tablet")).clicked() {
                        actions.push(AppAction::Companion(CompanionAction::Fit(true)));
                    }
                });
                ui.add_space(4.0);
            }
            // The three links.
            let prefs = cx.state.prefs.companion.clone();
            let switch = |ui: &mut egui::Ui, actions: &mut Vec<AppAction>, label: &str, on: bool, link: LinkKind| {
                let mut v = on;
                if ui.add_enabled(!selftest, egui::Checkbox::new(&mut v, label)).changed() {
                    actions.push(AppAction::Companion(CompanionAction::SetLink(link, v)));
                }
            };
            switch(ui, &mut actions, "USB cable", prefs.usb, LinkKind::Usb);
            ui.indent("usb", |ui| {
                if prefs.usb {
                    ui.weak(status.adb_line.as_str());
                    if status.adb_line == adb::AdbLine::NoAdb.text() {
                        ui.add(
                            egui::Label::new(format!(
                                "Download \"SDK Platform-Tools for Windows\" from {}, unpack it, then:",
                                adb::PLATFORM_TOOLS_URL
                            ))
                            .wrap(),
                        );
                        if ui.button("Browse for adb.exe...").clicked() {
                            actions.push(AppAction::Companion(CompanionAction::BrowseAdb(None)));
                        }
                    }
                } else {
                    ui.weak(format!("Turn on, connect the tablet with its cable, then choose USB in {TABLET_APP_NAME}."));
                }
            });
            switch(ui, &mut actions, "Wi-Fi", prefs.wifi, LinkKind::Wifi);
            ui.indent("wifi", |ui| {
                if prefs.wifi {
                    let addrs: Vec<String> = status.lan.iter().map(|a| a.to_string()).collect();
                    let port = status.tcp.first().map_or(prefs.port, |a| a.port());
                    ui.weak(format!(
                        "This PC: {} {} port {port}",
                        crate::companion::server::desktop_name(),
                        if addrs.is_empty() { String::new() } else { format!("({})", addrs.join(", ")) }
                    ));
                } else {
                    ui.weak("Turn on to connect over your home network (Windows asks once to allow the app on private networks).");
                }
            });
            switch(ui, &mut actions, "Bluetooth", prefs.bluetooth, LinkKind::Bluetooth);
            ui.indent("bt", |ui| {
                if prefs.bluetooth || (!status.bt_line.is_empty() && status.bt_line != "Off") {
                    ui.weak(status.bt_line.as_str());
                }
                if !prefs.bluetooth {
                    ui.weak(format!(
                        "Pair the tablet in Windows Settings > Bluetooth & devices once, turn this on, then choose Bluetooth \
                         in {TABLET_APP_NAME}. The picture is coarser and slower than USB or Wi-Fi."
                    ));
                }
            });
            if selftest {
                ui.weak(format!("Self-test: listening on {}", status.tcp.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")));
            }
            for e in &status.errors {
                ui.colored_label(super::error_color(ui), e);
            }
            ui.separator();
            // Pairing.
            ui.strong("Pairing");
            if let Some(code) = &status.code {
                ui.horizontal(|ui| {
                    ui.label("Type this code on the tablet:");
                    ui.label(egui::RichText::new(crypto::display_code(code)).monospace().size(20.0).strong());
                });
            } else {
                ui.weak("A tablet connected with the cable (USB) or Bluetooth is allowed here; Wi-Fi shows a code.");
            }
            if let Some((id, name, _)) = &status.confirm {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{name} wants to connect")).strong());
                    if ui.button("Allow").clicked() {
                        actions.push(AppAction::Companion(CompanionAction::Allow(*id)));
                    }
                    if ui.button("Deny").clicked() {
                        actions.push(AppAction::Companion(CompanionAction::Deny(*id)));
                    }
                });
            }
            if let Some(r) = &status.last_refused {
                ui.colored_label(super::warn_color(ui), format!("Refused: {r}"));
            }
            ui.add_space(4.0);
            ui.strong("Paired tablets");
            if prefs.paired.is_empty() {
                ui.weak("None yet.");
            }
            for p in &prefs.paired {
                ui.horizontal(|ui| {
                    ui.label(format!("{}  (since {})", p.name, p.added));
                    if ui.small_button("Forget").clicked() {
                        actions.push(AppAction::Companion(CompanionAction::Forget(p.id.clone())));
                    }
                });
            }
            ui.separator();
            // Options.
            ui.strong("Options");
            ui.checkbox(&mut cx.state.prefs.companion.allow_resize, "Let the tablet resize this window to its shape");
            ui.label("Fingers on the tablet:");
            ui.indent("fingers", |ui| {
                let f = &mut cx.state.prefs.companion.fingers;
                ui.radio_value(f, FingerMode::Navigate, "Navigate in the view, click elsewhere");
                ui.radio_value(f, FingerMode::Mouse, "Act like the mouse everywhere");
            });
            ui.horizontal(|ui| {
                ui.label("Port (advanced):");
                let r = ui.add_enabled(!selftest, egui::DragValue::new(&mut cx.state.prefs.companion.port).range(1024..=65535));
                if r.drag_stopped() || r.lost_focus() {
                    actions.push(AppAction::Companion(CompanionAction::SetPort(cx.state.prefs.companion.port)));
                }
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(SECURITY_TEXT).weak()).wrap());
        });
        ui.separator();
        let mut close = false;
        super::button_row(ui, |ui| close = ui.button("Close").clicked());
        close
    });
    cx.state.actions.extend(actions);
    if response.inner || super::backdrop_clicked(&response) {
        super::close(cx);
    }
}
