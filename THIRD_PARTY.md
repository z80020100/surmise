# Third-party material

## Completion specification data

`specs/` holds data derived from `@withfig/autocomplete`, published on npm.
`tools/spec-convert` produced it: the tool evaluates each published module,
keeps the fields that are data and drops the fields that are code. The command
names, the option names and the descriptions in `specs/` are the upstream's own
text. `specs/index.json` records the exact version each regeneration read.

That package is distributed under the MIT licence and `specs/LICENSE` carries
its licence text as published, including the copyright line. Note that the npm
manifest's `license` field says ISC while the `LICENSE` file inside the
published tarball says MIT. What is distributed is MIT and the manifest field is
wrong metadata.

Upstream: <https://github.com/withfig/autocomplete>

## Everything else

surmise's own code is MIT or Apache-2.0, at your option. See `LICENSE-MIT` and
`LICENSE-APACHE`. Its dependencies are in `Cargo.lock` and each carries its own
licence.
