//! Main egui workbench state, docked panel rendering, and `scires_app::App` wiring.

use crate::dock::{build_dock_state, DockTab};
use crate::plot::{render_plot, sample_plot_spec};
use crate::theme::{color_to_egui, theme_to_visuals, ThemeMode};
use eframe::egui::{self, Color32, Key, KeyboardShortcut, Modifiers, RichText, ScrollArea, TextEdit};
use egui_dock::{DockArea as EguiDockArea, DockState, Style, TabViewer};
use scires_app::{App, Command, Reply};
use scires_core::{DomainEvent, Language, Value};
use scires_debug::DebugState;
use scires_kernel::{mock::EchoKernelFactory, KernelStatus, Output};
use scires_ui::{Layout, Palette, Theme};
use scires_viz::{PlotSpec, PLOT_MIME};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Category of an entry displayed in the console panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleKind {
    /// Standard output from kernel execution.
    Stdout,
    /// Standard error from kernel execution.
    Stderr,
    /// Rich MIME display output.
    Display,
    /// Structured error or command failure.
    Error,
    /// Informational lifecycle message (e.g. restart notice).
    Info,
}

/// Ordered console line produced by executing code or managing a kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleEntry {
    /// Entry classification.
    pub kind: ConsoleKind,
    /// Human-readable content.
    pub text: String,
}

impl ConsoleEntry {
    /// Create a new console entry.
    #[must_use]
    pub fn new(kind: ConsoleKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }

    /// Accessible text prefix so output streams never rely on colour alone.
    #[must_use]
    pub fn prefix(&self) -> &'static str {
        match self.kind {
            ConsoleKind::Stdout => "[out]",
            ConsoleKind::Stderr => "[stderr]",
            ConsoleKind::Display => "[display]",
            ConsoleKind::Error => "[error]",
            ConsoleKind::Info => "[info]",
        }
    }
}

/// Shared observer state updated by [`scires_core::EventBus`] subscriptions.
#[derive(Default)]
struct ObservedEvents {
    last_kernel_status: Option<(Language, String)>,
    last_debug_state: Option<String>,
}

/// Standalone native workbench spike wired to [`scires_app::App`].
pub struct WorkbenchApp {
    app: App,
    observed: Arc<Mutex<ObservedEvents>>,
    active_language: Language,
    kernel_status: KernelStatus,
    layout: Layout,
    dock_state: DockState<DockTab>,
    theme_mode: ThemeMode,
    theme: Theme,
    files: Vec<String>,
    active_file: String,
    editor_code: String,
    console_entries: Vec<ConsoleEntry>,
    active_plot: PlotSpec,
    debug_state: DebugState,
    breakpoints: Vec<(String, u32)>,
}

impl Default for WorkbenchApp {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkbenchApp {
    /// Create a standalone workbench pre-registered with `EchoKernel` for Julia and Python.
    #[must_use]
    pub fn new() -> Self {
        let mut app = App::default();
        app.register_kernel(Arc::new(EchoKernelFactory::new(Language::Julia)));
        app.register_kernel(Arc::new(EchoKernelFactory::new(Language::Python)));

        let observed = Arc::new(Mutex::new(ObservedEvents::default()));
        let obs_clone = Arc::clone(&observed);
        app.events().subscribe(Box::new(move |event| {
            if let Ok(mut guard) = obs_clone.lock() {
                match event {
                    DomainEvent::KernelStatus { language, status } => {
                        guard.last_kernel_status = Some((language.clone(), status.clone()));
                    }
                    DomainEvent::DebugState(state) => {
                        guard.last_debug_state = Some(state.clone());
                    }
                    _ => {}
                }
            }
        }));

        let layout = Layout::scientific();
        let dock_state = build_dock_state(&layout);
        let theme_mode = ThemeMode::Dark;
        let theme = theme_mode.theme();

        Self {
            app,
            observed,
            active_language: Language::Julia,
            kernel_status: KernelStatus::NotStarted,
            layout,
            dock_state,
            theme_mode,
            theme,
            files: vec![
                "analysis.jl".into(),
                "model.jl".into(),
                "Project.toml".into(),
                "preprocess.py".into(),
            ],
            active_file: "analysis.jl".into(),
            editor_code: "x = 42\ny = 3.14".into(),
            console_entries: vec![ConsoleEntry::new(
                ConsoleKind::Info,
                "Standalone egui spike ready (EchoKernel). Press Run or Ctrl+Enter to execute.",
            )],
            active_plot: sample_plot_spec(),
            debug_state: DebugState::Idle,
            breakpoints: Vec::new(),
        }
    }

