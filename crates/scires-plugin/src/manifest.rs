use crate::PluginError;
use scires_core::Language;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;

/// When a plugin should be activated (lazy loading).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ActivationEvent {
    /// At startup (use sparingly).
    OnStartup,
    /// When a language kernel/file is first used.
    OnLanguage(Language),
    /// When a command is invoked.
    OnCommand(String),
}

impl FromStr for ActivationEvent {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once(':') {
            None if s == "onStartup" => Ok(Self::OnStartup),
            Some(("onLanguage", l)) if !l.is_empty() => Ok(Self::OnLanguage(l.parse().unwrap())),
            Some(("onCommand", c)) if !c.is_empty() => Ok(Self::OnCommand(c.to_owned())),
            _ => Err(format!("unknown activation event `{s}`")),
        }
    }
}

impl TryFrom<String> for ActivationEvent {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<ActivationEvent> for String {
    fn from(e: ActivationEvent) -> Self {
        match e {
            ActivationEvent::OnStartup => "onStartup".into(),
            ActivationEvent::OnLanguage(l) => format!("onLanguage:{l}"),
            ActivationEvent::OnCommand(c) => format!("onCommand:{c}"),
        }
    }
}

/// A command contributed to the command palette.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandContribution {
    /// Unique command id, conventionally prefixed by the plugin id.
    pub id: String,
    /// Human-readable title.
    pub title: String,
}

/// How a plugin's code is built and executed (see ADR-0006).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    /// Manifest-only: contributes commands, views, snippets or themes, no code.
    Declarative,
    /// Rust crate compiled into the host (first-party, trusted).
    #[default]
    Native,
    /// `wasm32-wasip2` component run in a wasmtime sandbox (third-party default).
    Wasm,
    /// Separate executable speaking JSON-RPC over stdio (any language).
    Process,
}

/// A dockable view contributed to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewContribution {
    /// View id, conventionally prefixed by the plugin id.
    pub id: String,
    /// Tab title.
    pub title: String,
    /// Preferred dock area: `left`, `center`, `right` or `bottom`.
    pub area: String,
}

/// What a plugin adds to the IDE.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Contributions {
    /// Language kernels provided.
    pub kernels: Vec<Language>,
    /// Debug adapters provided, per language.
    pub debuggers: Vec<Language>,
    /// Commands.
    pub commands: Vec<CommandContribution>,
    /// Plot renderers / visualizers by id.
    pub visualizers: Vec<String>,
    /// File extensions the plugin can import as data (e.g. "mat", "h5", "csv").
    pub importers: Vec<String>,
    /// Dockable views (panels).
    pub views: Vec<ViewContribution>,
}

/// Contents of `scires-plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// Reverse-DNS-like id, e.g. `scires.python`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Plugin version.
    pub version: Version,
    /// Compatible host versions.
    pub scires: VersionReq,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// How the plugin is built and run. Defaults to [`Runtime::Native`].
    #[serde(default)]
    pub runtime: Runtime,
    /// Entry point relative to the plugin directory (`.wasm` file or executable).
    /// Required for `wasm` and `process`, forbidden for `declarative`.
    #[serde(default)]
    pub entry: Option<String>,
    /// Activation events. Empty means activate on startup.
    #[serde(default)]
    pub activation: Vec<ActivationEvent>,
    /// Contributions.
    #[serde(default)]
    pub contributes: Contributions,
    /// Other plugins required, with version ranges.
    #[serde(default)]
    pub dependencies: BTreeMap<String, VersionReq>,
}

