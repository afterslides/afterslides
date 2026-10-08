//! Low-level Python bindings. The public, documented API lives in the
//! `afterslides` Python package, which wraps this module; everything here
//! works with plain slide and shape ids.

use std::path::PathBuf;

use afterslides::{
    Categories, ChartData, Error, Presentation, Series, ShapeInfo, ShapeRef, SlideId, XySeries,
};
use pyo3::exceptions::PyOSError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyBytes;

/// Maps a core error to the exception classes defined in `afterslides.errors`.
fn to_py(err: Error) -> PyErr {
    static MODULE: PyOnceLock<Py<PyModule>> = PyOnceLock::new();
    Python::attach(|py| {
        let class_name = match &err {
            Error::Io(io) => {
                return PyOSError::new_err((io.raw_os_error().unwrap_or(0), io.to_string()));
            }
            Error::NotFound(_) => "NotFoundError",
            Error::InvalidArgument(_) => "InvalidArgumentError",
            Error::Unsupported(_) => "UnsupportedError",
            Error::Zip(_) | Error::Xml(_) | Error::Package(_) | Error::Workbook(_) => {
                "PackageError"
            }
            _ => "AfterslidesError",
        };
        let raise = || -> PyResult<PyErr> {
            let module = MODULE
                .get_or_try_init(py, || py.import("afterslides.errors").map(|m| m.unbind()))?;
            let class = module.bind(py).getattr(class_name)?;
            Ok(PyErr::from_value(class.call1((err.to_string(),))?))
        };
        raise().unwrap_or_else(|e| e)
    })
}

trait IntoPyResult<T> {
    fn py(self) -> PyResult<T>;
}

impl<T> IntoPyResult<T> for afterslides::Result<T> {
    fn py(self) -> PyResult<T> {
        self.map_err(to_py)
    }
}

fn shape(slide: u32, id: u32) -> ShapeRef {
    ShapeRef {
        slide: SlideId(slide),
        id,
    }
}

fn pairs(replacements: &[(String, String)]) -> Vec<(&str, &str)> {
    replacements
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect()
}

/// Shape properties as a tuple-like record.
#[pyclass(module = "afterslides._native", name = "ShapeInfo", frozen, get_all)]
struct PyShapeInfo {
    slide: u32,
    id: u32,
    name: String,
    alt_text: String,
    title: String,
    kind: &'static str,
    placeholder_type: Option<String>,
    placeholder_idx: Option<u32>,
    has_text: bool,
    hidden: bool,
    parent: Option<u32>,
    frame: Option<(i64, i64, i64, i64)>,
}

impl From<ShapeInfo> for PyShapeInfo {
    fn from(s: ShapeInfo) -> Self {
        PyShapeInfo {
            slide: s.shape.slide.0,
            id: s.shape.id,
            name: s.name,
            alt_text: s.alt_text,
            title: s.title,
            kind: s.kind.as_str(),
            placeholder_type: s.placeholder.as_ref().map(|p| p.kind.clone()),
            placeholder_idx: s.placeholder.map(|p| p.idx),
            has_text: s.has_text,
            hidden: s.hidden,
            parent: s.parent,
            frame: s.frame,
        }
    }
}

type PySeries = (String, Vec<Option<f64>>);
type PyXySeries = (
    String,
    Vec<Option<f64>>,
    Vec<Option<f64>>,
    Option<Vec<Option<f64>>>,
);

#[derive(FromPyObject)]
enum PyCategories {
    Numbers(Vec<f64>),
    Labels(Vec<String>),
}

#[pyclass(module = "afterslides._native", name = "Presentation")]
struct PyPresentation {
    inner: Presentation,
}

#[pymethods]
impl PyPresentation {
    #[staticmethod]
    fn open(py: Python<'_>, path: PathBuf) -> PyResult<Self> {
        let inner = py.detach(|| Presentation::open(path)).py()?;
        Ok(PyPresentation { inner })
    }

