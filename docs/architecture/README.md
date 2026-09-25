# Architecture documentation

Knave is the umbrella owner for the desktop-wide architecture, configuration,
versioning, build, and change-impact policy.

- [Component boundaries](component-boundaries.md)
- [Configuration](configuration.md)
- [Session lifecycle](session.md)
- [Versioning and compatibility](versioning.md)
- [Build and packaging](build-and-packaging.md)
- [Performance and resource usage](performance.md)
- [Change impact](change-impact.md)
- [ADR 0001: environment boundaries](decisions/0001-knave-environment-boundaries.md)

These documents describe the Rust/Cargo-first environment. Knave is an
independent desktop environment; host desktop integration is out of scope.
The supported shell path is the Rust/wgpu implementation, and public control
commands belong to Knave's `knavectl` contract.