impl PluginManifest {
    /// Parse and validate a TOML manifest.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Parse`] on TOML syntax/schema errors, or
    /// [`PluginError::Invalid`] if semantic validation fails.
    pub fn from_toml(src: &str) -> Result<Self, PluginError> {
        let m: Self = toml::from_str(src).map_err(|e| PluginError::Parse(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    /// Semantic validation.
    pub fn validate(&self) -> Result<(), PluginError> {
        let valid_id = !self.id.is_empty()
            && self.id.split('.').all(|seg| {
                !seg.is_empty()
                    && seg.starts_with(|c: char| c.is_ascii_lowercase())
                    && seg
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            });
        if !valid_id {
            return Err(PluginError::Invalid {
                field: "id",
                reason: "use lowercase dot-separated segments, e.g. `acme.signal-tools`".into(),
            });
        }
        if self.name.trim().is_empty() {
            return Err(PluginError::Invalid {
                field: "name",
                reason: "must not be empty".into(),
            });
        }
        if self.dependencies.contains_key(&self.id) {
            return Err(PluginError::Invalid {
                field: "dependencies",
                reason: "a plugin cannot depend on itself".into(),
            });
        }
        let mut seen = std::collections::HashSet::new();
        for c in &self.contributes.commands {
            if !seen.insert(&c.id) {
                return Err(PluginError::Invalid {
                    field: "contributes.commands",
                    reason: format!("duplicate command `{}`", c.id),
                });
            }
        }
        match (self.runtime, &self.entry) {
            (Runtime::Wasm | Runtime::Process, None) => {
                return Err(PluginError::Invalid {
                    field: "entry",
                    reason: "required for `wasm` and `process` runtimes".into(),
                });
            }
            (Runtime::Declarative, Some(_)) => {
                return Err(PluginError::Invalid {
                    field: "entry",
                    reason: "declarative plugins have no code entry point".into(),
                });
            }
            (Runtime::Wasm, Some(e)) if !e.ends_with(".wasm") => {
                return Err(PluginError::Invalid {
                    field: "entry",
                    reason: "wasm entry must be a `.wasm` file".into(),
                });
            }
            _ => {}
        }
        for v in &self.contributes.views {
            if !matches!(v.area.as_str(), "left" | "center" | "right" | "bottom") {
                return Err(PluginError::Invalid {
                    field: "contributes.views",
                    reason: format!("view `{}` has unknown area `{}`", v.id, v.area),
                });
            }
        }
        Ok(())
    }

    /// Whether this plugin should activate for `event`.
    pub fn activates_on(&self, event: &ActivationEvent) -> bool {
        self.activation.is_empty()
            || self.activation.contains(&ActivationEvent::OnStartup)
            || self.activation.contains(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PY: &str = r#"
        id = "scires.python"
        name = "Python support"
        version = "0.1.0"
        scires = "^0.1"
        activation = ["onLanguage:python"]

        [contributes]
        kernels = ["python"]
        commands = [{ id = "python.selectInterpreter", title = "Select interpreter" }]

        [dependencies]
        "scires.arrow" = "^0.1"
    "#;

    #[test]
    fn parses_manifest() {
        let m = PluginManifest::from_toml(PY).unwrap();
        assert_eq!(m.contributes.kernels, vec![Language::Python]);
        assert!(m.activates_on(&ActivationEvent::OnLanguage(Language::Python)));
        assert!(!m.activates_on(&ActivationEvent::OnLanguage(Language::R)));
        assert_eq!(m.dependencies.len(), 1);
    }

    #[test]
    fn rejects_bad_id() {
        let src = PY.replace("scires.python", "Scires..Python");
        assert!(matches!(
            PluginManifest::from_toml(&src),
            Err(PluginError::Invalid { field: "id", .. })
        ));
    }

    #[test]
    fn rejects_unknown_fields_and_events() {
        assert!(PluginManifest::from_toml(&format!("{PY}\nfoo = 1")).is_err());
        let src = PY.replace("onLanguage:python", "onWhatever");
        assert!(PluginManifest::from_toml(&src).is_err());
    }

    fn with_header(extra: &str) -> String {
        PY.replacen("activation", &format!("{extra}\n        activation"), 1)
    }

    #[test]
    fn runtime_defaults_to_native() {
        let m = PluginManifest::from_toml(PY).unwrap();
        assert_eq!(m.runtime, Runtime::Native);
    }

    #[test]
    fn wasm_and_process_require_entry() {
        for rt in ["wasm", "process"] {
            let src = with_header(&format!("runtime = \"{rt}\""));
            assert!(matches!(
                PluginManifest::from_toml(&src),
                Err(PluginError::Invalid { field: "entry", .. })
            ));
        }
        let ok = with_header("runtime = \"wasm\"\nentry = \"plugin.wasm\"");
        assert!(PluginManifest::from_toml(&ok).is_ok());
        let bad = with_header("runtime = \"wasm\"\nentry = \"plugin.exe\"");
        assert!(PluginManifest::from_toml(&bad).is_err());
    }

    #[test]
    fn declarative_forbids_entry() {
        let src = with_header("runtime = \"declarative\"\nentry = \"x\"");
        assert!(PluginManifest::from_toml(&src).is_err());
    }

    #[test]
    fn validates_view_area() {
        let ok = format!(
            "{PY}\n[[contributes.views]]\nid = \"py.env\"\ntitle = \"Env\"\narea = \"left\""
        )
        .replace("[dependencies]\n        \"scires.arrow\" = \"^0.1\"", "");
        assert_eq!(
            PluginManifest::from_toml(&ok).unwrap().contributes.views.len(),
            1
        );
        let bad = ok.replace("\"left\"", "\"top\"");
        assert!(matches!(
            PluginManifest::from_toml(&bad),
            Err(PluginError::Invalid {
                field: "contributes.views",
                ..
            })
        ));
    }
}
