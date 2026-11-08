# Shell build and packaging

The shell is Rust/Cargo-only. Cargo builds its libraries, binary, tests, and
workspace targets; no Qt, CMake, or generated native plugin is part of the
supported build. Cargo fetches `knave-ui` and `knave-renderer` from the standalone
Knave UI repository at the same exact revision; no sibling source checkout is
needed for an ordinary Shell build. Coordinated development uses Knave
workspaces, which patch both crates to their recorded source snapshot.

Build changes must check debug and release artifact paths, generated files,
dependency discovery, staged installation, and runtime lookup paths. Do not
silently install into `/usr/local` or assume a workspace artifact is the
installed artifact.

A shell build passing compilation does not prove Wayland, layer-shell, GPU, or
installed-session startup. Those paths require explicit integration or live
smoke verification and must be reported separately from unit-test results.
