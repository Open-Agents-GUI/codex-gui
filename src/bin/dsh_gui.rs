#![cfg_attr(target_family = "wasm", no_main)]

use clap::Parser;
use codex_gui::{init_tracing, run_dsh_app};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(about = "Native GPUI client for DeepSeek Harness")]
struct Cli {
    /// Run dsh from a DeepSeek Harness source checkout with `pnpm dsh`.
    #[arg(long, value_name = "PATH")]
    dsh_repo: Option<PathBuf>,

    /// Installed dsh executable to run when --dsh-repo is not supplied.
    #[arg(long, value_name = "PATH", default_value = "dsh")]
    dsh_bin: PathBuf,

    /// dsh profile containing the Codex app-server adapter.
    #[arg(long, default_value = "app-server")]
    profile: String,
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    let cli = Cli::parse();
    let launch = match cli.dsh_repo {
        Some(repo) => codex_gui::DshLaunchConfig::from_repository(repo, cli.profile),
        None => codex_gui::DshLaunchConfig::from_executable(cli.dsh_bin, cli.profile),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("failed to create dsh runtime");

    init_tracing();
    run_dsh_app(runtime.handle().clone(), launch);
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    panic!("the dsh process transport is not available on wasm");
}
