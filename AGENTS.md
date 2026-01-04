# Repository Guidelines

## Project Structure & Module Organization
- `waytray-daemon/` contains the backend daemon, module system, and D-Bus services (`waytray-daemon/src/`).
- `waytray-client/` contains the GTK4 client UI (`waytray-client/src/`).
- `packaging/` holds distro packaging assets (RPM spec in `packaging/rpm/`).
- `examples/` contains sample scripts for the scripts module (`examples/scripts/`).
- Root `Cargo.toml` defines the Rust workspace for both crates.

## Build, Test, and Development Commands
- `cargo build` builds debug binaries for both crates.
- `cargo build --release` produces optimized binaries in `target/release/`.
- `cargo check` runs a fast compile check without producing binaries.
- `cargo check -p waytray-daemon` checks only the daemon (useful without GTK4 installed).
- `cargo test` runs any available tests (currently minimal/none).

## Coding Style & Naming Conventions
- Rust 2021 edition; follow standard Rust style (rustfmt defaults: 4‑space indent, trailing commas).
- Naming: `snake_case` for functions/vars/modules, `CamelCase` for types/traits, `SCREAMING_SNAKE_CASE` for constants.
- Keep module boundaries clear: daemon logic in `waytray-daemon/`, UI logic in `waytray-client/`.

## Testing Guidelines
- Tests are currently sparse; add unit tests alongside modules where practical.
- Use `cargo test` locally before submitting changes.
- Prefer targeted tests in the relevant crate (e.g., `cargo test -p waytray-daemon`).

## Commit & Pull Request Guidelines
- Commit messages are short, imperative, and capitalized (e.g., “Add GPU module”, “Fix tray item cleanup”).
- Release commits use “Release vX.Y.Z” formatting.
- PRs should include: a concise summary, rationale, and any user‑visible behavior changes.
- If UI behavior changes, include screenshots or a short description of the interaction.

## Configuration & Runtime Notes
- Runtime config lives at `~/.config/waytray/config.toml` and is hot‑reloaded by the daemon.
- The client uses GTK4 (and GTK Layer Shell on Wayland when available); the daemon can be built without GTK4.
