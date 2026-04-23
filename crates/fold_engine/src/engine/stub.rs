//! Non-Linux placeholder backend. macOS (sandbox_init + launchd) and
//! Windows (Job Objects + Windows containers) will be implemented in
//! subsequent slices; for now these targets build but refuse to spawn.

use anyhow::{bail, Result};

use super::Engine;
use crate::manifest::Manifest;
use crate::state::FoldRecord;

pub struct StubEngine;

impl StubEngine {
    pub fn new() -> Self { Self }
}

impl Engine for StubEngine {
    fn spawn(&self, _manifest: Manifest) -> Result<FoldRecord> {
        bail!("fold_engine: host platform not yet supported (Linux MVP only)")
    }
    fn stop(&self, _rec: &FoldRecord) -> Result<()> {
        bail!("fold_engine: host platform not yet supported")
    }
    fn is_alive(&self, _rec: &FoldRecord) -> bool { false }
}
