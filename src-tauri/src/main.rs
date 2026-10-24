//! All-in-One Desktop executable.
//!
//! Default mode starts the real Tauri 2.x desktop app. `--smoke-test` runs the same
//! microkernel IPC workflow headlessly for CI and development verification.

use std::{env, path::PathBuf, sync::Arc};

use all_in_one_host::DesktopHost;
#[path = "../tests/support/cli_smoke.rs"]
mod smoke;

fn main() -> anyhow::Result<()> {
    if env::args().any(|arg| arg == "--smoke-test") {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".runtime_data")
            .join("smoke_test");
        let host = Arc::new(DesktopHost::bootstrap(data_dir, true)?);
        host.seed_demo_data_if_empty()?;
        smoke::run_cli_smoke_test(&host)?;
        return Ok(());
    }

    all_in_one_host::run();
    Ok(())
}
