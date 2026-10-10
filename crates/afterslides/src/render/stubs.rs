//! Content not drawn yet; replaced step by step.

use kurbo::{Affine, Rect};

use super::scene::{Layer, Scene};
use super::shapes::Xfrm;
use crate::xml::Element;

impl<'a> Scene<'a, '_> {
    pub(super) fn text_frame(
        &mut self,
        _chain: &[&'a Element],
        _rect: Rect,
        _transform: Affine,
        _xfrm: Xfrm,
    ) {
    }

    pub(super) fn table(
        &mut self,
        _tbl: &'a Element,
        _layer: Layer<'a>,
        bounds: Rect,
        transform: Affine,
    ) {
        self.unsupported_box(bounds, transform);
    }

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
