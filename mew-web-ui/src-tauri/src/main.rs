#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "macos")]
    if mew_cef_host::embed::run_subprocess_if_needed() {
        return;
    }

    mew_desktop_lib::run();
}
