mod address;
mod app;
mod connect;
mod i18n;
mod icons;
mod keysym;
mod prefs;
mod secrets;
mod tauri;
mod toolbar;
mod viewer;
mod wire;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
