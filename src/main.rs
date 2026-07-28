#![cfg_attr(windows, windows_subsystem = "windows")]

mod totp;

#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod secret_store;
#[cfg(windows)]
mod windows_app;

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_app::run() {
        windows_app::show_fatal_error(&error);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("SIT TOTP for Windows is available only on Windows.");
}
