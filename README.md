# cargo-doc2readme ![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue) [![cargo-doc2readme on crates.io](https://img.shields.io/crates/v/cargo-doc2readme)](https://crates.io/crates/cargo-doc2readme) [![Source Code Repository](https://img.shields.io/badge/Code-On%20GitHub-blue?logo=GitHub)](https://github.com/msrd0/cargo-doc2readme) ![Rust Version: 1.61.0](https://img.shields.io/badge/rustc-1.61.0-orange.svg)

`cargo doc2readme` is a cargo subcommand to create a readme file to display on
[GitHub][__link0] or [crates.io][__link1],
containing the rustdoc comments from your code.

## Installation

If you are using ArchLinux, you can install cargo-doc2readme from the AUR:

```bash
yay -S cargo-doc2readme
```

On other Operating Systems, make sure you have Rust installed (using your
distributions package manager, but if your package manager is garbage or you are
running Windows, try [rustup][__link2]) and then run the following command:

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

If you want to run this using GitHub Actions, you can use the pre-built docker image:

```yaml
readme:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v3
    - uses: docker://ghcr.io/msrd0/cargo-doc2readme
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

[`cargo readme`][__link3] is a similar tool. However, it brings its own Rust code
parser that only covers the 95% use case. Also, it does not support Rust path links
introduced in Rust 1.48, making your readme ugly due to GitHub showing the unsupported
links as raw markdown, and being less convenient for the reader that has to search
[docs.rs][__link4] instead of clicking on a link.

## Stability Guarantees

This project adheres to semantic versioning. All versions will be tested against the
latest stable rust version at the time of the release. All non-bugfix changes to the
rustdoc input processing and markdown output or the default readme template are
considered breaking changes, as well as any non-backwards-compatible changes to the
command-line arguments or to these stability guarantees. All other changes, including
any changes to the Rust code, or bumping the MSRV, are not considered breaking changes.


 [__link0]: https://github.com
 [__link1]: https://crates.io
 [__link2]: https://rustup.rs/
 [__link3]: https://github.com/livioribeiro/cargo-readme
 [__link4]: https://docs.rs
