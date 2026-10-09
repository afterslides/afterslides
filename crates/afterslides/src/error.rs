use std::fmt;

/// Everything that can go wrong while reading, editing or writing a deck.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not a valid zip archive: {0}")]
    Zip(String),

    #[error("malformed XML: {0}")]
    Xml(String),

    #[error("invalid package: {0}")]
    Package(String),

    #[error("{0} not found")]
    NotFound(Target),

    #[error("{0}")]
    Unsupported(String),

    #[error("{0}")]
    InvalidArgument(String),

    #[error("could not write embedded workbook: {0}")]
    Workbook(String),
}

/// What a [`Error::NotFound`] was looking for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Target {
    Part(String),
    Slide(u32),
    SlideIndex(usize),
    Shape { slide: u32, shape: u32 },
    ShapeNamed(String),
    Relationship(String),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Part(name) => write!(f, "part {name}"),
            Target::Slide(id) => write!(f, "slide with id {id}"),
            Target::SlideIndex(idx) => write!(f, "slide at index {idx}"),
            Target::Shape { slide, shape } => write!(f, "shape {shape} on slide {slide}"),
            Target::ShapeNamed(name) => write!(f, "shape named {name:?}"),
            Target::Relationship(id) => write!(f, "relationship {id}"),
        }
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        match e {
            zip::result::ZipError::Io(io) => Error::Io(io),
            other => Error::Zip(other.to_string()),
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
