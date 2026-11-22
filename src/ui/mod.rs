pub mod about_dialog;
pub mod app_dialog;
pub mod dialogs;
pub mod diff_page;
mod diff_toolbar;
pub mod find_in_files_dialog;
pub(crate) mod gutter_selection;
pub mod run_script_dialog;
pub mod status_bar;
pub mod theme;
mod theme_catalog;
mod theme_palette;
pub mod toast;
pub mod window;

/// Shorten paths under home for display without changing their identity.
fn display_path(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Format a local timestamp as `YYYY-MM-DD HH:MM`, returning `None` for
/// pre-epoch or unrepresentable times.
pub(crate) fn format_local_time(at: std::time::SystemTime) -> Option<String> {
    let secs = at.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let secs = i64::try_from(secs).ok()?;
    let date = gtk::glib::DateTime::from_unix_local(secs).ok()?;
    date.format("%Y-%m-%d %H:%M")
        .ok()
        .map(|text| text.to_string())
}

pub fn initialize(app: &adw::Application) {
    theme::install();
    window::initialize(app);
    theme::apply_appearance(crate::core::appearance::Appearance::load_from(
        &crate::core::appearance::Appearance::default_path(),
    ));
    theme::install_css();
}
