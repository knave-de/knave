# Build and packaging

The Knave stack is Rust/Cargo-only for the supported environment. Cargo owns
libraries, binaries, tests, and workspace orchestration across Knave and the
Rust/wgpu shell. No Make, CMake, Qt, or native plugin build is required by the
supported runtime.

Installers use isolated release artifacts and an explicit user, system, or
staged prefix. They must not install implicitly into `/usr/local`.

Every build change checks:

- debug and release artifact paths;
- generated files and dependency discovery;
- reproducibility from a clean checkout;
- installed-versus-workspace artifact drift; and
- package contents and runtime lookup paths.

Build cleanup is not complete until the replacement path is exercised, the
transitional path is no longer required, and removal has a separate reviewed
change. Compilation alone does not prove Wayland, GPU, VT, or installed-binary
startup.
