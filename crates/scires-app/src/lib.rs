//! Application layer: use cases invoked by the UI (command pattern).
//!
//! The UI never touches domain crates directly; it sends [`Command`]s to [`App`] and
//! receives [`Reply`] values while subscribing to domain events via [`App::events`].
//!
//! # Examples
//!
//! ```
//! use std::sync::Arc;
//! use scires_app::{App, Command};
//! use scires_core::{Language, Value};
//! use scires_kernel::mock::EchoKernelFactory;
//!
//! let mut app = App::default();
//! app.register_kernel(Arc::new(EchoKernelFactory::new(Language::Julia)));
//! app.dispatch(Command::Execute {
//!     language: Language::Julia,
//!     code: "x = 42".into(),
//! })?;
//!
//! assert_eq!(app.workspace().get(&Language::Julia, "x"), Some(&Value::Int(42)));
//! # Ok::<(), scires_app::AppError>(())
//! ```

use scires_core::{EventBus, InMemoryEventBus, Language, Workspace};
use scires_debug::BreakpointStore;
use scires_kernel::{ExecuteRequest, ExecuteResult, KernelError, KernelFactory, KernelManager};
use scires_plugin::{ActivationEvent, PluginError, PluginManifest, PluginRegistry};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

/// Host version, used for plugin compatibility checks.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Application errors.
#[derive(Debug, Error)]
pub enum AppError {
    /// Kernel error.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// Plugin error.
    #[error(transparent)]
    Plugin(#[from] PluginError),
    /// Debugger error.
    #[error(transparent)]
    Debug(#[from] scires_debug::DebugError),
    /// Workspace error.
    #[error(transparent)]
    Workspace(#[from] scires_core::WorkspaceError),
}

/// User intents.
#[derive(Debug, Clone)]
pub enum Command {
    /// Run code in a language (console / "Run selection").
    Execute {
        /// Target language.
        language: Language,
        /// Source code.
        code: String,
    },
    /// Run a file; language detected from its extension.
    RunFile(PathBuf, String),
    /// Toggle a breakpoint.
    ToggleBreakpoint(PathBuf, u32),
    /// Clear workspace (optionally a single language).
    ClearWorkspace(Option<Language>),
    /// Restart a kernel.
    RestartKernel(Language),
    /// Install a plugin from manifest text.
    InstallPlugin(String),
}

/// Result of a command.
#[derive(Debug)]
pub enum Reply {
    /// Nothing to report.
    Ok,
    /// Execution result.
    Executed(ExecuteResult),
    /// Breakpoint is now set (`true`) or removed.
    Breakpoint(bool),
    /// Installed plugin id.
    PluginInstalled(String),
}

/// The application facade.
pub struct App {
    events: Arc<dyn EventBus>,
    workspace: Workspace,
    kernels: KernelManager,
    plugins: PluginRegistry,
    breakpoints: BreakpointStore,
}

impl Default for App {
    fn default() -> Self {
        Self::new(Arc::new(InMemoryEventBus::new()))
    }
}

impl App {
    /// Create the application with an event bus.
    pub fn new(events: Arc<dyn EventBus>) -> Self {
        Self {
            workspace: Workspace::new(events.clone()),
            kernels: KernelManager::new(events.clone()),
            plugins: PluginRegistry::new(VERSION.parse().expect("valid crate version")),
            breakpoints: BreakpointStore::new(),
            events,
        }
    }

    /// Event bus for UI subscription.
    pub fn events(&self) -> &Arc<dyn EventBus> {
        &self.events
    }

    /// Read-only workspace.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Plugin registry.
    pub fn plugins(&self) -> &PluginRegistry {
        &self.plugins
    }

    /// Register a kernel factory (normally done by a language plugin on activation).
    pub fn register_kernel(&mut self, factory: Arc<dyn KernelFactory>) {
        self.kernels.register(factory);
    }

    /// Mark enabled plugins interested in `event` as activated, in dependency order.
    fn fire(&mut self, event: &ActivationEvent) -> Result<(), AppError> {
        for id in self.plugins.activation_order()? {
            if self
                .plugins
                .manifest(&id)
                .is_some_and(|m| m.activates_on(event))
            {
                self.plugins.mark_activated(&id, Ok(()))?;
            }
        }
        Ok(())
    }

    fn execute(&mut self, language: Language, code: String) -> Result<Reply, AppError> {
        self.fire(&ActivationEvent::OnLanguage(language.clone()))?;
        let res = self.kernels.execute(&language, &ExecuteRequest::new(code))?;
        for v in self.kernels.variables(&language)? {
            self.workspace.set(v)?;
        }
        Ok(Reply::Executed(res))
    }

    /// Dispatch a command.
    ///
    /// # Errors
    ///
    /// Propagates [`AppError`] from the underlying kernel, plugin registry,
    /// breakpoint store, or workspace.
    pub fn dispatch(&mut self, cmd: Command) -> Result<Reply, AppError> {
        match cmd {
            Command::Execute { language, code } => self.execute(language, code),
            Command::RunFile(path, code) => {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let lang = Language::from_extension(ext).unwrap_or(Language::Julia);
                self.execute(lang, code)
            }
            Command::ToggleBreakpoint(p, l) => Ok(Reply::Breakpoint(self.breakpoints.toggle(p, l)?)),
            Command::ClearWorkspace(l) => {
                self.workspace.clear(l.as_ref());
                Ok(Reply::Ok)
            }
            Command::RestartKernel(l) => {
                self.kernels.restart(&l)?;
                self.workspace.clear(Some(&l));
                Ok(Reply::Ok)
            }
            Command::InstallPlugin(src) => {
                let m = PluginManifest::from_toml(&src)?;
                let id = m.id.clone();
                self.plugins.install(m)?;
                Ok(Reply::PluginInstalled(id))
            }
        }
    }

    /// Graceful shutdown.
    pub fn shutdown(&mut self) -> Result<(), AppError> {
        self.kernels.shutdown_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scires_core::Value;
    use scires_kernel::mock::EchoKernelFactory;
    use scires_plugin::PluginState;

    #[test]
    fn polyglot_execution_updates_workspace() {
        let mut app = App::default();
        app.register_kernel(Arc::new(EchoKernelFactory::new(Language::Julia)));
        app.register_kernel(Arc::new(EchoKernelFactory::new(Language::Python)));

        app.dispatch(Command::Execute {
            language: Language::Julia,
            code: "x = 1".into(),
        })
        .unwrap();
        app.dispatch(Command::RunFile("train.py".into(), "y = 2".into()))
            .unwrap();

        assert_eq!(app.workspace().get(&Language::Julia, "x"), Some(&Value::Int(1)));
        assert_eq!(app.workspace().get(&Language::Python, "y"), Some(&Value::Int(2)));

        app.dispatch(Command::RestartKernel(Language::Python)).unwrap();
        assert_eq!(app.workspace().len(), 1);
    }

    #[test]
    fn plugin_activates_lazily_on_language() {
        let mut app = App::default();
        app.register_kernel(Arc::new(EchoKernelFactory::new(Language::R)));
        let manifest = format!(
            "id = \"scires.r\"\nname = \"R\"\nversion = \"0.1.0\"\nscires = \"^{VERSION}\"\nactivation = [\"onLanguage:r\"]"
        );
        app.dispatch(Command::InstallPlugin(manifest)).unwrap();
        assert_eq!(app.plugins().state("scires.r"), Some(&PluginState::Enabled));
        app.dispatch(Command::Execute {
            language: Language::R,
            code: "1".into(),
        })
        .unwrap();
        assert_eq!(app.plugins().state("scires.r"), Some(&PluginState::Active));
    }
}
