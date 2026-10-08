# Architecture

afterslides edits existing PowerPoint files. It never builds a full object
model of the deck; it works directly on the XML of the parts it needs and
leaves everything else alone. That keeps it small, fast, and faithful to
templates that use features we don't know about.

```
Python API (python/afterslides/_api.py)
   Presentation, Slide, Shape, Table, Chart   ← thin handles, no state
        │ ids
PyO3 bindings (crates/afterslides-py)          ← errors mapped, GIL released for I/O
        │
Rust core (crates/afterslides)
   presentation / slide / shape / table / chart / text
        │
   opc: zip, content types, relationships, GC
        │
   xml: mutable tree, prefixes preserved
```

## Layers

**`xml`** is a small DOM on top of `quick-xml`. Each element stores its
qualified name exactly as written plus the resolved namespace URI. Lookups go
by `(namespace, local name)`, output uses the original names. New elements
copy prefix and namespace from a neighbour (`Element::new_like`), so they
match whatever the producer used. DTDs are ignored and only the five
predefined entities are expanded.

**`opc`** reads the zip into memory. Each part stays raw bytes until someone
asks for its XML; parsed parts are only serialized again if they were opened
for writing. `[Content_Types].xml` and relationship parts are handled the
same way. After deletions, `collect_garbage` drops every part that can no
longer be reached from the package relationships, which takes care of
shared media and chart workbooks without reference counting.

**Domain modules** implement operations on top of the package:

| Module | Responsibility |
| --- | --- |
| `presentation` | open/save, slide list (`p:sldIdLst`), handles |
| `shape` | walking `p:spTree` (groups, `mc:AlternateContent`), text, deletion |
| `text` | text bodies, cross-run placeholder replacement |
| `table` | `a:tbl` cells, row/column insert and delete, frame size |
| `chart` | series caches, series cloning, embedded workbook |
| `slide` | delete/duplicate/move slides and all references to them |

## Handles instead of objects

Slides are identified by the `id` of their `p:sldId` entry and shapes by
their `p:cNvPr` id. Both are stable while the file is being edited: other
slides can be added, moved or removed without invalidating them. The Python
objects are just such handles plus a reference to the presentation, which
means they hold no state that could go stale and never form reference cycles.

## Charts

A chart lives in its own part (`/ppt/charts/chartN.xml`) with an embedded
workbook (`/ppt/embeddings/*.xlsx`). PowerPoint draws from the value caches
in the chart XML and opens the workbook only for *Edit Data*. Replacing data:

1. Make the number of `c:ser` elements match the new series. Extra series are
   deep copies of the template's last series (formatting included) with
   fresh `c:idx`, `c:order` and `c16:uniqueId`; surplus series are removed.
2. Rewrite each series' name (`c:tx`), categories (`c:cat`) and values
   (`c:val`, or `c:xVal`/`c:yVal`/`c:bubbleSize`): formula plus cache,
   keeping the template's number format.
3. Drop per-point formatting and labels past the new point count.
4. Generate a fresh workbook with `rust_xlsxwriter` in the layout the
   formulas point to: categories in column A, one column per series.

## Why not a typed OOXML model?

Generated object models cover the schema but tend to lose what they don't
model (vendor extensions, newer elements) or normalize prefixes, and they
make every part expensive to load. For template filling we touch a handful of
elements in a handful of parts; a namespace-aware DOM with careful
round-tripping is simpler and safer.
