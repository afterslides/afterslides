"""Fill PowerPoint templates with data: text, tables and native charts.

>>> from afterslides import Presentation
>>> prs = Presentation("template.pptx")
>>> prs.replace_text({"{{customer}}": "Acme"})
>>> prs.shape("Revenue").chart.replace_data(["Q1", "Q2"], {"2026": [1.5, 2.0]})
>>> prs.save("report.pptx")
"""

from afterslides._api import (
    Chart,
    Presentation,
    Series,
    Shape,
    Slide,
    Slides,
    Table,
    XySeries,
)
from afterslides._native import __version__
from afterslides.errors import (
    AfterslidesError,
    InvalidArgumentError,
    NotFoundError,
    PackageError,
    UnsupportedError,
)

__all__ = [
    "AfterslidesError",
    "Chart",
    "InvalidArgumentError",
    "NotFoundError",
    "PackageError",
    "Presentation",
    "Series",
    "Shape",
    "Slide",
    "Slides",
    "Table",
    "UnsupportedError",
    "XySeries",
    "__version__",
]
