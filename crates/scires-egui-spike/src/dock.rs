//! Mapping from [`scires_ui::Layout`] to an [`egui_dock::DockState`].

use egui_dock::{DockState, NodeIndex};
use scires_ui::{DockArea, Layout, Panel};

/// Tab entry stored in [`egui_dock::DockState`], mirroring a visible [`scires_ui::Panel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockTab {
    /// Stable panel identifier (e.g. `"editor"`, `"console"`).
    pub id: String,
    /// Human-readable tab title.
    pub title: String,
}

impl From<&Panel> for DockTab {
    fn from(panel: &Panel) -> Self {
        Self {
            id: panel.id.clone(),
            title: panel.title.clone(),
        }
    }
}

/// Build an [`egui_dock::DockState`] from the visible panels in `layout`.
///
/// The scientific arrangement places:
/// - [`DockArea::Center`] in the central anchor pane,
/// - [`DockArea::Left`] on the left sidebar (20% width),
/// - [`DockArea::Right`] on the right sidebar (28% of remaining width),
/// - [`DockArea::Bottom`] below the central pane (32% height).
#[must_use]
pub fn build_dock_state(layout: &Layout) -> DockState<DockTab> {
    let center: Vec<DockTab> = layout
        .panels_in(DockArea::Center)
        .map(DockTab::from)
        .collect();
    let left: Vec<DockTab> = layout.panels_in(DockArea::Left).map(DockTab::from).collect();
    let right: Vec<DockTab> = layout
        .panels_in(DockArea::Right)
        .map(DockTab::from)
        .collect();
    let bottom: Vec<DockTab> = layout
        .panels_in(DockArea::Bottom)
        .map(DockTab::from)
        .collect();

    if center.is_empty() && left.is_empty() && right.is_empty() && bottom.is_empty() {
        return DockState::new(Vec::new());
    }

    // Seed the root with center panels when present, otherwise promote the first non-empty area.
    let (mut state, mut center_node, left_rem, right_rem, bottom_rem) = if !center.is_empty() {
        (
            DockState::new(center),
            NodeIndex::root(),
            left,
            right,
            bottom,
        )
    } else if !left.is_empty() {
        (
            DockState::new(left),
            NodeIndex::root(),
            Vec::new(),
            right,
            bottom,
        )
    } else if !right.is_empty() {
        (
            DockState::new(right),
            NodeIndex::root(),
            Vec::new(),
            Vec::new(),
            bottom,
        )
    } else {
        (
            DockState::new(bottom),
            NodeIndex::root(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    };

    let surface = state.main_surface_mut();
    if !left_rem.is_empty() {
        let [old_center, _left_node] = surface.split_left(center_node, 0.20, left_rem);
        center_node = old_center;
    }
    if !right_rem.is_empty() {
        let [old_center, _right_node] = surface.split_right(center_node, 0.72, right_rem);
        center_node = old_center;
    }
    if !bottom_rem.is_empty() {
        let _ = surface.split_below(center_node, 0.68, bottom_rem);
    }

    state
}

/// Collect the panel identifiers currently present across all tabs in `state`.
#[must_use]
pub fn dock_tab_ids(state: &DockState<DockTab>) -> Vec<String> {
    state
        .iter_all_tabs()
        .map(|(_, tab)| tab.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientific_layout_maps_all_six_panels() {
        let layout = Layout::scientific();
        let dock = build_dock_state(&layout);
        let ids = dock_tab_ids(&dock);

        assert_eq!(ids.len(), 6);
        for expected in ["files", "editor", "variables", "plots", "console", "debugger"] {
            assert!(
                ids.iter().any(|id| id == expected),
                "missing panel `{expected}` in dock state: {ids:?}"
            );
        }
    }

    #[test]
    fn hidden_panels_are_omitted_from_dock_state() {
        let mut layout = Layout::scientific();
        assert!(!layout.toggle("plots").unwrap());
        assert!(!layout.toggle("debugger").unwrap());

        let dock = build_dock_state(&layout);
        let ids = dock_tab_ids(&dock);
        assert_eq!(ids.len(), 4);
        assert!(!ids.iter().any(|id| id == "plots"));
        assert!(!ids.iter().any(|id| id == "debugger"));
    }
}
