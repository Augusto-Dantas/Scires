use crate::DebugError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Unique breakpoint identifier.
pub type BreakpointId = u64;

/// A source breakpoint, optionally conditional (like Spyder's Shift+F12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breakpoint {
    /// Identifier.
    pub id: BreakpointId,
    /// Source file.
    pub source: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// Optional condition expression in the source language.
    pub condition: Option<String>,
    /// Stop only after this many hits.
    pub hit_count: Option<u32>,
    /// Disabled breakpoints are kept but not sent to the adapter.
    pub enabled: bool,
}

/// Session-independent breakpoint storage.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BreakpointStore {
    next_id: BreakpointId,
    items: BTreeMap<BreakpointId, Breakpoint>,
}

impl BreakpointStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a breakpoint. Adding at an existing location returns the existing id.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::InvalidBreakpoint`] if `line` is `0` or if `condition` is empty/whitespace.
    pub fn add(
        &mut self,
        source: impl Into<PathBuf>,
        line: u32,
        condition: Option<String>,
    ) -> Result<BreakpointId, DebugError> {
        if line == 0 {
            return Err(DebugError::InvalidBreakpoint("lines are 1-based".into()));
        }
        if condition.as_deref().is_some_and(|c| c.trim().is_empty()) {
            return Err(DebugError::InvalidBreakpoint("empty condition".into()));
        }
        let source = source.into();
        if let Some(bp) = self
            .items
            .values_mut()
            .find(|b| b.source == source && b.line == line)
        {
            bp.condition = condition;
            return Ok(bp.id);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.items.insert(
            id,
            Breakpoint {
                id,
                source,
                line,
                condition,
                hit_count: None,
                enabled: true,
            },
        );
        Ok(id)
    }

    /// Toggle a breakpoint at a location (editor gutter click). Returns `true` if now set.
    pub fn toggle(&mut self, source: impl Into<PathBuf>, line: u32) -> Result<bool, DebugError> {
        let source = source.into();
        if let Some(id) = self
            .items
            .values()
            .find(|b| b.source == source && b.line == line)
            .map(|b| b.id)
        {
            self.items.remove(&id);
            Ok(false)
        } else {
            self.add(source, line, None).map(|_| true)
        }
    }

    /// Remove by id.
    pub fn remove(&mut self, id: BreakpointId) -> Option<Breakpoint> {
        self.items.remove(&id)
    }

    /// Enable/disable a breakpoint.
    pub fn set_enabled(&mut self, id: BreakpointId, enabled: bool) -> bool {
        self.items
            .get_mut(&id)
            .map(|b| b.enabled = enabled)
            .is_some()
    }

    /// Enabled breakpoints for a source file, ordered by line.
    pub fn active_for(&self, source: &Path) -> Vec<Breakpoint> {
        let mut v: Vec<_> = self
            .items
            .values()
            .filter(|b| b.enabled && b.source == source)
            .cloned()
            .collect();
        v.sort_by_key(|b| b.line);
        v
    }

    /// All distinct source files with breakpoints.
    pub fn sources(&self) -> Vec<PathBuf> {
        let mut v: Vec<_> = self.items.values().map(|b| b.source.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// All breakpoints.
    pub fn all(&self) -> impl Iterator<Item = &Breakpoint> {
        self.items.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_adds_and_removes() {
        let mut s = BreakpointStore::new();
        assert!(s.toggle("a.jl", 3).unwrap());
        assert_eq!(s.active_for(Path::new("a.jl")).len(), 1);
        assert!(!s.toggle("a.jl", 3).unwrap());
        assert!(s.active_for(Path::new("a.jl")).is_empty());
    }

    #[test]
    fn dedups_location_and_validates() {
        let mut s = BreakpointStore::new();
        let a = s.add("a.jl", 1, None).unwrap();
        let b = s.add("a.jl", 1, Some("x > 1".into())).unwrap();
        assert_eq!(a, b);
        assert!(s.add("a.jl", 0, None).is_err());
        assert!(s.add("a.jl", 2, Some("  ".into())).is_err());
    }

    #[test]
    fn disabled_are_not_active() {
        let mut s = BreakpointStore::new();
        let id = s.add("a.jl", 5, None).unwrap();
        s.set_enabled(id, false);
        assert!(s.active_for(Path::new("a.jl")).is_empty());
    }
}
