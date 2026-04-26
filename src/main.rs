#![no_std]
#![no_main]

use crate::figures::FIGURES;
use core::u8;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, Config};
use embassy_rp::peripherals::{I2C0, USB};
use embassy_rp::usb::{self, Driver};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{self, CdcAcmClass};
use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*};
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

mod figures;

type MyUsbDriver = usb::Driver<'static, USB>;
type MyUsbDevice = embassy_usb::UsbDevice<'static, MyUsbDriver>;

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
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
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

    // initialize USB
    // Create the driver, from the HAL.
    let usb_driver = usb::Driver::new(p.USB, Irqs);

    // Create embassy-usb Config
    let usb_config = {
        let mut config = embassy_usb::Config::new(0xdead, 0xbeef);
        config.manufacturer = Some("Meins");
        config.product = Some("AJS37 Data Panel");
        config.serial_number = Some("08154711");
        config.max_power = 100;
        config.max_packet_size_0 = 64;
        config
    };

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut builder = {
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

        let builder = embassy_usb::Builder::new(
            usb_driver,
            usb_config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [], // no msos descriptors
            CONTROL_BUF.init([0; 64]),
        );
        builder
    };

    // Create classes on the builder.
    let mut class = {
        static STATE: StaticCell<cdc_acm::State> = StaticCell::new();
        let state = STATE.init(cdc_acm::State::new());
        cdc_acm::CdcAcmClass::new(&mut builder, state, 64)
    };

    // Build the builder.
    let usb = builder.build();

    // spawn tasks
    spawner.spawn(led_task(led)).unwrap();
    spawner.spawn(usb_task(usb)).unwrap();
    spawner.spawn(serial_task(class)).unwrap();

    // main loop
    let mut count = 999993;
    loop {
        draw_value(&mut buffer, count);
        display.draw(&buffer).await.unwrap();
        count += 1;
        Timer::after_millis(250).await;
    }
}

#[embassy_executor::task]
async fn led_task(mut led: Output<'static>) -> ! {
    loop {
        led.toggle();
        Timer::after_millis(500).await;
    }
}

#[embassy_executor::task]
async fn usb_task(mut usb: MyUsbDevice) -> ! {
    usb.run().await
}

#[embassy_executor::task]
async fn serial_task(mut class: CdcAcmClass<'static, Driver<'static, USB>>) -> ! {
    let mut input_buffer = [0u8; 256];
    loop {
        class.wait_connection().await;
        if let Ok(size) = class.read_packet(&mut input_buffer).await
            && size > 0
        {
            info!("Received {} bytes", size);
            let _ = class.write_packet(&size.to_le_bytes()).await;
        }
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

fn draw_value(buffer: &mut [u8], value: u32) {
    let mut first = true;
    let mut current = value % 1000000;
    let mut i: usize = 0;
    while i < 6 {
        let divisor = 10_u32.pow(5 - i as u32) as u32;
        let digit = current / divisor;
        current -= digit * divisor;
        let glyph = {
            if digit == 0 {
                if first {
                    if i == 5 { FIGURES[0] } else { FIGURES[10] }
                } else {
                    FIGURES[0]
                }
            } else {
                first = false;
                FIGURES[digit as usize]
            }
        };
        draw_figure(buffer, i, glyph);
        i += 1;
    }
}
