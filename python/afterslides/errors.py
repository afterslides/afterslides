"""Exceptions raised by afterslides.

Every error derives from :class:`AfterslidesError`. The more specific classes
also derive from the matching built-in exception, so ``except KeyError`` or
``except ValueError`` keep working where that reads more naturally.
File system errors are raised as plain :class:`OSError`.
"""

from __future__ import annotations

__all__ = [
    "AfterslidesError",
    "InvalidArgumentError",
    "NotFoundError",
    "PackageError",
    "UnsupportedError",
]


class AfterslidesError(Exception):
    """Base class for all afterslides errors."""


class PackageError(AfterslidesError):
    """The file is not a valid .pptx (broken zip, malformed XML, missing parts)."""


class NotFoundError(AfterslidesError, LookupError):
    """A slide, shape or part does not exist (any more)."""


class InvalidArgumentError(AfterslidesError, ValueError):
    """An argument does not fit the template, e.g. too many values for a table row."""


class UnsupportedError(AfterslidesError):
    """The operation does not apply to this shape or chart type."""
