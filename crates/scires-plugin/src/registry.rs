use crate::{PluginError, PluginManifest};
use semver::Version;
use std::collections::{BTreeMap, HashSet};

/// Plugin lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Installed but disabled by the user.
    Disabled,
    /// Enabled, waiting for an activation event.
    Enabled,
    /// Activated and running.
    Active,
    /// Activation failed.
    Failed(String),
}

#[derive(Debug)]
struct Entry {
    manifest: PluginManifest,
    state: PluginState,
}

/// Registry of installed plugins. Pure business logic: no file system or loading.
#[derive(Debug)]
pub struct PluginRegistry {
    host_version: Version,
    plugins: BTreeMap<String, Entry>,
}

impl PluginRegistry {
    /// Registry for a host of the given version.
    pub fn new(host_version: Version) -> Self {
        Self {
            host_version,
            plugins: BTreeMap::new(),
        }
    }

    /// Install a plugin (initially [`PluginState::Enabled`]). Checks host compatibility.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Invalid`] if the manifest is invalid,
    /// [`PluginError::AlreadyInstalled`] if the id is already registered, or
    /// [`PluginError::IncompatibleHost`] if the host version does not match.
    pub fn install(&mut self, manifest: PluginManifest) -> Result<(), PluginError> {
        manifest.validate()?;
        if self.plugins.contains_key(&manifest.id) {
            return Err(PluginError::AlreadyInstalled(manifest.id));
        }
        if !manifest.scires.matches(&self.host_version) {
            return Err(PluginError::IncompatibleHost {
                id: manifest.id,
                required: manifest.scires.to_string(),
                host: self.host_version.to_string(),
            });
        }
        self.plugins.insert(
            manifest.id.clone(),
            Entry {
                manifest,
                state: PluginState::Enabled,
            },
        );
        Ok(())
    }

    /// Uninstall a plugin. Fails if enabled plugins depend on it.
    pub fn uninstall(&mut self, id: &str) -> Result<PluginManifest, PluginError> {
        self.ensure_no_dependents(id)?;
        self.plugins
            .remove(id)
            .map(|e| e.manifest)
            .ok_or_else(|| PluginError::NotFound(id.to_owned()))
    }

    /// Manifest of an installed plugin.
    pub fn manifest(&self, id: &str) -> Option<&PluginManifest> {
        self.plugins.get(id).map(|e| &e.manifest)
    }

    /// State of an installed plugin.
    pub fn state(&self, id: &str) -> Option<&PluginState> {
        self.plugins.get(id).map(|e| &e.state)
    }

