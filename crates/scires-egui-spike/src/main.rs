//! Native binary entry point for the SciRes egui workbench spike.

use scires_egui_spike::WorkbenchApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("SciRes Workbench (egui spike)"),
        ..Default::default()
    };

    eframe::run_native(
        "SciRes Workbench (egui spike)",
        options,
        Box::new(|_cc| Box::new(WorkbenchApp::new())),
    )
}
