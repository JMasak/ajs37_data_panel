use macroquad::{color::Color, shapes::draw_rectangle};
use simple_string_patterns::alphanumeric::StripCharacters;

const SIZE_X: usize = 21;
const SIZE_Y: usize = 32;
const TILE_SIZE: usize = 8;

pub struct EditorGrid {
    start_x: f32,
    start_y: f32,
    scale: f32,
    pixels: [bool; SIZE_X * SIZE_Y],
    background: Color,
    foreground_active: Color,
    foreground_inactive: Color,
}

impl EditorGrid {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        EditorGrid {
            start_x: x,
            start_y: y,
            scale: EditorGrid::calc_scale(width, height),
            ..Default::default()
        }
    }

    pub fn resize(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.start_x = x;
        self.start_y = y;
        self.scale = EditorGrid::calc_scale(width, height);
    }

    pub fn draw(&self) {
        let (min_width, min_height) = EditorGrid::calc_min_size();

        draw_rectangle(
            self.start_x,
            self.start_y,
            min_width * self.scale,
            min_height * self.scale,
            self.background,
        );

        for y in 0..SIZE_Y {
            for x in 0..SIZE_X {
                draw_rectangle(
                    self.start_x + self.scale + (x * (TILE_SIZE + 1)) as f32 * self.scale,
                    self.start_y + self.scale + (y * (TILE_SIZE + 1)) as f32 * self.scale,
                    TILE_SIZE as f32 * self.scale,
                    TILE_SIZE as f32 * self.scale,
                    if self.pixels[x + y * SIZE_X] {
                        self.foreground_active
                    } else {
                        self.foreground_inactive
                    },
                );
            }
        }
    }

    pub fn click(&mut self, posx: f32, posy: f32) {
        let (min_width, min_height) = EditorGrid::calc_min_size();
        if posx > self.start_x
            && posx < (self.start_x + min_width * self.scale)
            && posy > self.start_y
            && posy < (self.start_y + min_height * self.scale)
        {
            let x = ((posx - self.start_x) / ((TILE_SIZE + 1) as f32 * self.scale)) as usize;
            let y = ((posy - self.start_y) / ((TILE_SIZE + 1) as f32 * self.scale)) as usize;
            self.pixels[x + y * SIZE_X] = !self.pixels[x + y * SIZE_X];
        }
    }

    const fn calc_min_size() -> (f32, f32) {
        let min_width = (SIZE_X * TILE_SIZE + SIZE_X + 1) as f32;
        let min_height = (SIZE_Y * TILE_SIZE + SIZE_Y + 1) as f32;
        (min_width, min_height)
    }

    fn calc_scale(width: f32, height: f32) -> f32 {
        let (min_width, min_height) = EditorGrid::calc_min_size();
        let scale_width = width / min_width;
        let scale_height = height / min_height;
        let mut scale = scale_width.min(scale_height);
        if scale < 1.0 {
            scale = 1.0
        }
        scale
    }

    pub fn get_data(&self) -> String {
        let mut data = String::new();
        let mut initial = true;
        data.push_str("[");
        for y in 0..SIZE_Y / 8 {
            for x in 0..SIZE_X {
                let mut value = 0u8;
                for i in 0..8 {
                    if self.pixels[x + (y * 8 + i) * SIZE_X] {
                        value |= 1 << i;
                    }
                }
                if !initial {
                    data.push_str(",");
                }
                data.push_str(&format!("{}", value));
                initial = false;
            }
        }
        data.push_str("]");
        data
    }

    pub fn set_data(&mut self, data: &str) {
        println!("{}", data);

        let mut values: Vec<u8> = Vec::new();
        for valstr in data.split(',') {
            let valstr = valstr.strip_non_digits();
            println!("{}", valstr);
            let value = u8::from_str_radix(&valstr, 10).unwrap_or(0);
            values.push(value);
        }
        println!("{:#?}", values);
        for y in 0..SIZE_Y / 8 {
            for x in 0..SIZE_X {
                let value = values.get(x + y * SIZE_X).copied().unwrap_or(0);
                for i in 0..8 {
                    self.pixels[x + (y * 8 + i) * SIZE_X] = (value & (1 << i)) != 0;
                }
            }
        }
    }
}

impl Default for EditorGrid {
    fn default() -> Self {
        EditorGrid {
            start_x: 0.0,
            start_y: 0.0,
            scale: 1.0,
            pixels: [false; SIZE_X * SIZE_Y],
            background: Color {
                r: 0.2,
                g: 0.2,
                b: 0.2,
                a: 1.0,
            },
            foreground_active: Color {
                r: 0.8,
                g: 0.8,
                b: 0.8,
                a: 1.0,
            },
            foreground_inactive: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        }
    }
}
