# Fold Engine — Recent Updates (2026)

## Overview
The Fold Engine is VeerOS's secure, cross-platform compute envelope. Recent updates include:

- **cgroups v2 resource limits**: Fine-grained CPU/memory/pid control for each fold.
- **seccomp default-deny**: Minimal syscall surface, with per-fold denylist.
- **Rootless mode**: User namespaces, no root required for basic folds.
- **TOML manifest**: Declarative runtime configuration, including namespaces, seccomp, limits, env.
- **State registry**: On-disk fold state in `$XDG_STATE_HOME/veeros/fold/`.
- **CLI improvements**: `fold spawn`, `fold list`, `fold logs`, `fold stop`, `fold rm`.
- **Integration with veer-vm**: `fold vm spawn` launches microVMMs as folds.

See also: [crates/fold_engine/README.md](../crates/fold_engine/README.md)

## Roadmap
- Ed25519-signed manifests (via crypto crate)
- veth/bridge networking
- OverlayFS rootfs layering
- GPU/accelerator attach
- Snapshot/migration
- macOS/Windows backends

---

_Last updated: 2026-04-25_
