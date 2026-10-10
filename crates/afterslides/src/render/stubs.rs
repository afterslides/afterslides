//! Content not drawn yet; replaced step by step.

use kurbo::{Affine, Rect};

use super::scene::{Layer, Scene};
use crate::xml::Element;

impl<'a> Scene<'a, '_> {
    pub(super) fn chart_frame(
        &mut self,
        _frame: &'a Element,
        _layer: Layer<'a>,
        bounds: Rect,
        transform: Affine,
    ) {
        self.unsupported_box(bounds, transform);
    }
}