    #[staticmethod]
    fn from_bytes(py: Python<'_>, data: &[u8]) -> PyResult<Self> {
        // Copy out of the Python buffer before releasing the interpreter.
        let data = data.to_vec();
        let inner = py.detach(move || Presentation::from_bytes(&data)).py()?;
        Ok(PyPresentation { inner })
    }

    fn save(&mut self, py: Python<'_>, path: PathBuf) -> PyResult<()> {
        let inner = &mut self.inner;
        py.detach(move || inner.save(path)).py()
    }

    // Saving needs `&mut`: it drops parts that are no longer referenced.
    #[allow(clippy::wrong_self_convention)]
    fn to_bytes<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let inner = &mut self.inner;
        let bytes = py.detach(move || inner.to_bytes()).py()?;
        Ok(PyBytes::new(py, &bytes))
    }

    fn slide_size(&self) -> PyResult<(i64, i64)> {
        self.inner.slide_size().py()
    }

    // ---- slides ------------------------------------------------------------

    fn slides(&self) -> PyResult<Vec<u32>> {
        Ok(self.inner.slides().py()?.into_iter().map(|s| s.0).collect())
    }

    fn slide_index(&self, slide: u32) -> PyResult<usize> {
        self.inner.slide_index(SlideId(slide)).py()
    }

    fn slide_part(&self, slide: u32) -> PyResult<String> {
        self.inner.slide_part(SlideId(slide)).py()
    }

    fn delete_slide(&mut self, slide: u32) -> PyResult<()> {
        self.inner.delete_slide(SlideId(slide)).py()
    }

    #[pyo3(signature = (slide, position=None))]
    fn duplicate_slide(&mut self, slide: u32, position: Option<usize>) -> PyResult<u32> {
        Ok(self.inner.duplicate_slide(SlideId(slide), position).py()?.0)
    }

    fn move_slide(&mut self, slide: u32, position: usize) -> PyResult<()> {
        self.inner.move_slide(SlideId(slide), position).py()
    }

    #[pyo3(signature = (replacements, slide=None, shape_id=None))]
    fn replace_text(
        &mut self,
        replacements: Vec<(String, String)>,
        slide: Option<u32>,
        shape_id: Option<u32>,
    ) -> PyResult<usize> {
        let pairs = pairs(&replacements);
        match (slide, shape_id) {
            (Some(s), Some(id)) => self.inner.replace_text_in_shape(shape(s, id), &pairs),
            (Some(s), None) => self.inner.replace_text_on_slide(SlideId(s), &pairs),
            _ => self.inner.replace_text(&pairs),
        }
        .py()
    }

    // ---- shapes ------------------------------------------------------------

    fn shapes(&self, slide: u32) -> PyResult<Vec<PyShapeInfo>> {
        Ok(self
            .inner
            .shapes(SlideId(slide))
            .py()?
            .into_iter()
            .map(PyShapeInfo::from)
            .collect())
    }

    fn shape_info(&self, slide: u32, id: u32) -> PyResult<PyShapeInfo> {
        Ok(self.inner.shape_info(shape(slide, id)).py()?.into())
    }

    fn shape_text(&self, slide: u32, id: u32) -> PyResult<Option<String>> {
        self.inner.shape_text(shape(slide, id)).py()
    }

    fn set_shape_text(&mut self, slide: u32, id: u32, text: &str) -> PyResult<()> {
        self.inner.set_shape_text(shape(slide, id), text).py()
    }

    fn delete_shape(&mut self, slide: u32, id: u32) -> PyResult<()> {
        self.inner.delete_shape(shape(slide, id)).py()
    }

    // ---- tables ------------------------------------------------------------

    fn table_values(&self, slide: u32, id: u32) -> PyResult<Vec<Vec<String>>> {
        self.inner.table_values(shape(slide, id)).py()
    }

    fn table_size(&self, slide: u32, id: u32) -> PyResult<(usize, usize)> {
        self.inner.table_size(shape(slide, id)).py()
    }

    fn set_table_cell_text(
        &mut self,
        slide: u32,
        id: u32,
        row: usize,
        col: usize,
        text: &str,
    ) -> PyResult<()> {
        self.inner
            .set_table_cell_text(shape(slide, id), row, col, text)
            .py()
    }

    fn insert_table_row(&mut self, slide: u32, id: u32, source: usize, at: usize) -> PyResult<()> {
        self.inner
            .insert_table_row(shape(slide, id), source, at)
            .py()
    }

    fn delete_table_row(&mut self, slide: u32, id: u32, row: usize) -> PyResult<()> {
        self.inner.delete_table_row(shape(slide, id), row).py()
    }

    fn delete_table_column(&mut self, slide: u32, id: u32, col: usize) -> PyResult<()> {
        self.inner.delete_table_column(shape(slide, id), col).py()
    }

    fn fill_table(
        &mut self,
        slide: u32,
        id: u32,
        data: Vec<Vec<String>>,
        start_row: usize,
        resize: bool,
    ) -> PyResult<()> {
        self.inner
            .fill_table(shape(slide, id), &data, start_row, resize)
            .py()
    }

    // ---- charts ------------------------------------------------------------

    fn chart_types(&self, slide: u32, id: u32) -> PyResult<Vec<String>> {
        self.inner.chart_types(shape(slide, id)).py()
    }

    /// Returns `(categories, numeric, [(name, values)])`.
    fn chart_data<'py>(
        &self,
        py: Python<'py>,
        slide: u32,
        id: u32,
    ) -> PyResult<(Bound<'py, PyAny>, bool, Vec<PySeries>)> {
        let data = self.inner.chart_data(shape(slide, id)).py()?;
        let (categories, numeric) = match data.categories {
            Categories::Labels(v) => (v.into_pyobject(py)?.into_any(), false),
            Categories::Numbers(v) => (v.into_pyobject(py)?.into_any(), true),
        };
        let series = data
            .series
            .into_iter()
            .map(|s| (s.name, s.values))
            .collect();
        Ok((categories, numeric, series))
    }

    fn set_chart_data(
        &mut self,
        slide: u32,
        id: u32,
        categories: PyCategories,
        series: Vec<PySeries>,
    ) -> PyResult<()> {
        let data = ChartData {
            categories: match categories {
                PyCategories::Labels(v) => Categories::Labels(v),
                PyCategories::Numbers(v) => Categories::Numbers(v),
            },
            series: series
                .into_iter()
                .map(|(name, values)| Series { name, values })
                .collect(),
        };
        self.inner.set_chart_data(shape(slide, id), &data).py()
    }

    fn chart_xy_data(&self, slide: u32, id: u32) -> PyResult<Vec<PyXySeries>> {
        Ok(self
            .inner
            .chart_xy_data(shape(slide, id))
            .py()?
            .into_iter()
            .map(|s| (s.name, s.x, s.y, s.sizes))
            .collect())
    }

    fn set_chart_xy_data(&mut self, slide: u32, id: u32, series: Vec<PyXySeries>) -> PyResult<()> {
        let series: Vec<XySeries> = series
            .into_iter()
            .map(|(name, x, y, sizes)| XySeries { name, x, y, sizes })
            .collect();
        self.inner.set_chart_xy_data(shape(slide, id), &series).py()
    }

    fn chart_title(&self, slide: u32, id: u32) -> PyResult<Option<String>> {
        self.inner.chart_title(shape(slide, id)).py()
    }

    fn set_chart_title(&mut self, slide: u32, id: u32, title: &str) -> PyResult<()> {
        self.inner.set_chart_title(shape(slide, id), title).py()
    }
}

#[pymodule(gil_used = false)]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPresentation>()?;
    m.add_class::<PyShapeInfo>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
