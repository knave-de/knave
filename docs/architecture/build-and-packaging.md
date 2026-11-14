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

The workspace emits only `knave`. Component crates are libraries. Both Cargo
profiles place the executable at `target/PROFILE/knave`; internal process modes
re-execute that artifact instead of resolving component names through PATH.
The installer installs only this executable and shared data. Old installed
executables are not deleted automatically; no current launch path uses them.
The separate XDG portal backend remains a service executable and must use the
coordinated private dialog launch contract.