    /// Borrow the underlying [`scires_app::App`].
    #[must_use]
    pub fn app(&self) -> &App {
        &self.app
    }

    /// Borrow the current [`scires_ui::Layout`].
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Borrow the current [`egui_dock::DockState`].
    #[must_use]
    pub fn dock_state(&self) -> &DockState<DockTab> {
        &self.dock_state
    }

    /// Borrow the current [`scires_ui::Theme`].
    #[must_use]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Current kernel lifecycle status displayed in the toolbar and status strip.
    #[must_use]
    pub fn kernel_status(&self) -> KernelStatus {
        self.kernel_status
    }

    /// Borrow the current editor buffer.
    #[must_use]
    pub fn editor_code(&self) -> &str {
        &self.editor_code
    }

    /// Replace the editor buffer contents.
    pub fn set_editor_code(&mut self, code: impl Into<String>) {
        self.editor_code = code.into();
    }

    /// Borrow ordered console entries.
    #[must_use]
    pub fn console_entries(&self) -> &[ConsoleEntry] {
        &self.console_entries
    }

    /// Borrow the active [`PlotSpec`].
    #[must_use]
    pub fn active_plot(&self) -> &PlotSpec {
        &self.active_plot
    }

    /// Toggle between Dark and Light themes using [`scires_ui::Theme`] tokens.
    pub fn toggle_theme(&mut self) {
        self.theme_mode.toggle();
        self.theme = self.theme_mode.theme();
    }

    /// Toggle visibility of a panel by id in [`scires_ui::Layout`] and rebuild the dock tree.
    pub fn toggle_panel(&mut self, id: &str) {
        if self.layout.toggle(id).is_ok() {
            self.dock_state = build_dock_state(&self.layout);
        }
    }

