# Contributing to Oversample

Thank you for your interest in contributing to Oversample!

## Licensing

This project uses a split licensing model:

- **Library crates** (`oversample-core/`, `xc-lib/`) are dual-licensed under the **MIT License** and **Apache License 2.0**. You may use library code under either license at your option.
- **Application crates** (`oversample`, `oversample-desktop`, `xc-cli`) are licensed under the **GNU General Public License v3.0** (GPL-3.0-only).
- Some files within the application crates are **triple-licensed** (GPL-3.0-only OR MIT OR Apache-2.0), as marked by `SPDX-License-Identifier` headers at the top of each file.

### Contribution License Terms

By submitting a pull request, you agree that your contribution is licensed under **all three** of the following licenses, regardless of where in the codebase it lands:

- **MIT License**
- **Apache License 2.0**
- **GNU General Public License v3.0**

This means every contribution can be used under any of these licenses. While the application is currently distributed under GPL-3.0, triple-licensing all contributions preserves the option to relicense the entire codebase as MIT/Apache-2.0 in the future.

### Developer Certificate of Origin

Please sign off your commits to certify the [Developer Certificate of Origin](https://developercertificate.org/):

```
git commit -s -m "Your commit message"
```

This adds a `Signed-off-by` line to your commit, certifying that you have the right to submit the code under the project's license terms.

## Getting Started

See [CLAUDE.md](CLAUDE.md) for build commands, project structure, and architecture details.

### Quick Start

```bash
# Web (WASM) dev server
trunk serve --release

# Desktop (Tauri) dev mode
cargo tauri dev

# Quick compile check
cargo check
```

## Guidelines

- Keep PRs focused on a single change
- Follow existing code style and patterns
- Test your changes manually (load audio files, verify spectrogram rendering, test playback)
- There is no automated test suite currently
