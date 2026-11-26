//! The shared, polyglot workspace (MATLAB's "Workspace" panel).

use crate::{DomainEvent, EventBus, Language, Value, Variable};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

/// Workspace errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkspaceError {
    /// Name is not a valid identifier.
    #[error("invalid variable name `{0}`")]
    InvalidName(String),
    /// Variable not found.
    #[error("variable `{name}` not found in {language} workspace")]
    NotFound {
        /// Language namespace.
        language: Language,
        /// Variable name.
        name: String,
    },
}

/// Variables keyed by `(language, name)`; each language has its own namespace.
pub struct Workspace {
    vars: BTreeMap<(Language, String), Value>,
    events: Arc<dyn EventBus>,
}

impl Workspace {
    /// Create an empty workspace publishing to `events`.
    pub fn new(events: Arc<dyn EventBus>) -> Self {
        Self {
            vars: BTreeMap::new(),
            events,
        }
    }

    fn validate_name(name: &str) -> Result<(), WorkspaceError> {
        let mut chars = name.chars();
        let ok = matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
            && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '!');
        if ok {
            Ok(())
        } else {
            Err(WorkspaceError::InvalidName(name.to_owned()))
        }
    }

    /// Insert or update a variable.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::InvalidName`] if `var.name` is not a valid identifier.
    pub fn set(&mut self, var: Variable) -> Result<(), WorkspaceError> {
        Self::validate_name(&var.name)?;
        self.vars
            .insert((var.language.clone(), var.name.clone()), var.value.clone());
        self.events.publish(DomainEvent::VariableChanged(var));
        Ok(())
    }

    /// Get a variable value.
    pub fn get(&self, language: &Language, name: &str) -> Option<&Value> {
        self.vars.get(&(language.clone(), name.to_owned()))
    }

    /// Remove a variable.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::NotFound`] if no variable with `name` exists in `language`.
    pub fn remove(&mut self, language: &Language, name: &str) -> Result<Value, WorkspaceError> {
        let v = self
            .vars
            .remove(&(language.clone(), name.to_owned()))
            .ok_or_else(|| WorkspaceError::NotFound {
                language: language.clone(),
                name: name.to_owned(),
            })?;
        self.events.publish(DomainEvent::VariableRemoved {
            language: language.clone(),
            name: name.to_owned(),
        });
        Ok(v)
    }

    /// List variables, optionally filtered by language, sorted by language then name.
    pub fn list(&self, language: Option<&Language>) -> Vec<Variable> {
        self.vars
            .iter()
            .filter(|((l, _), _)| language.map_or(true, |f| f == l))
            .map(|((l, n), v)| Variable::new(n.clone(), l.clone(), v.clone()))
            .collect()
    }

    /// Clear one language namespace or everything (MATLAB `clear`).
    pub fn clear(&mut self, language: Option<&Language>) {
        match language {
            Some(l) => self.vars.retain(|(k, _), _| k != l),
            None => self.vars.clear(),
        }
        self.events
            .publish(DomainEvent::WorkspaceCleared(language.cloned()));
    }

    /// Number of variables.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// Whether the workspace is empty.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryEventBus;

    fn ws() -> Workspace {
        Workspace::new(Arc::new(InMemoryEventBus::new()))
    }

    #[test]
    fn namespaces_are_per_language() {
        let mut w = ws();
        w.set(Variable::new("x", Language::Julia, Value::Int(1))).unwrap();
        w.set(Variable::new("x", Language::Python, Value::Int(2))).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w.get(&Language::Python, "x"), Some(&Value::Int(2)));
        w.clear(Some(&Language::Julia));
        assert_eq!(w.list(None).len(), 1);
    }

    #[test]
    fn rejects_invalid_names() {
        let mut w = ws();
        let err = w.set(Variable::new("1x", Language::Julia, Value::Nil));
        assert!(matches!(err, Err(WorkspaceError::InvalidName(_))));
    }

    #[test]
    fn remove_missing_fails() {
        let mut w = ws();
        assert!(w.remove(&Language::R, "nope").is_err());
    }
}
