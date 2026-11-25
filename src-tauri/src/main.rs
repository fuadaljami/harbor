#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

fn main() {
    if let Err(err) = harbor_lib::run() {
        eprintln!("Harbor failed to start: {err}");
        std::process::exit(1);
    }
}