    /// Execute the current editor buffer through [`scires_app::App::dispatch`].
    pub fn execute_code(&mut self) {
        let code = self.editor_code.clone();
        match self.app.dispatch(Command::Execute {
            language: self.active_language.clone(),
            code,
        }) {
            Ok(Reply::Executed(result)) => {
                self.kernel_status = KernelStatus::Idle;
                for output in result.outputs {
                    match output {
                        Output::Stdout(text) => {
                            self.console_entries
                                .push(ConsoleEntry::new(ConsoleKind::Stdout, text));
                        }
                        Output::Stderr(text) => {
                            self.console_entries
                                .push(ConsoleEntry::new(ConsoleKind::Stderr, text));
                        }
                        Output::Display { mime, data } => {
                            if mime == PLOT_MIME {
                                match PlotSpec::from_json(&data) {
                                    Ok(spec) => {
                                        self.active_plot = spec;
                                        self.console_entries.push(ConsoleEntry::new(
                                            ConsoleKind::Display,
                                            format!("Rendered {PLOT_MIME} in Plots panel"),
                                        ));
                                    }
                                    Err(err) => {
                                        self.console_entries.push(ConsoleEntry::new(
                                            ConsoleKind::Error,
                                            format!("Invalid plot payload ({PLOT_MIME}): {err}"),
                                        ));
                                    }
                                }
                            } else {
                                self.console_entries.push(ConsoleEntry::new(
                                    ConsoleKind::Display,
                                    format!("{mime}: {data}"),
                                ));
                            }
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(err) => {
                self.console_entries.push(ConsoleEntry::new(
                    ConsoleKind::Error,
                    format!("Execution failed ({}): {err}", self.active_language),
                ));
            }
        }
        self.sync_observed_events();
    }

    /// Dispatch [`Command::InterruptKernel`] for the active language.
    pub fn interrupt_kernel(&mut self) {
        match self
            .app
            .dispatch(Command::InterruptKernel(self.active_language.clone()))
        {
            Ok(_) => {
                self.console_entries.push(ConsoleEntry::new(
                    ConsoleKind::Info,
                    format!("Interrupt requested for {} kernel.", self.active_language),
                ));
            }
            Err(err) => {
                self.console_entries.push(ConsoleEntry::new(
                    ConsoleKind::Error,
                    format!("Interrupt failed ({}): {err}", self.active_language),
                ));
            }
        }
        self.sync_observed_events();
    }

    /// Dispatch [`Command::RestartKernel`] for the active language.
    pub fn restart_kernel(&mut self) {
        match self
            .app
            .dispatch(Command::RestartKernel(self.active_language.clone()))
        {
            Ok(_) => {
                self.kernel_status = KernelStatus::Idle;
                self.console_entries.push(ConsoleEntry::new(
                    ConsoleKind::Info,
                    format!(
                        "Restarted {} kernel (in-memory namespace cleared).",
                        self.active_language
                    ),
                ));
            }
            Err(err) => {
                self.console_entries.push(ConsoleEntry::new(
                    ConsoleKind::Error,
                    format!("Restart failed ({}): {err}", self.active_language),
                ));
            }
        }
        self.sync_observed_events();
    }

    /// Toggle a breakpoint in `active_file` at `line` via [`Command::ToggleBreakpoint`].
    pub fn toggle_breakpoint(&mut self, line: u32) {
        let file = self.active_file.clone();
        match self
            .app
            .dispatch(Command::ToggleBreakpoint(PathBuf::from(&file), line))
        {
            Ok(Reply::Breakpoint(enabled)) => {
                if enabled {
                    self.breakpoints.push((file, line));
                } else {
                    self.breakpoints.retain(|(f, l)| !(f == &file && *l == line));
                }
            }
            Ok(_) => {}
            Err(err) => {
                self.console_entries.push(ConsoleEntry::new(
                    ConsoleKind::Error,
                    format!("Breakpoint error: {err}"),
                ));
            }
        }
    }

    fn sync_observed_events(&mut self) {
        if let Ok(guard) = self.observed.lock() {
            if let Some((lang, status)) = &guard.last_kernel_status {
                if lang == &self.active_language {
                    self.kernel_status = parse_kernel_status(status);
                }
            }
            if let Some(state) = &guard.last_debug_state {
                self.debug_state = match state.as_str() {
                    "running" => DebugState::Running,
                    "terminated" => DebugState::Terminated,
                    _ => DebugState::Idle,
                };
            }
        }
    }

    /// Render one full workbench frame into `ctx` (also usable headlessly in tests).
    pub fn ui(&mut self, ctx: &egui::Context) {
        ctx.set_visuals(theme_to_visuals(&self.theme));

        let run_shortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
        if ctx.input_mut(|i| i.consume_shortcut(&run_shortcut)) {
            self.execute_code();
        }

        let mut run_clicked = false;
        let mut interrupt_clicked = false;
        let mut restart_clicked = false;
        let mut theme_clicked = false;
        let mut panel_to_toggle: Option<String> = None;

        egui::TopBottomPanel::top("scires_toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("SciRes");
                ui.separator();

                let (icon, label, color) =
                    kernel_status_badge(self.kernel_status, &self.theme.palette);
                ui.label(
                    RichText::new(format!("{icon} {}: {label}", self.active_language))
                        .color(color)
                        .strong(),
                );
                ui.separator();

                if ui.button("▶ Run (Ctrl+Enter)").clicked() {
                    run_clicked = true;
                }
                if ui.button("■ Interrupt").clicked() {
                    interrupt_clicked = true;
                }
                if ui.button("↻ Restart").clicked() {
                    restart_clicked = true;
                }
                ui.separator();

                ui.menu_button("Panels", |ui| {
                    for (id, label) in [
                        ("files", "Files"),
                        ("editor", "Editor"),
                        ("variables", "Variables"),
                        ("plots", "Plots"),
                        ("console", "Console"),
                        ("debugger", "Debugger"),
                    ] {
                        let visible = self.layout.panel(id).is_some_and(|p| p.visible);
                        let mark = if visible { "[x]" } else { "[ ]" };
                        if ui.button(format!("{mark} {label}")).clicked() {
                            panel_to_toggle = Some(id.to_owned());
                            ui.close_menu();
                        }
                    }
                });

                if ui
                    .button(format!("Theme: {}", self.theme_mode.label()))
                    .clicked()
                {
                    theme_clicked = true;
                }
            });
        });

        egui::TopBottomPanel::bottom("scires_status_strip").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let (k_icon, k_label, k_color) =
                    kernel_status_badge(self.kernel_status, &self.theme.palette);
                ui.label(
                    RichText::new(format!("{k_icon} Kernel ({}): {k_label}", self.active_language))
                        .color(k_color),
                );
                ui.separator();
                ui.label("Env: Standalone EchoKernel (no external Julia required)");
                ui.separator();
                ui.label(format!(
                    "Debugger: {}",
                    debug_state_label(self.debug_state)
                ));
                ui.separator();
                ui.label(format!("Vars: {}", self.app.workspace().len()));
            });
        });

        let mut dock_state = std::mem::replace(&mut self.dock_state, DockState::new(Vec::new()));
        let mut clear_console = false;
        let mut toggle_bp_line: Option<u32> = None;

        {
            let mut viewer = WorkbenchTabViewer {
                app: &self.app,
                palette: &self.theme.palette,
                files: &self.files,
                active_file: &mut self.active_file,
                editor_code: &mut self.editor_code,
                console_entries: &self.console_entries,
                active_plot: &self.active_plot,
                debug_state: self.debug_state,
                breakpoints: &self.breakpoints,
                run_requested: &mut run_clicked,
                clear_console: &mut clear_console,
                toggle_bp_line: &mut toggle_bp_line,
            };

            let style = Style::from_egui(ctx.style().as_ref());
            EguiDockArea::new(&mut dock_state)
                .style(style)
                .show(ctx, &mut viewer);
        }

        self.dock_state = dock_state;

        if clear_console {
            self.console_entries.clear();
        }
        if let Some(line) = toggle_bp_line {
            self.toggle_breakpoint(line);
        }
        if run_clicked {
            self.execute_code();
        }
        if interrupt_clicked {
            self.interrupt_kernel();
        }
        if restart_clicked {
            self.restart_kernel();
        }
        if theme_clicked {
            self.toggle_theme();
        }
        if let Some(id) = panel_to_toggle {
            self.toggle_panel(&id);
        }
    }
}

