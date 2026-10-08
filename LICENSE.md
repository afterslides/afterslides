# License

afterslides is free software: you can redistribute it and/or modify it under
the terms of the **GNU Lesser General Public License** as published by the
Free Software Foundation, either **version 3** of the License, or (at your
option) any later version.

afterslides is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR
A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details.

The full license texts are in this repository:

- [`COPYING.LESSER`](COPYING.LESSER): GNU Lesser General Public License v3
- [`COPYING`](COPYING): GNU General Public License v3, which the LGPL builds on

SPDX identifier: `LGPL-3.0-or-later`

Copyright © 2026 Andreas Kluth and the afterslides contributors.

## What this means in practice

This summary is not legal advice and does not replace the license text.

- **Using afterslides** from your application, open source or proprietary, is
  fine. Installing the Python package or depending on the Rust crate does not
  put your own code under the LGPL.
- **Modifying afterslides** and distributing the result (including as part of
  a product) requires you to publish those modifications under the LGPL.
- **Distributing** afterslides (for example in a container image you ship to
  customers) requires passing on the license and offering the source of the
  afterslides version you ship.
- For the Rust crate, which is usually linked statically: the LGPL lets
  recipients relink with a modified afterslides. Providing your application's
  object files, or the source, satisfies that. If this is a problem for your
  use case, open an issue.

## Third-party code

afterslides depends on crates and packages under permissive licenses
(MIT, Apache-2.0, BSD). Their licenses are listed in their respective
repositories; `cargo about` or `pip-licenses` can produce a full report.
