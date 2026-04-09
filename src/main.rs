#![no_std]
#![no_main]

use core::u8;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, Config, InterruptHandler};
use embassy_rp::peripherals::I2C0;
use embassy_time::Timer;
use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*};
use {defmt_rtt as _, panic_probe as _};

mod figures;

// Program metadata for `picotool info`.
// This isn't needed, but it's recommended to have these minimal entries.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"AJS37 Data Panel"),
    embassy_rp::binary_info::rp_program_description!(c"DCS Peripheral Firmware"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

bind_interrupts!(struct Irqs {
    I2C0_IRQ => InterruptHandler<I2C0>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    // initialize LED output
    let led = Output::new(p.PIN_25, Level::Low);

    // initialize I2C
    let sda = p.PIN_0;
    let scl = p.PIN_1;
    info!("set up i2c ");
    let i2c = i2c::I2c::new_async(p.I2C0, scl, sda, Irqs, Config::default());

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306Async::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().await.unwrap();
    let mut buffer = [0u8; 512];
    draw_figure(&mut buffer, 0, &figures::ONE);
    draw_figure(&mut buffer, 1, &figures::SIX);
    draw_figure(&mut buffer, 2, &figures::NINE);
    draw_figure(&mut buffer, 3, &figures::EIGHT);
    draw_figure(&mut buffer, 4, &figures::NONE);
    draw_figure(&mut buffer, 5, &figures::ZERO_BOLD);
    display.draw(&buffer).await.unwrap();

    // spawn tasks
    spawner.spawn(led_task(led)).unwrap();

    // main loop
    let mut i = 0;
    loop {
        Timer::after_millis(1000).await;
        let brightness = match i {
            0 => Brightness::DIMMEST,
            1 => Brightness::DIM,
            2 => Brightness::NORMAL,
            3 => Brightness::BRIGHT,
            _ => Brightness::BRIGHTEST,
        };
        let figure = match i {
            0 => &figures::ZERO,
            1 => &figures::ZERO_MED,
            2 => &figures::ZERO_BOLD,
            3 => &figures::ZERO_MED,
            _ => &figures::ZERO,
        };
        draw_figure(&mut buffer, 0, figure);
        display.draw(&buffer).await.unwrap();
        display.set_brightness(brightness).await.unwrap();
        i += 1;
        if i > 4 {
            i = 0;
        }
    }
}

#[embassy_executor::task]
async fn led_task(mut led: Output<'static>) -> ! {
    loop {
        led.toggle();
        Timer::after_millis(500).await;
    }
}

pub fn draw_figure(buffer: &mut [u8], index: usize, figure: &[u8]) {
    let offset = index * 21 + 1;
    for i in 0..21 {
        buffer[offset + i] = figure[i];
        buffer[offset + i + 128] = figure[i + 21];
        buffer[offset + i + 256] = figure[i + 42];
        buffer[offset + i + 384] = figure[i + 63];
    }
}
