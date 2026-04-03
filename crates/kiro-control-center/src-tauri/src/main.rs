// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Set a connect timeout to prevent infinite hangs when SSH port 22 is
    // blocked by a firewall.
    #[allow(unsafe_code)]
    // SAFETY: called once at startup before any concurrent git operations.
    unsafe {
        if let Err(e) = git2::opts::set_server_connect_timeout_in_milliseconds(
            kiro_market_core::git::CONNECT_TIMEOUT_MS,
        ) {
            eprintln!("warning: failed to set git connect timeout (SSH may hang): {e}");
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting Kiro Control Center");
    kcc_lib::run();
}
