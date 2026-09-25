# Build and packaging

The new Knave stack is Rust/Cargo-first. Cargo owns Rust libraries, binaries,
tests, and workspace orchestration. The intended Rust shell and wgpu renderer
must be buildable without a Make-based orchestration layer.

The current Qt shell is transitional and may continue using CMake. Transitional
native builds use Ninja, isolated debug/release directories, and a staged
installation prefix. They must not install implicitly into `/usr/local`.

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
