// draw or delete single pixels by clicking on it
// Ctrl+S copies the current glyph as an array to the clipboard
// Ctrl+D pastes the clipboard contents into the editor !!!Caution!!! not having a valid array in clipboard will delete the current glyph
// Ctrl+[Left, Right, Up, Down] shift buffer

use arboard::Clipboard;
use macroquad::{input, prelude::*};

mod editor_grid;

#[macroquad::main("Char Edit")]
async fn main() {
    let mut width = screen_width();
    let mut height = screen_height();
    let mut editor = editor_grid::EditorGrid::new(0.0, 0.0, width, height);
    let mut clipboard = Clipboard::new().unwrap();

    loop {
        clear_background(BLACK);

        if screen_width() != width || screen_height() != height {
            width = screen_width();
            height = screen_height();
            editor.resize(0.0, 0.0, width, height);
        }

        editor.draw();
        draw_text("CTRL+S - copy to clipboard", 20.0, 20.0, 20.0, WHITE);
        draw_text("CTRL+D - paste from clipboards", 20.0, 40.0, 20.0, WHITE);
        draw_text("CTRL+ArrowKeys - shift buffer", 20.0, 60.0, 20.0, WHITE);

        if input::is_mouse_button_pressed(input::MouseButton::Left) {
            let (posx, posy) = input::mouse_position();
            editor.click(posx, posy);
        }

        if input::is_key_down(input::KeyCode::LeftControl) {
            if input::is_key_pressed(input::KeyCode::S) {
                clipboard.set_text(editor.get_data()).unwrap();
            }
            if input::is_key_pressed(input::KeyCode::D) {
                editor.set_data(&clipboard.get_text().unwrap_or_default());
            }
            if input::is_key_pressed(input::KeyCode::Left) {
                editor.shift_left();
            }
            if input::is_key_pressed(input::KeyCode::Right) {
                editor.shift_right();
            }
            if input::is_key_pressed(input::KeyCode::Up) {
                editor.shift_up();
            }
            if input::is_key_pressed(input::KeyCode::Down) {
                editor.shift_down();
            }
        }

        next_frame().await
    }
}
