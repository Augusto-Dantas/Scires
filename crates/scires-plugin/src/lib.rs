//! Plug-and-play plugin system (MATLAB toolboxes / VS Code extensions).
//!
//! A plugin is described by a [`PluginManifest`] (`scires-plugin.toml`). The
//! [`PluginRegistry`] validates compatibility, resolves dependencies in topological order,
//! and tracks [`PluginState`] transitions. Runtime behaviour is provided through [`Plugin`]
//! and [`PluginContext`].
//!
//! # Examples
//!
//! ```
//! use semver::Version;
//! use scires_plugin::{PluginManifest, PluginRegistry, PluginState};
//!
//! let manifest = PluginManifest::from_toml(r#"
//!     id = "scires.python"
//!     name = "Python support"
//!     version = "0.1.0"
//!     scires = "^0.1"
//! "#)?;
//!
//! let mut registry = PluginRegistry::new(Version::new(0, 1, 0));
//! registry.install(manifest)?;
//! assert_eq!(registry.state("scires.python"), Some(&PluginState::Enabled));
//! # Ok::<(), scires_plugin::PluginError>(())
//! ```

mod manifest;
mod registry;

pub use manifest::{
    ActivationEvent, CommandContribution, Contributions, PluginManifest, Runtime, ViewContribution,
};
pub use registry::{PluginRegistry, PluginState};

use thiserror::Error;

/// Plugin errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PluginError {
    /// Manifest could not be parsed.
    #[error("invalid manifest: {0}")]
    Parse(String),
    /// Manifest is semantically invalid.
    #[error("invalid manifest field `{field}`: {reason}")]
    Invalid {
        /// Offending field.
        field: &'static str,
        /// Explanation.
        reason: String,
    },
    /// Plugin requires a different host version.
    #[error("plugin `{id}` requires SciRes {required}, host is {host}")]
    IncompatibleHost {
        /// Plugin id.
        id: String,
        /// Required range.
        required: String,
        /// Host version.
        host: String,
    },
    /// Already installed.
    #[error("plugin `{0}` is already installed")]
    AlreadyInstalled(String),
    /// Unknown plugin.
    #[error("plugin `{0}` is not installed")]
    NotFound(String),
    /// Dependency missing or wrong version.
    #[error("plugin `{id}` depends on `{dependency}` {required}")]
    MissingDependency {
        /// Plugin id.
        id: String,
        /// Dependency id.
        dependency: String,
        /// Required range.
        required: String,
    },
    /// Cyclic dependencies.
    #[error("dependency cycle involving `{0}`")]
    Cycle(String),
    /// Other enabled plugins depend on this one.
    #[error("plugin `{id}` is required by: {dependents:?}")]
    HasDependents {
        /// Plugin id.
        id: String,
        /// Enabled dependents.
        dependents: Vec<String>,
    },
    /// Plugin failed during activation.
    #[error("plugin `{id}` failed to activate: {reason}")]
    Activation {
        /// Plugin id.
        id: String,
        /// Reason.
        reason: String,
    },
}

/// Host services offered to a plugin during activation (capability-based API).
pub trait PluginContext {
    /// Register a command handler id contributed by the plugin.
    fn register_command(&mut self, id: &str);
    /// Log a message on behalf of the plugin.
    fn log(&mut self, message: &str);
}

/// Runtime behaviour of a plugin.
pub trait Plugin: Send {
    /// Called once when an activation event fires.
    fn activate(&mut self, ctx: &mut dyn PluginContext) -> Result<(), String>;
    /// Called on disable / shutdown.
    fn deactivate(&mut self) {}
}
