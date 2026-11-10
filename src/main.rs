// Release builds on Windows are GUI apps: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    markdown_converter::ui::run()
}
