# Security policy

afterslides parses files that often come from outside your organization, so
we treat parser bugs as potential security issues.

## Supported versions

Until 1.0, only the latest release receives fixes.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's
[private vulnerability reporting](https://github.com/afterslides/afterslides/security/advisories/new)
or email mail@andreaskluth.net with a description and, if possible, a file
that reproduces the problem. You will get an answer within a week.

## Hardening already in place

- No `unsafe` code in the Rust crates (`unsafe_code = "forbid"` in the workspace lints).
- XML is parsed without DTD processing: no external entities, no entity
  expansion, so no XXE or "billion laughs".
- Zip entries are limited to 512 MiB each and 2 GiB in total after
  decompression, guarding against zip bombs. Size headers are not trusted;
  limits are enforced while inflating.
- Nothing is ever extracted to disk, so path traversal in zip entry names
  has no effect.
- External relationships (links to other files or URLs) are kept as they are
  but never followed.

If you process untrusted files in a long-running service, still run it with
memory limits; a crafted file within the limits above can use a few GiB.
