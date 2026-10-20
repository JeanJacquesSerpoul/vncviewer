mod address;
mod app;
mod i18n;
mod keysym;
mod prefs;
mod tauri;
mod viewer;
mod wire;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