impl eframe::App for WorkbenchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

struct WorkbenchTabViewer<'a> {
    app: &'a App,
    palette: &'a Palette,
    files: &'a [String],
    active_file: &'a mut String,
    editor_code: &'a mut String,
    console_entries: &'a [ConsoleEntry],
    active_plot: &'a PlotSpec,
    debug_state: DebugState,
    breakpoints: &'a [(String, u32)],
    run_requested: &'a mut bool,
    clear_console: &'a mut bool,
    toggle_bp_line: &'a mut Option<u32>,
}

impl TabViewer for WorkbenchTabViewer<'_> {
    type Tab = DockTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.title.clone().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab.id.as_str() {
            "files" => self.render_files(ui),
            "editor" => self.render_editor(ui),
            "variables" => self.render_variables(ui),
            "plots" => self.render_plots(ui),
            "console" => self.render_console(ui),
            "debugger" => self.render_debugger(ui),
            other => {
                ui.label(format!("Panel `{other}` has no registered view."));
            }
        }
    }
}

impl WorkbenchTabViewer<'_> {
    fn render_files(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Project Files (read-only spike)").strong());
        ui.separator();
        for file in self.files {
            let selected = self.active_file == file;
            if ui.selectable_label(selected, file).clicked() {
                *self.active_file = file.clone();
            }
        }
    }

    fn render_editor(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong(format!("File: {}", self.active_file));
            if ui.button("▶ Run (Ctrl+Enter)").clicked() {
                *self.run_requested = true;
            }
        });
        ui.separator();

        let line_count = self.editor_code.lines().count().max(1);
        let gutter: String = (1..=line_count)
            .map(|n| format!("{n:>3}"))
            .collect::<Vec<_>>()
            .join("\n");

        ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(
                    RichText::new(gutter)
                        .monospace()
                        .color(color_to_egui(self.palette.text_muted)),
                );
                ui.add(
                    TextEdit::multiline(self.editor_code)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(12),
                );
            });
        });
    }

    fn render_variables(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Workspace Variables (read-only)").strong());
        ui.separator();
        let vars = self.app.workspace().list(None);
        if vars.is_empty() {
            ui.label("No variables yet. Run e.g. `x = 42` in the editor.");
            return;
        }
        ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("scires_variables_grid")
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Language");
                    ui.strong("Type");
                    ui.strong("Size");
                    ui.strong("Preview");
                    ui.end_row();

                    for var in vars {
                        let size = var
                            .value
                            .size()
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join("x");
                        ui.monospace(&var.name);
                        ui.label(var.language.to_string());
                        ui.label(var.value.type_label());
                        ui.monospace(size);
                        ui.monospace(value_preview(&var.value));
                        ui.end_row();
                    }
                });
        });
    }

    fn render_plots(&self, ui: &mut egui::Ui) {
        if let Err(err) = render_plot(ui, self.active_plot, self.palette) {
            ui.colored_label(
                color_to_egui(self.palette.danger),
                format!("[error] Plot render failed: {err}"),
            );
        }
    }

    fn render_console(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Console Output");
            if ui.button("Clear").clicked() {
                *self.clear_console = true;
            }
        });
        ui.separator();
        ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in self.console_entries {
                    let color = match entry.kind {
                        ConsoleKind::Stdout => color_to_egui(self.palette.text),
                        ConsoleKind::Stderr | ConsoleKind::Error => {
                            color_to_egui(self.palette.danger)
                        }
                        ConsoleKind::Display => color_to_egui(self.palette.accent),
                        ConsoleKind::Info => color_to_egui(self.palette.text_muted),
                    };
                    ui.label(
                        RichText::new(format!("{} {}", entry.prefix(), entry.text))
                            .monospace()
                            .color(color),
                    );
                }
            });
    }

    fn render_debugger(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong(format!("Session: {}", debug_state_label(self.debug_state)));
            if ui.button("Toggle BP line 1").clicked() {
                *self.toggle_bp_line = Some(1);
            }
        });
        ui.separator();
        ui.small("DAP adapter not attached in standalone spike; displaying BreakpointStore & DebugState.");
        if self.breakpoints.is_empty() {
            ui.label("Breakpoints: none");
        } else {
            for (file, line) in self.breakpoints {
                ui.monospace(format!("● {file}:{line}"));
            }
        }
    }
}

