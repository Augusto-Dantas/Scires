//! Rendering [`scires_viz::PlotSpec`] using `egui_plot` with textual accessibility summaries.

use crate::theme::color_to_egui;
use eframe::egui::{self, Color32};
use egui_plot::{Bar, BarChart, Legend, Line, Plot, PlotPoints, Points};
use scires_ui::Palette;
use scires_viz::{LayerData, Mark, PlotSpec, VizError};

/// Build a default demonstration [`PlotSpec`] combining line and scatter layers.
#[must_use]
pub fn sample_plot_spec() -> PlotSpec {
    let xs: Vec<f64> = (0..=40).map(|i| f64::from(i) * 0.25).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| (-0.15 * x).exp() * x.sin()).collect();

    let sample_x: Vec<f64> = (0..=8).map(|i| f64::from(i) * 1.25).collect();
    let sample_y: Vec<f64> = sample_x
        .iter()
        .map(|&x| (-0.15 * x).exp() * x.sin())
        .collect();

    PlotSpec::plot(xs, ys)
        .label("exp(-0.15x) * sin(x)")
        .layer(LayerData::Xy {
            x: sample_x,
            y: sample_y,
            mark: Mark::Scatter,
        })
        .label("samples")
        .title("Damped Oscillation (PlotSpec)")
        .labels("t (s)", "amplitude")
}

/// Produce a plain-text accessibility summary of a [`PlotSpec`].
///
/// Ensures plot meaning and data bounds remain available without relying on chart colour alone.
#[must_use]
pub fn plot_accessibility_summary(spec: &PlotSpec) -> String {
    let title = spec.title.as_deref().unwrap_or("Untitled plot");
    let x_label = spec.x_axis.label.as_deref().unwrap_or("x");
    let y_label = spec.y_axis.label.as_deref().unwrap_or("y");

    let mut parts = Vec::with_capacity(spec.layers.len());
    for (idx, layer) in spec.layers.iter().enumerate() {
        let name = layer
            .label
            .clone()
            .unwrap_or_else(|| format!("series {}", idx + 1));
        let desc = match &layer.data {
            LayerData::Xy { x, y, mark } => {
                let kind = match mark {
                    Mark::Line => "line",
                    Mark::Scatter => "scatter",
                    Mark::Bar => "bar",
                };
                let range = y_range(y)
                    .map(|(min, max)| format!(", y in [{min:.2}, {max:.2}]"))
                    .unwrap_or_default();
                format!("{name} ({kind}, {} points{range})", x.len().min(y.len()))
            }
            LayerData::Heatmap { z } => {
                let rows = z.len();
                let cols = z.first().map_or(0, Vec::len);
                format!("{name} (heatmap {rows}x{cols})")
            }
        };
        parts.push(desc);
    }

    format!(
        "{title} — axes: {x_label} vs {y_label}; {} layer(s): {}",
        spec.layers.len(),
        parts.join("; ")
    )
}

fn y_range(values: &[f64]) -> Option<(f64, f64)> {
    let mut iter = values.iter().copied().filter(|v| v.is_finite());
    let first = iter.next()?;
    let (min, max) = iter.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)));
    Some((min, max))
}

fn layer_color(index: usize, palette: &Palette) -> Color32 {
    match index % 4 {
        0 => color_to_egui(palette.accent),
        1 => color_to_egui(palette.success),
        2 => color_to_egui(palette.warning),
        _ => color_to_egui(palette.danger),
    }
}

/// Render a validated [`scires_viz::PlotSpec`] into `ui` using `egui_plot`.
///
/// # Errors
///
/// Returns [`VizError`] if `spec.validate()` fails.
pub fn render_plot(ui: &mut egui::Ui, spec: &PlotSpec, palette: &Palette) -> Result<(), VizError> {
    spec.validate()?;

    if let Some(title) = &spec.title {
        ui.heading(title);
    }

    let summary = plot_accessibility_summary(spec);
    ui.small(&summary);

    let mut plot = Plot::new("scires_plot_canvas")
        .allow_zoom(true)
        .allow_drag(true)
        .min_size(egui::vec2(180.0, 140.0));
    if let Some(x_label) = &spec.x_axis.label {
        plot = plot.x_axis_label(x_label.clone());
    }
    if let Some(y_label) = &spec.y_axis.label {
        plot = plot.y_axis_label(y_label.clone());
    }
    if spec.legend {
        plot = plot.legend(Legend::default());
    }

    let mut unsupported_heatmaps = 0usize;
    plot.show(ui, |plot_ui| {
        for (idx, layer) in spec.layers.iter().enumerate() {
            let color = layer_color(idx, palette);
            let name = layer
                .label
                .clone()
                .unwrap_or_else(|| format!("Layer {}", idx + 1));
            match &layer.data {
                LayerData::Xy { x, y, mark } => {
                    let coords: Vec<[f64; 2]> =
                        x.iter().copied().zip(y.iter().copied()).map(|(a, b)| [a, b]).collect();
                    match mark {
                        Mark::Line => {
                            plot_ui.line(Line::new(PlotPoints::from(coords)).name(name).color(color));
                        }
                        Mark::Scatter => {
                            plot_ui.points(
                                Points::new(PlotPoints::from(coords))
                                    .name(name)
                                    .radius(3.5)
                                    .color(color),
                            );
                        }
                        Mark::Bar => {
                            let bars: Vec<Bar> =
                                coords.into_iter().map(|[bx, by]| Bar::new(bx, by)).collect();
                            plot_ui.bar_chart(BarChart::new(bars).name(name).color(color));
                        }
                    }
                }
                LayerData::Heatmap { .. } => {
                    unsupported_heatmaps += 1;
                }
            }
        }
    });

    if unsupported_heatmaps > 0 {
        ui.colored_label(
            color_to_egui(palette.warning),
            format!(
                "[warn] {unsupported_heatmaps} heatmap layer(s) omitted: raster heatmap renderer not wired in egui spike."
            ),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_summary_describes_layers_and_axes() {
        let spec = sample_plot_spec();
        let summary = plot_accessibility_summary(&spec);
        assert!(summary.contains("Damped Oscillation"));
        assert!(summary.contains("t (s) vs amplitude"));
        assert!(summary.contains("2 layer(s)"));
        assert!(summary.contains("line"));
        assert!(summary.contains("scatter"));
    }

    #[test]
    fn render_plot_rejects_empty_spec() {
        let ctx = egui::Context::default();
        let palette = scires_ui::Theme::dark().palette;
        let mut result = Ok(());
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = render_plot(ui, &PlotSpec::default(), &palette);
            });
        });
        assert_eq!(result, Err(VizError::Empty));
    }
}
