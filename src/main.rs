#![no_std]
#![no_main]

use core::fmt::Write;
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
    buffer[0] = 0xaa;
    buffer[129] = 0xaa;
    buffer[256] = 0xaa;
    buffer[129 + 256] = 0xaa;
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
