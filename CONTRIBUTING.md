# Contributing to riff

## Development setup

Install Git, Rust 1.93.1 or newer through rustup, a native C build toolchain,
and Python 3.10 or newer. Python is only needed for the PTY tests and uses
the standard library, with no pip dependencies.

Clone the repository:

```sh
git clone https://github.com/jhargis/riff.git
cd riff
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
cargo +stable fetch --locked
cargo +stable fmt --check
cargo +stable test --all-targets --locked
cargo +stable clippy --all-targets --locked -- -D warnings
cargo +stable build --release --locked
```

Use Cargo's normal writable cache. No project-specific Cargo configuration or
pre-populated cache is required. The initial fetch needs registry access.
Keep `Cargo.lock` in version control because riff is a binary application.
When changing dependencies, review the lockfile and attribution changes together.

Run the same full Rust test command with `cargo +1.93.1` after installing that
toolchain to check the minimum supported Rust version. Do not raise the
minimum version accidentally through a dependency update.

## Tests and isolation

The Linux PTY checks require a regular non-root user, accessible `/dev/ptmx`,
`fork`, Unix terminal APIs, and native filesystem notifications:

```sh
python3 tests/pty_smoke.py target/release/riff
python3 tests/keyboard_protocol_smoke.py target/release/riff
python3 tests/final_regressions_smoke.py target/release/riff
python3 tests/pty_reload.py target/release/riff
python3 tests/pty_watch_permissions.py target/release/riff
python3 tests/pty_watch_permission_submodules.py target/release/riff
python3 tests/pty_intraline.py target/release/riff
```

The permission/watch scripts specifically check ignored permission-denied siblings,
lowercase `ctrl+w` dispatch, and native submodule watching using only owned
fixtures. The intraline script checks word-background toggles with search,
wrapping, theme acceptance, and watch/manual reload in both syntax modes and
layouts. The harness opens its own controlling PTYs, not your terminal. It
writes transcripts and results below ignored `target/` directories. Config-writing
tests use temporary HOME/XDG_CONFIG_HOME/APPDATA or explicit fixture paths.
Generated `target/` and `riff-input-tests-*` fixtures are excluded from text
normalization so CRLF and binary-input assertions keep testing literal bytes.

Git tests initialize disposable, test-owned repositories and make fixture
commits with synthetic identities. They do not need your Git name/email,
credentials, an existing remote, or network access. Keep all fixture writes
and Git mutations confined to their owned temporary directories. Never use a
real checkout or actual user settings as a writable test fixture. Do not
replace real Git globally or weaken existing runtime assertions.

To keep inherited system/global Git settings out of the test processes on
Linux, prefix test commands with:

```sh
GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null cargo +stable test --all-targets --locked
```

Apply the same prefix to Python PTY commands if needed. This changes only the
child process environment, not your Git configuration. These are also the Git
environment settings used by CI.

## Changes and licensing

Keep changes focused and add regressions for behavior changes. Run formatting,
all Rust tests, strict Clippy, and the relevant real PTY tests before a pull
request. Keep handwritten source files at or below 1,000 lines. New or modified
Python must have type annotations for function signatures and variables.

Contributions must be original or include compatible permission and preserved
attribution. Contributions to riff's original code are under the repository's
[MIT license](LICENSE). Do not copy another project's implementation without
reviewing its license and provenance. Bundled syntax definitions have separate
terms in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) and `licenses/`;
do not remove or relicense those notices.

Do not include credentials, personal email addresses, local paths, build
artifacts, or private verification notes in contributions. Use GitHub's provided
noreply address for commit metadata if you want to keep your email private.
