pub mod about_dialog;
pub mod app_dialog;
pub mod dialogs;
pub mod diff_page;
mod diff_toolbar;
pub(crate) mod gutter_selection;
pub mod status_bar;
pub mod theme;
mod theme_catalog;
mod theme_palette;
pub mod toast;
pub mod window;

pub fn initialize(app: &adw::Application) {
    theme::install();
    window::initialize(app);
    theme::apply_appearance(crate::core::appearance::Appearance::load_from(
        &crate::core::appearance::Appearance::default_path(),
    ));
    theme::install_css();
}
