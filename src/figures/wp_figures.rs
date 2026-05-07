// Figures for the Waypoint Panel OLED display
// OLED Display has a resolution of 128 x 64 pixels
// Waypoint Panel displays 2 figures
// 128 / 2 = 64 pixels per figure
// resolution per figure: 64 pixels wide, 64 pixels tall
pub const FIGURE_WIDTH: usize = 64;
pub const FIGURE_HEIGHT: usize = 64;
pub const FIGURE_PIXELS_PER_BYTE: usize = 8;
pub const NONE: [u8; (FIGURE_WIDTH * FIGURE_HEIGHT) / FIGURE_PIXELS_PER_BYTE] =
    [0; (FIGURE_WIDTH * FIGURE_HEIGHT) / FIGURE_PIXELS_PER_BYTE];
