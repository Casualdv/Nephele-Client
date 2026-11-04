#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod app;
mod crash;
mod profiles;
mod updates;

fn main() {
    if let Err(error) = app::run() {
        #[cfg(windows)]
        let _ = rfd::MessageDialog::new()
            .set_title("Nephele Launcher")
            .set_description(error.to_string())
            .show();
        #[cfg(not(windows))]
        eprintln!("Nephele Launcher: {error}");
        std::process::exit(1);
    }
}
