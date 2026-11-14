# Third-party notices

Riff's independently authored code is licensed under MIT; see [LICENSE](LICENSE).
That license does not replace the licenses of Cargo dependencies or embedded
syntax definitions.

## Source repository and Cargo dependencies

Build dependencies are retrieved by Cargo rather than vendored. The README
screenshot contains the separately attributed example excerpt below.
`Cargo.lock` pins the resolved versions; Cargo retrieves the crates and their upstream license files
when building. Keep `LICENSE`, this notice, and all files in `licenses/`
(the five syntax-notice parts and Similar's Apache-2.0 license) in source copies. Build output and dependency caches are not source-repo
release materials.

Not every dependency is MIT licensed. Examples from the locked default Linux
normal/build dependency graph, with versioned source and license files:

| Dependency | Declared license | Upstream source and license |
| --- | --- | --- |
| option-ext 0.2.0 (via dirs → dirs-sys) | MPL-2.0 | [Source / LICENSE.txt](https://docs.rs/crate/option-ext/0.2.0/source/) |
| unicode-ident 1.0.27 (proc-macro dependency) | (MIT OR Apache-2.0) AND Unicode-3.0 | [Source / LICENSE-UNICODE](https://docs.rs/crate/unicode-ident/1.0.27/source/) |
| inotify 0.11.5 and inotify-sys 0.1.8 | ISC | [inotify](https://docs.rs/crate/inotify/0.11.5/source/), [inotify-sys](https://docs.rs/crate/inotify-sys/0.1.8/source/) |
| notify 8.2.2 | CC0-1.0 | [Source / LICENSE-CC0](https://docs.rs/crate/notify/8.2.2/source/) |
| foldhash 0.2.0 | Zlib | [Source / LICENSE](https://docs.rs/crate/foldhash/0.2.0/source/) |
| ryu 1.0.23 | Apache-2.0 OR BSL-1.0 | [Source / license alternatives](https://docs.rs/crate/ryu/1.0.23/source/) |
| similar 3.2.1 | Apache-2.0 | [Source](https://docs.rs/crate/similar/3.2.1/source/), [included license](licenses/similar-APACHE-2.0.txt) |
| unicode-segmentation 1.13.3 (also a direct dependency) | MIT OR Apache-2.0 | [Source / licenses](https://docs.rs/crate/unicode-segmentation/1.13.3/source/) |

This is not an exhaustive inventory. Syntect 5.3.0 is MIT licensed; many other
crates offer MIT as one alternative. `OR` offers a license choice; `AND` requires
both terms, so Unicode notices must not be discarded when choosing MIT.
The lockfile and unfiltered Cargo metadata also contain disabled optional and
platform-specific packages. For example, finl_unicode and wezterm-bidi declare
additional Unicode-DFS-2016 terms, while terminfo declares WTFPL; these are not
in Riff's default dependency graph. Do not equate every metadata entry with code
redistributed in an executable, or assume a build-time/proc-macro dependency is
linked into it. Recheck the actual target and enabled features for each release.

The notices here preserve bundled grammar attributions; **they are not a
complete dependency-license bundle for binary redistribution**. See the binary
checklist below before distributing an executable.

## README screenshot

The README screenshot shows illustrative edits to Ratatui's MIT-licensed
[hello-world example](https://github.com/ratatui/ratatui/blob/e665c36cb14752a61cd777fbd06dbef8474f2add/examples/apps/hello-world/src/main.rs).
The pinned source commit is `e665c36cb14752a61cd777fbd06dbef8474f2add`.
The edits are demonstrations, not an actual upstream change. The image contains
only riff's UI and adapted excerpts of this public example code. Retain the
following complete upstream MIT notice with the screenshot:

```text
The MIT License (MIT)

Copyright (c) 2016-2022 Florian Dehau
Copyright (c) 2023-2025 The Ratatui Developers

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Token diff dependency

Word highlighting calls similar's generic `capture_diff_slices` API with Myers
on bounded token slices; it does not copy or modify upstream source. Similar's
std/text defaults add no mandatory dependencies. Copyright/author attribution:
Armin Ronacher, Pierre-Étienne Meunier, and Brandon Williams; upstream authors
and license remain unchanged. Include [its Apache-2.0 license](licenses/similar-APACHE-2.0.txt)
with binary redistribution. Riff's original code remains MIT. The direct
unicode-segmentation dependency reuses the existing locked grapheme library.
Optional dependency entries in Cargo.lock are not necessarily enabled code.

## Bundled grammars

Riff uses two-face 0.5.2+bat-0.26.1's fancy-regex syntax bundle,
curated by the bat project. two-face and bat are MIT OR Apache-2.0;
the MIT option and notices are reproduced below. Embedded grammars retain
their own licenses, not riff's MIT license. No two-face themes are used.

The linked syntax notices are generated verbatim with
`two_face::acknowledgement::listing().for_syntaxes()` and `License::write_md`,
keeping entries for which `License::needs_acknowledgement()` is true.
This is a conservative superset: it also includes grammars excluded from
the fancy-regex bundle. Full upstream license listing (including licenses
that do not require acknowledgement):

https://codeberg.org/CosmicHarper/two-face/src/tag/v0.5.2+bat-0.26.1/generated/acknowledgements_full.md

## two-face and bat (MIT option)

Upstream MIT texts: [two-face 0.5.2+bat-0.26.1](https://codeberg.org/CosmicHarper/two-face/src/tag/v0.5.2+bat-0.26.1/LICENSE-MIT)
and [bat v0.26.1](https://github.com/sharkdp/bat/blob/v0.26.1/LICENSE-MIT).

Copyright (c) 2018-2023 bat-developers (https://github.com/sharkdp/bat).

MIT License

Copyright (c) 2023-2025 The `two-face` developers (https://codeberg.org/CosmicHarper/two-face).

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Embedded syntax notices

The 66 complete license entries are split only at entry boundaries to keep each
file below 1,000 lines; redistribute **all five parts** with this notice.
The license texts themselves are unmodified.

- [Part 1](licenses/syntax-notices-1.md)
- [Part 2](licenses/syntax-notices-2.md)
- [Part 3](licenses/syntax-notices-3.md)
- [Part 4](licenses/syntax-notices-4.md)
- [Part 5](licenses/syntax-notices-5.md)

Riff modifies the bundled TypeScript/TSX expression rules in memory at load
time to recognize `satisfies` using the existing `as` parser transitions.
These changes are in `src/highlight.rs`; the upstream grammar license is retained
in the syntax notices above (TypeScript, Apache-2.0).

## Before distributing binaries

This source-repository preparation does not publish or prepare binary releases.
For a future binary distribution:

1. Record the exact lockfile, target, and enabled features. Inspect the selected
   graph with `cargo tree --locked --target <target-triple> --edges normal,build`
   (adding the release's feature flags), rather than treating all optional
   metadata packages as linked dependencies.
2. Use an established attribution tool such as
   [cargo-about](https://embarkstudios.github.io/cargo-about/) and its
   `cargo about generate` command, configured for that manifest, target, features,
   license choices, and output template. Review its generated attribution against
   upstream license files, copyright notices, and any additional `NOTICE` or
   embedded-data terms; metadata alone is not the full license text.
3. Ship the resulting required dependency license texts and attributions with the
   binary, **in addition to** Riff's `LICENSE`, this notice, and all five syntax
   notice files. The source links above are for discovery, not substitutes for
   license texts required to accompany redistributed copies. Review any bundled
   runtime or system-library materials as well.
4. In particular, `option-ext` is MPL-2.0, not an MIT alternative. Make its matching
   source (including any modifications) available under MPL-2.0 and inform binary
   recipients how to obtain it, as required by sections 3.1–3.2; preserve its
   license and notices. This does not relicense Riff's independent source files.

Repeat this review when dependencies, features, embedded assets, or targets
change. This is maintenance guidance, not formal legal clearance.
