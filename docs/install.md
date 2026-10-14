# Install

os-int is a local binary. There is no cloud account. After install, run `os-int doctor`, then import the [demo case](../examples/demo-case/).

## Sixty-second path (Linux and macOS)

A **published** tagged GitHub Release is the supported one-liner. It verifies a SHA-256 checksum and installs `os-int` into `~/.local/bin` (override with `OS_INT_INSTALL_DIR`).

Pin both the script URL and `OS_INT_VERSION` to the same tag. Do not fetch `install.sh` from `main`; a newer script on `main` may not match the tagged archive layout.

```bash
OS_INT_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/pjourney/osint-rat/v0.1.0/scripts/install.sh | bash
```

`OS_INT_VERSION=latest` (the script default) calls GitHub `/releases/latest`. **Draft releases are not latest.** The Release workflow attaches binaries to a draft GitHub Release; `/releases/latest` stays empty until a maintainer publishes that draft. Pin a tag after publish, or install from source until then.

If `~/.local/bin` is not on your `PATH`, add it, then:

```bash
os-int --version
os-int doctor
```

### cargo-binstall

The crate is not on crates.io yet. Fetch GitHub Release artifacts from this repository:

```bash
cargo binstall --git https://github.com/pjourney/osint-rat
```

`Cargo.toml` carries `[package.metadata.binstall]` with hardcoded `.tar.gz` / `.zip` URLs so the binary is fetched from GitHub Releases, not built from source. `cargo binstall os-int` (crates.io lookup) will fail until a crate is published; that publish is deferred.

### Homebrew

A formula is not published from this repository. If you maintain a tap, the release artifacts are:

| File | Platform |
| --- | --- |
| `os-int-x86_64-unknown-linux-gnu.tar.gz` | Linux x86_64 |
| `os-int-aarch64-apple-darwin.tar.gz` | macOS Apple silicon |
| `os-int-x86_64-apple-darwin.tar.gz` | macOS Intel |
| `os-int-x86_64-pc-windows-msvc.zip` | Windows x64 |

A tap recipe would `url` one of those tarballs, `sha256` the matching `.sha256` file, and install the nested binary (`os-int-<target>/os-int`). Do not publish a tap from a phase pull request; tag `v0.1.0` on `main` first, then publish the draft GitHub Release. Linux aarch64 has no prebuilt archive; use the source install below.

## Windows

Download `os-int-x86_64-pc-windows-msvc.zip` from a **published** [GitHub Release](https://github.com/pjourney/osint-rat/releases), verify the `.sha256` file, and put `os-int.exe` on your `PATH`. The zip nests `os-int-x86_64-pc-windows-msvc/os-int.exe`. PowerShell:

```powershell
Get-FileHash os-int-x86_64-pc-windows-msvc.zip -Algorithm SHA256
```

There is no `install.sh` for Windows. `cargo install --git https://github.com/pjourney/osint-rat` works when a Rust toolchain is present.

## From source

Rust **1.88** or newer (edition 2024). SQLite is bundled.

```bash
git clone https://github.com/pjourney/osint-rat.git
cd osint-rat
cargo install --path .
```

Or:

```bash
cargo install --git https://github.com/pjourney/osint-rat
```

Use this path when no published tagged release exists yet.

## Next

Import [examples/demo-case](../examples/demo-case/README.md) and open Graph. Collection is optional for that walkthrough.
