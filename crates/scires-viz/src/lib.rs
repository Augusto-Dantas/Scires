//! Declarative plot model (grammar-of-graphics, inspired by Vega-Lite / Makie / ggplot2).
//!
//! Kernels emit a [`PlotSpec`] as MIME `application/vnd.scires.plot+json`; any
//! [`Renderer`] plugin (GPU, SVG, web) can draw it. MATLAB-like helpers (`plot`,
//! `scatter`, `bar`, `heatmap`) keep the common case one line.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// MIME type for serialized plot specs.
pub const PLOT_MIME: &str = "application/vnd.scires.plot+json";

/// Visualization errors.
#[derive(Debug, Error, PartialEq)]
pub enum VizError {
    /// x/y lengths differ.
    #[error("layer {layer}: x has {x} points but y has {y}")]
    LengthMismatch {
        /// Layer index.
        layer: usize,
        /// Length of x.
        x: usize,
        /// Length of y.
        y: usize,
    },
    /// Heatmap matrix is ragged.
    #[error("layer {0}: heatmap rows have different lengths")]
    RaggedMatrix(usize),
    /// Plot has no layers.
    #[error("plot has no layers")]
    Empty,
    /// Serialization failure.
    #[error("serialization: {0}")]
    Serde(String),
}

/// Geometric mark of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mark {
    /// Connected line.
    Line,
    /// Points.
    Scatter,
    /// Bars.
    Bar,
}

/// Data for one layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LayerData {
    /// x/y series.
    Xy {
        /// X values.
        x: Vec<f64>,
        /// Y values.
        y: Vec<f64>,
        /// Mark.
        mark: Mark,
    },
    /// Row-major matrix.
    Heatmap {
        /// Matrix values.
        z: Vec<Vec<f64>>,
    },
}

/// One layer of a plot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Data.
    pub data: LayerData,
    /// Legend label.
    pub label: Option<String>,
    /// CSS-like color.
    pub color: Option<String>,
}

/// Axis scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    /// Linear.
    #[default]
    Linear,
    /// Base-10 logarithmic.
    Log,
}

/// Axis configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Axis {
    /// Label.
    pub label: Option<String>,
    /// Scale.
    pub scale: Scale,
    /// Fixed limits.
    pub limits: Option<(f64, f64)>,
}

/// Complete plot description.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlotSpec {
    /// Title.
    pub title: Option<String>,
    /// Layers, drawn in order.
    pub layers: Vec<Layer>,
    /// X axis.
    pub x_axis: Axis,
    /// Y axis.
    pub y_axis: Axis,
    /// Show legend.
    pub legend: bool,
}

impl PlotSpec {
    fn xy(x: Vec<f64>, y: Vec<f64>, mark: Mark) -> Self {
        Self::default().layer(LayerData::Xy { x, y, mark })
    }

    /// Line plot (MATLAB `plot(x, y)`).
    pub fn plot(x: Vec<f64>, y: Vec<f64>) -> Self {
        Self::xy(x, y, Mark::Line)
    }

    /// Scatter plot.
    pub fn scatter(x: Vec<f64>, y: Vec<f64>) -> Self {
        Self::xy(x, y, Mark::Scatter)
    }

    /// Bar chart.
    pub fn bar(x: Vec<f64>, y: Vec<f64>) -> Self {
        Self::xy(x, y, Mark::Bar)
    }

    /// Heatmap (MATLAB `imagesc`).
    pub fn heatmap(z: Vec<Vec<f64>>) -> Self {
        Self::default().layer(LayerData::Heatmap { z })
    }

    /// Add a layer (MATLAB `hold on`).
    pub fn layer(mut self, data: LayerData) -> Self {
        self.layers.push(Layer {
            data,
            label: None,
            color: None,
        });
        self
    }

    /// Set the title.
    pub fn title(mut self, t: impl Into<String>) -> Self {
        self.title = Some(t.into());
        self
    }

    /// Set axis labels.
    pub fn labels(mut self, x: impl Into<String>, y: impl Into<String>) -> Self {
        self.x_axis.label = Some(x.into());
        self.y_axis.label = Some(y.into());
        self
    }

    /// Label the last layer and enable the legend.
    pub fn label(mut self, l: impl Into<String>) -> Self {
        if let Some(last) = self.layers.last_mut() {
            last.label = Some(l.into());
            self.legend = true;
        }
        self
    }

    /// Validate data consistency.
    pub fn validate(&self) -> Result<(), VizError> {
        if self.layers.is_empty() {
            return Err(VizError::Empty);
        }
        for (i, l) in self.layers.iter().enumerate() {
            match &l.data {
                LayerData::Xy { x, y, .. } if x.len() != y.len() => {
                    return Err(VizError::LengthMismatch {
                        layer: i,
                        x: x.len(),
                        y: y.len(),
                    })
                }
                LayerData::Heatmap { z } => {
                    if let Some(first) = z.first() {
                        if z.iter().any(|r| r.len() != first.len()) {
                            return Err(VizError::RaggedMatrix(i));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Validate and serialize to JSON for transport.
    pub fn to_json(&self) -> Result<String, VizError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|e| VizError::Serde(e.to_string()))
    }

    /// Deserialize and validate.
    pub fn from_json(s: &str) -> Result<Self, VizError> {
        let p: Self = serde_json::from_str(s).map_err(|e| VizError::Serde(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }
}

/// Port implemented by rendering backends (contributed by plugins).
pub trait Renderer {
    /// Output produced (e.g. SVG string, PNG bytes, GPU frame handle).
    type Output;
    /// Backend id, matching a plugin `visualizers` contribution.
    fn id(&self) -> &str;
    /// Render a validated spec.
    fn render(&self, spec: &PlotSpec) -> Result<Self::Output, VizError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_and_roundtrip() {
        let p = PlotSpec::plot(vec![0.0, 1.0], vec![0.0, 1.0])
            .label("linear")
            .title("t")
            .labels("x", "y");
        assert!(p.legend);
        let json = p.to_json().unwrap();
        assert_eq!(PlotSpec::from_json(&json).unwrap(), p);
    }

    #[test]
    fn validation() {
        assert_eq!(PlotSpec::default().validate(), Err(VizError::Empty));
        assert!(matches!(
            PlotSpec::scatter(vec![1.0], vec![]).validate(),
            Err(VizError::LengthMismatch { .. })
        ));
        assert_eq!(
            PlotSpec::heatmap(vec![vec![1.0], vec![1.0, 2.0]]).validate(),
            Err(VizError::RaggedMatrix(0))
        );
    }
}