fn kernel_status_badge(status: KernelStatus, palette: &Palette) -> (&'static str, &'static str, Color32) {
    match status {
        KernelStatus::NotStarted => ("○", "Not started", color_to_egui(palette.text_muted)),
        KernelStatus::Starting => ("◐", "Starting", color_to_egui(palette.warning)),
        KernelStatus::Handshaking => ("◑", "Handshaking", color_to_egui(palette.warning)),
        KernelStatus::Idle => ("●", "Ready", color_to_egui(palette.success)),
        KernelStatus::Busy => ("◉", "Running", color_to_egui(palette.warning)),
        KernelStatus::Interrupting => ("◆", "Interrupting", color_to_egui(palette.warning)),
        KernelStatus::Restarting => ("↻", "Restarting", color_to_egui(palette.warning)),
        KernelStatus::Failed => ("▲", "Failed", color_to_egui(palette.danger)),
        KernelStatus::Stopped => ("■", "Stopped", color_to_egui(palette.text_muted)),
    }
}

fn parse_kernel_status(label: &str) -> KernelStatus {
    match label {
        "starting" => KernelStatus::Starting,
        "handshaking" => KernelStatus::Handshaking,
        "idle" => KernelStatus::Idle,
        "busy" => KernelStatus::Busy,
        "interrupting" => KernelStatus::Interrupting,
        "restarting" => KernelStatus::Restarting,
        "failed" => KernelStatus::Failed,
        "stopped" => KernelStatus::Stopped,
        _ => KernelStatus::NotStarted,
    }
}

