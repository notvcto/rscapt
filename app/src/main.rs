// Suppress console window in release builds on Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rscapt_app_lib::run()
}
