#![doc = include_str!("../README.md")]

pub mod chart;
pub mod error;
mod notes;
mod opc;
pub mod picture;
pub mod presentation;
#[cfg(feature = "render")]
pub mod render;
pub mod shape;
mod slide;
pub mod table;
mod text;
mod xml;

pub use chart::{Categories, ChartData, Series, XySeries};
pub use error::{Error, Result};
pub use picture::Fit;
pub use presentation::{Presentation, ShapeRef, SlideId};
pub use shape::{Placeholder, ShapeInfo, ShapeKind};