fn debug_state_label(state: DebugState) -> &'static str {
    match state {
        DebugState::Idle => "○ Idle",
        DebugState::Running => "◉ Running",
        DebugState::Paused(_) => "⏸ Paused",
        DebugState::Terminated => "■ Terminated",
    }
}

fn value_preview(value: &Value) -> String {
    match value {
        Value::Nil => "nothing".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format!("{f:.4}"),
        Value::Complex { re, im } => format!("{re} + {im}im"),
        Value::Str(s) => format!("\"{s}\""),
        Value::Array { preview, .. } => format!("[{}]", preview.join(", ")),
        Value::Table { columns, rows } => format!("{rows}x{} table", columns.len()),
        Value::Opaque { summary, .. } => summary.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_workbench_renders_and_executes_code() {
        let mut wb = WorkbenchApp::new();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        // Render an initial full frame with all 6 docked panels, toolbar, status strip, and plot.
        let initial_out = ctx.run(egui::RawInput::default(), |ctx| {
            wb.ui(ctx);
        });
        assert!(!initial_out.shapes.is_empty());
        assert!(
            initial_out.platform_output.accesskit_update.is_some(),
            "AccessKit tree update should be emitted when enabled"
        );
        assert_eq!(wb.kernel_status(), KernelStatus::NotStarted);

        // Execute code through App and verify workspace + console + kernel status update.
        wb.set_editor_code("alpha = 7");
        wb.execute_code();

        assert_eq!(wb.kernel_status(), KernelStatus::Idle);
        assert_eq!(
            wb.app().workspace().get(&Language::Julia, "alpha"),
            Some(&Value::Int(7))
        );
        assert!(wb
            .console_entries()
            .iter()
            .any(|e| e.kind == ConsoleKind::Stdout && e.text.contains("alpha = 7")));

        // Render a second frame after theme toggle and execution to ensure updated state draws cleanly.
        wb.toggle_theme();
        let second_out = ctx.run(egui::RawInput::default(), |ctx| {
            wb.ui(ctx);
        });
        assert!(!second_out.shapes.is_empty());
        assert_eq!(wb.theme().name, "SciRes Light");
    }

    #[test]
    fn restart_clears_workspace_and_records_info() {
        let mut wb = WorkbenchApp::new();
        wb.set_editor_code("x = 10");
        wb.execute_code();
        assert_eq!(wb.app().workspace().len(), 1);

        wb.restart_kernel();
        assert!(wb.app().workspace().is_empty());
        assert!(wb
            .console_entries()
            .iter()
            .any(|e| e.kind == ConsoleKind::Info && e.text.contains("Restarted")));
    }

    #[test]
    fn toggle_breakpoint_tracks_active_file() {
        let mut wb = WorkbenchApp::new();
        wb.toggle_breakpoint(5);
        assert_eq!(wb.breakpoints, vec![("analysis.jl".to_string(), 5)]);
        wb.toggle_breakpoint(5);
        assert!(wb.breakpoints.is_empty());
    }
}