    /// Ids of all installed plugins.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.plugins.keys().map(String::as_str)
    }

    fn entry_mut(&mut self, id: &str) -> Result<&mut Entry, PluginError> {
        self.plugins
            .get_mut(id)
            .ok_or_else(|| PluginError::NotFound(id.to_owned()))
    }

    fn is_on(&self, id: &str) -> bool {
        matches!(
            self.state(id),
            Some(PluginState::Enabled | PluginState::Active)
        )
    }

    fn enabled_dependents(&self, id: &str) -> Vec<String> {
        self.plugins
            .values()
            .filter(|e| e.manifest.dependencies.contains_key(id) && self.is_on(&e.manifest.id))
            .map(|e| e.manifest.id.clone())
            .collect()
    }

    fn ensure_no_dependents(&self, id: &str) -> Result<(), PluginError> {
        let dependents = self.enabled_dependents(id);
        if dependents.is_empty() {
            Ok(())
        } else {
            Err(PluginError::HasDependents {
                id: id.to_owned(),
                dependents,
            })
        }
    }

    /// Enable a plugin; all its dependencies must be installed, compatible and enabled.
    pub fn enable(&mut self, id: &str) -> Result<(), PluginError> {
        let deps = self
            .manifest(id)
            .ok_or_else(|| PluginError::NotFound(id.to_owned()))?
            .dependencies
            .clone();
        for (dep, req) in deps {
            let ok = self
                .manifest(&dep)
                .is_some_and(|m| req.matches(&m.version))
                && self.is_on(&dep);
            if !ok {
                return Err(PluginError::MissingDependency {
                    id: id.to_owned(),
                    dependency: dep,
                    required: req.to_string(),
                });
            }
        }
        let e = self.entry_mut(id)?;
        if matches!(e.state, PluginState::Disabled | PluginState::Failed(_)) {
            e.state = PluginState::Enabled;
        }
        Ok(())
    }

    /// Disable a plugin. Fails if enabled plugins depend on it.
    pub fn disable(&mut self, id: &str) -> Result<(), PluginError> {
        self.ensure_no_dependents(id)?;
        self.entry_mut(id)?.state = PluginState::Disabled;
        Ok(())
    }

    /// Record activation result (called by the plugin host).
    pub fn mark_activated(&mut self, id: &str, result: Result<(), String>) -> Result<(), PluginError> {
        let e = self.entry_mut(id)?;
        if e.state != PluginState::Enabled {
            return Ok(());
        }
        match result {
            Ok(()) => {
                e.state = PluginState::Active;
                Ok(())
            }
            Err(reason) => {
                e.state = PluginState::Failed(reason.clone());
                Err(PluginError::Activation {
                    id: id.to_owned(),
                    reason,
                })
            }
        }
    }

    /// Activation order for all enabled plugins: dependencies first (topological sort).
    pub fn activation_order(&self) -> Result<Vec<String>, PluginError> {
        fn visit(
            reg: &PluginRegistry,
            id: &str,
            done: &mut HashSet<String>,
            stack: &mut HashSet<String>,
            out: &mut Vec<String>,
        ) -> Result<(), PluginError> {
            if done.contains(id) {
                return Ok(());
            }
            if !stack.insert(id.to_owned()) {
                return Err(PluginError::Cycle(id.to_owned()));
            }
            if let Some(m) = reg.manifest(id) {
                for dep in m.dependencies.keys() {
                    visit(reg, dep, done, stack, out)?;
                }
            }
            stack.remove(id);
            done.insert(id.to_owned());
            if reg.is_on(id) {
                out.push(id.to_owned());
            }
            Ok(())
        }

        let (mut done, mut stack, mut out) = (HashSet::new(), HashSet::new(), Vec::new());
        for id in self.plugins.keys() {
            visit(self, id, &mut done, &mut stack, &mut out)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, deps: &[(&str, &str)]) -> PluginManifest {
        let deps = deps
            .iter()
            .map(|(d, r)| format!("\"{d}\" = \"{r}\""))
            .collect::<Vec<_>>()
            .join("\n");
        PluginManifest::from_toml(&format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\nscires = \"^0.1\"\n[dependencies]\n{deps}"
        ))
        .unwrap()
    }

    fn reg() -> PluginRegistry {
        PluginRegistry::new(Version::new(0, 1, 0))
    }

    #[test]
    fn rejects_incompatible_host() {
        let mut r = PluginRegistry::new(Version::new(1, 0, 0));
        assert!(matches!(
            r.install(manifest("a", &[])),
            Err(PluginError::IncompatibleHost { .. })
        ));
    }

    #[test]
    fn dependency_order_and_protection() {
        let mut r = reg();
        r.install(manifest("ml", &[("arrow", "^0.1")])).unwrap();
        r.install(manifest("arrow", &[])).unwrap();
        assert_eq!(r.activation_order().unwrap(), vec!["arrow", "ml"]);
        assert!(matches!(
            r.disable("arrow"),
            Err(PluginError::HasDependents { .. })
        ));
        r.disable("ml").unwrap();
        r.disable("arrow").unwrap();
        assert!(matches!(
            r.enable("ml"),
            Err(PluginError::MissingDependency { .. })
        ));
    }

    #[test]
    fn detects_cycles() {
        let mut r = reg();
        r.install(manifest("a", &[("b", "*")])).unwrap();
        r.install(manifest("b", &[("a", "*")])).unwrap();
        assert!(matches!(r.activation_order(), Err(PluginError::Cycle(_))));
    }

    #[test]
    fn activation_failure_is_recorded() {
        let mut r = reg();
        r.install(manifest("a", &[])).unwrap();
        assert!(r.mark_activated("a", Err("boom".into())).is_err());
        assert_eq!(r.state("a"), Some(&PluginState::Failed("boom".into())));
        r.enable("a").unwrap();
        r.mark_activated("a", Ok(())).unwrap();
        assert_eq!(r.state("a"), Some(&PluginState::Active));
    }
}
