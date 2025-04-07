# This repository has moved to <https://codeberg.org/msrd0/cargo-doc2readme>

# cargo-doc2readme ![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue) [![cargo-doc2readme on crates.io](https://img.shields.io/crates/v/cargo-doc2readme)](https://crates.io/crates/cargo-doc2readme) [![Source Code Repository](https://img.shields.io/badge/Code-On%20Codeberg-blue?logo=Codeberg)](https://codeberg.org/msrd0/cargo-doc2readme) ![Rust Version: 1.61.0](https://img.shields.io/badge/rustc-1.61.0-orange.svg)

`cargo doc2readme` is a cargo subcommand to create a readme file to display on
[Codeberg][__link0], [GitHub][__link1] or [crates.io][__link2],
containing the rustdoc comments from your code.

## Installation

If you are using ArchLinux, you can install cargo-doc2readme from the AUR:

```bash
yay -S cargo-doc2readme
```

On other Operating Systems, make sure you have Rust installed (using your
distributions package manager, but if your package manager is garbage or you are
running Windows, try [rustup][__link3]) and then run the following command:

```bash
cargo install cargo-doc2readme
```

## Usage

To generate your readme, simply run

```bash
cargo doc2readme
```

This will output the readme to a file called `README.md`, using `README.j2` or the
built-in template.

If you want to run this using Forgejo or GitHub Actions, you can use the pre-built
docker image:

```yaml
readme:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: docker://codeberg.org/msrd0/cargo-doc2readme
      with:
        entrypoint: cargo
        args: doc2readme --check
```

This will use the latest stable Rust version available when the latest release of
cargo doc2readme was created. If you need a newer/nightly Rust compiler, use the
`ghcr.io/msrd0/cargo-doc2readme:nightly` docker image instead.

## Features

* parse markdown from your rustdoc comments and embed it into your readme
* use existing crates to parse Rust and Markdown
* support your `[CustomType]` rustdoc links
* default, minimalistic readme template with some useful badges
* custom readme templates

## Non-Goals

* verbatim copy of your markdown
* easy readability of the generated markdown source code

## Similar tools

[`cargo readme`][__link4] is a similar tool. However, it brings its own Rust code
parser that only covers the 95% use case. Also, it does not support Rust path links
introduced in Rust 1.48, making your readme ugly since the unsupported links are shown
as raw markdown, and being less convenient for the reader that has to search [docs.rs][__link5]
instead of clicking on a link.

[`cargo rdme`][__link6] is another tool which places your rustdoc into your readme.
However, it does not come with template support, which is helpful for bigger repositories
that contain multiple crates, and has only very basic support for Rust path links. All
links not starting with `crate::` or `::std` are unsupported by `cargo rdme`.

## Stability Guarantees

This project adheres to semantic versioning. All versions will be tested against the
latest stable rust version at the time of the release. All non-bugfix changes to the
rustdoc input processing and markdown output or the default readme template are
considered breaking changes, as well as any non-backwards-compatible changes to the
command-line arguments or to these stability guarantees. All other changes, including
any changes to the Rust code, or bumping the MSRV, are not considered breaking changes.


 [__link0]: https://codeberg.org
 [__link1]: https://github.com
 [__link2]: https://crates.io
 [__link3]: https://rustup.rs/
 [__link4]: https://github.com/webern/cargo-readme
 [__link5]: https://docs.rs
 [__link6]: https://github.com/orium/cargo-rdme
