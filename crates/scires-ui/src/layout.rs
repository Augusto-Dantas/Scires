use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Where a panel is docked in the IDE window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockArea {
    /// Left sidebar (files, outline, plugins).
    Left,
    /// Central editor area.
    Center,
    /// Right sidebar (variables, plots, help).
    Right,
    /// Bottom panel (console, debugger, problems).
    Bottom,
}

/// A dockable view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    /// Unique id, e.g. `console` or `acme.spectrum`.
    pub id: String,
    /// Tab title.
    pub title: String,
    /// Current dock area.
    pub area: DockArea,
    /// Whether the panel is shown.
    pub visible: bool,
}

/// Errors from layout operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UiError {
    /// A panel with this id already exists.
    #[error("panel `{0}` already exists")]
    DuplicatePanel(String),
    /// No panel with this id.
    #[error("panel `{0}` not found")]
    UnknownPanel(String),
    /// The panel id is empty.
    #[error("panel id must not be empty")]
    EmptyId,
}

/// The dock layout of the IDE window. Panel order inside an area is insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    panels: Vec<Panel>,
}

impl Layout {
    /// The default Spyder/RStudio-like scientific layout.
    #[must_use]
    pub fn scientific() -> Self {
        let mut l = Self::default();
        for (id, title, area) in [
            ("files", "Files", DockArea::Left),
            ("editor", "Editor", DockArea::Center),
            ("variables", "Variables", DockArea::Right),
            ("plots", "Plots", DockArea::Right),
            ("console", "Console", DockArea::Bottom),
            ("debugger", "Debugger", DockArea::Bottom),
        ] {
            // Ids are static and unique, so this cannot fail.
            let _ = l.add_panel(id, title, area);
        }
        l
    }

    /// Add a visible panel.
    ///
    /// # Errors
    ///
    /// [`UiError::EmptyId`] for an empty id, [`UiError::DuplicatePanel`] if it exists.
    pub fn add_panel(&mut self, id: &str, title: &str, area: DockArea) -> Result<(), UiError> {
        if id.trim().is_empty() {
            return Err(UiError::EmptyId);
        }
        if self.panel(id).is_some() {
            return Err(UiError::DuplicatePanel(id.into()));
        }
        self.panels.push(Panel {
            id: id.into(),
            title: title.into(),
            area,
            visible: true,
        });
        Ok(())
    }

    /// Look up a panel.
    #[must_use]
    pub fn panel(&self, id: &str) -> Option<&Panel> {
        self.panels.iter().find(|p| p.id == id)
    }

    fn panel_mut(&mut self, id: &str) -> Result<&mut Panel, UiError> {
        self.panels
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| UiError::UnknownPanel(id.into()))
    }

    /// Move a panel to another dock area (appended last).
    ///
    /// # Errors
    ///
    /// [`UiError::UnknownPanel`] if the id does not exist.
    pub fn move_panel(&mut self, id: &str, area: DockArea) -> Result<(), UiError> {
        let idx = self
            .panels
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| UiError::UnknownPanel(id.into()))?;
        let mut p = self.panels.remove(idx);
        p.area = area;
        self.panels.push(p);
        Ok(())
    }

    /// Toggle visibility; returns the new state.
    ///
    /// # Errors
    ///
    /// [`UiError::UnknownPanel`] if the id does not exist.
    pub fn toggle(&mut self, id: &str) -> Result<bool, UiError> {
        let p = self.panel_mut(id)?;
        p.visible = !p.visible;
        Ok(p.visible)
    }

    /// Remove a panel (e.g. when its plugin is disabled).
    ///
    /// # Errors
    ///
    /// [`UiError::UnknownPanel`] if the id does not exist.
    pub fn remove_panel(&mut self, id: &str) -> Result<Panel, UiError> {
        let idx = self
            .panels
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| UiError::UnknownPanel(id.into()))?;
        Ok(self.panels.remove(idx))
    }

    /// Visible panels in an area, in order.
    pub fn panels_in(&self, area: DockArea) -> impl Iterator<Item = &Panel> {
        self.panels
            .iter()
            .filter(move |p| p.area == area && p.visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientific_layout_has_core_panels() {
        let l = Layout::scientific();
        assert_eq!(l.panels_in(DockArea::Bottom).count(), 2);
        assert!(l.panel("editor").is_some());
    }

    #[test]
    fn rejects_duplicate_and_empty_ids() {
        let mut l = Layout::scientific();
        assert_eq!(
            l.add_panel("console", "X", DockArea::Left),
            Err(UiError::DuplicatePanel("console".into()))
        );
        assert_eq!(l.add_panel(" ", "X", DockArea::Left), Err(UiError::EmptyId));
    }

    #[test]
    fn move_appends_to_target_area() {
        let mut l = Layout::scientific();
        l.move_panel("variables", DockArea::Bottom).unwrap();
        let ids: Vec<_> = l.panels_in(DockArea::Bottom).map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["console", "debugger", "variables"]);
    }

    #[test]
    fn toggle_hides_panel() {
        let mut l = Layout::scientific();
        assert!(!l.toggle("plots").unwrap());
        assert!(l.panels_in(DockArea::Right).all(|p| p.id != "plots"));
    }

    #[test]
    fn unknown_panel_errors() {
        let mut l = Layout::default();
        let e = UiError::UnknownPanel("x".into());
        assert_eq!(l.toggle("x"), Err(e.clone()));
        assert_eq!(l.move_panel("x", DockArea::Left), Err(e.clone()));
        assert_eq!(l.remove_panel("x"), Err(e));
    }
}
