#![doc = include_str!("../README.md")]

pub mod chart;
pub mod error;
mod opc;
pub mod presentation;
pub mod shape;
mod slide;
pub mod table;
mod text;
mod xml;

pub use chart::{Categories, ChartData, Series, XySeries};
pub use error::{Error, Result};
pub use presentation::{Presentation, ShapeRef, SlideId};
pub use shape::{Placeholder, ShapeInfo, ShapeKind};
