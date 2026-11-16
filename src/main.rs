mod core;
mod ui;

use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "io.github.AverniteDF.Notediff";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(ui::initialize);

    app.connect_activate(|app| {
        ui::window::open_empty_window(app);
    });

    app.connect_open(|app, files, _hint| {
        if files.is_empty() {
            ui::window::open_empty_window(app);
            return;
        }

        for file in files {
            let Some(path) = file.path() else {
                continue;
            };
            ui::window::open_window_for_path(app, &path);
        }
    });

    // Pass argv through so GTK can parse its own options.
    let args: Vec<String> = std::env::args().collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    app.run_with_args(&refs)
}
