#![no_std]
#![no_main]

use crate::figures::DP_FIGURES;
use core::ptr::addr_of_mut;
use cortex_m::asm::nop;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{self, Config};
use embassy_rp::multicore::Stack;
use embassy_rp::peripherals::{I2C0, I2C1, USB};
use embassy_rp::usb::{self, Driver};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{self, CdcAcmClass};
use embassy_usb::driver::EndpointError;
use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*};
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

mod dcs_bios;
mod figures;

type MyUsbDriver = usb::Driver<'static, USB>;
type MyUsbDevice = embassy_usb::UsbDevice<'static, MyUsbDriver>;

const USB_SERIAL_INPUT_BUFFER_SIZE: usize = 256;
const AJS37_DCS_BIOS_ADDRESS_START: u16 = 0x4600;
const AJS37_DCS_BIOS_ADDRESS_END: u16 = AJS37_DCS_BIOS_ADDRESS_START + 0x200;
const AJS37_NAV_INDICATOR_DATA_1: u16 = 0x46A8;
const AJS37_NAV_INDICATOR_DATA_2: u16 = 0x46AA;
const AJS37_NAV_INDICATOR_DATA_3: u16 = 0x46AC;
const AJS37_NAV_INDICATOR_DATA_4: u16 = 0x46AE;
const AJS37_NAV_INDICATOR_DATA_5: u16 = 0x46B0;
const AJS37_NAV_INDICATOR_DATA_6: u16 = 0x46B2;

static FIGURE_SIGNAL: Signal<ThreadModeRawMutex, [u8; 6]> = Signal::new(); // for multicore usage replace ThreadModeRawMutex with CriticalSectionRawMutex
static mut CORE1_STACK: Stack<4096> = Stack::new();

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
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    // initialize LED output
    let led = Output::new(p.PIN_25, Level::Low);
    // initialize buttons
    let dial_trigger = Input::new(p.PIN_21, Pull::Up);
    let _button = Input::new(p.PIN_20, Pull::Up);

    // initialize I2C0 -> DataPanelDisplay
    let sda = p.PIN_0;
    let scl = p.PIN_1;
    info!("set up i2c ");
    let i2c = i2c::I2c::new_async(p.I2C0, scl, sda, Irqs, Config::default());

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306Async::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().await.unwrap();
    let mut buffer = [0u8; 512];
    for i in 0..6 {
        draw_figure(&mut buffer, i, DP_FIGURES[8]);
    }
    display.draw(&buffer).await.unwrap();

    // initialize I2C1 -> WaypointDisplay
    let sda = p.PIN_2;
    let scl = p.PIN_3;
    let i2c = i2c::I2c::new_async(p.I2C1, scl, sda, Irqs, Config::default());
    let interface = I2CDisplayInterface::new(i2c);
    let mut waypoint_display =
        Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
    waypoint_display.init().await.unwrap();

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

        embassy_usb::Builder::new(
            usb_driver,
            usb_config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [], // no msos descriptors
            CONTROL_BUF.init([0; 64]),
        )
    };

    // Create classes on the builder.
    let class = {
        static STATE: StaticCell<cdc_acm::State> = StaticCell::new();
        let state = STATE.init(cdc_acm::State::new());
        cdc_acm::CdcAcmClass::new(&mut builder, state, 64)
    };

    // Build the builder.
    let usb = builder.build();

    let stepper_outputs = [
        Output::new(p.PIN_6, Level::Low),
        Output::new(p.PIN_7, Level::Low),
        Output::new(p.PIN_8, Level::Low),
        Output::new(p.PIN_9, Level::Low),
    ];

    embassy_rp::multicore::spawn_core1(
        p.CORE1,
        unsafe { &mut *addr_of_mut!(CORE1_STACK) },
        move || {
            core1_fn(stepper_outputs, dial_trigger);
        },
    );

    // spawn tasks
    spawner.spawn(led_task(led)).unwrap();
    spawner.spawn(usb_task(usb)).unwrap();
    spawner.spawn(serial_task(class)).unwrap();
    spawner.spawn(waypoint_task(waypoint_display)).unwrap();

    // main loop
    loop {
        let data = FIGURE_SIGNAL.wait().await;
        for i in 0..6 {
            draw_figure(&mut buffer, i, DP_FIGURES[data[i] as usize]);
        }
        display.draw(&buffer).await.unwrap();
    }
}

fn core1_fn(mut stepper_outputs: [Output<'static>; 4], dial_trigger: Input<'static>) -> ! {
    const PINS: usize = 4;
    const START_DELAY: usize = 3000;
    const MIN_DELAY: usize = 1800;
    const BRAKE_STEPS: usize = 200;
    const ACC_DELAY: usize = 1;
    const ACC_STEP: usize = (START_DELAY - MIN_DELAY) / BRAKE_STEPS;
    const STEPS: usize = 1500;
    let mut i = 0;
    let mut count = 0;
    let mut direction = false; // turn counter clockwise initially to reference with dial trigger
    let mut delay = START_DELAY;
    let mut acc_delay_count = 0;

    loop {
        if dial_trigger.is_low() {
            count = 0;
            acc_delay_count = 0;
            direction = true;
            delay = START_DELAY;
        }
        for pin in &mut stepper_outputs {
            pin.set_low();
        }
        if direction {
            if i < PINS - 1 {
                i += 1;
            } else {
                i = 0;
            }
        } else {
            if i > 0 {
                i -= 1;
            } else {
                i = PINS - 1;
            }
        }
        stepper_outputs[i].set_high();
        count += 1;
        acc_delay_count += 1;
        if acc_delay_count > ACC_DELAY {
            acc_delay_count = 0;
            if count < STEPS - (BRAKE_STEPS * ACC_DELAY) {
                if delay >= MIN_DELAY + ACC_STEP {
                    delay -= ACC_STEP;
                    //info!("accelerating: {}", delay);
                }
            } else {
                // break
                if delay <= START_DELAY - ACC_STEP {
                    delay += ACC_STEP;
                    //info!("braking: {}", delay);
                }
            }
        }
        if count > STEPS {
            count = 0;
            delay = START_DELAY;
            direction = !direction;
        }
        for _ in 0..delay {
            nop();
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

#[embassy_executor::task]
async fn waypoint_task(
    mut waypoint_display: Ssd1306Async<
        I2CInterface<i2c::I2c<'static, I2C1, i2c::Async>>,
        DisplaySize128x64,
        ssd1306::mode::BufferedGraphicsModeAsync<DisplaySize128x64>,
    >,
) -> ! {
    let mut c = 0;
    let mut waypoint_buffer = [0u8; 1024];
    waypoint_display
        .set_row(0)
        .await
        .expect("Could not set row");
    waypoint_display
        .set_column(0)
        .await
        .expect("Could not set column");
    loop {
        waypoint_buffer.fill(0);
        for i in 0..1024 {
            waypoint_buffer[i] = (i + c % 255) as u8;
            c += 1;
            if i % 128 == 0 {
                //info!("Waypoint drawing new line: {}", i);
            }
        }
        waypoint_display.draw(&waypoint_buffer).await.unwrap();
        Timer::after_millis(250).await;
    }
}

#[embassy_executor::task]
async fn usb_task(mut usb: MyUsbDevice) -> ! {
    usb.run().await
}

#[embassy_executor::task]
async fn serial_task(mut class: CdcAcmClass<'static, Driver<'static, USB>>) -> ! {
    let mut input_buffer = [0u8; USB_SERIAL_INPUT_BUFFER_SIZE];
    let mut receive_state = dcs_bios::ReceiveState::WaitingForSync;
    let mut read_offset = 0;
    let mut write_offset = 0;
    let mut data = [0u8; 6];
    loop {
        class.wait_connection().await;
        loop {
            match class
                .read_packet(&mut input_buffer[write_offset..USB_SERIAL_INPUT_BUFFER_SIZE])
                .await
            {
                Ok(size) => {
                    if size > 0 {
                        write_offset += size;
                        info!("Received {} bytes", size);
                        let _ = class.write_packet(&size.to_le_bytes()).await;
                    }
                }
                Err(e) => {
                    receive_state = dcs_bios::ReceiveState::WaitingForSync;
                    read_offset = 0;
                    write_offset = 0;
                    match e {
                        EndpointError::Disabled => {
                            break;
                        }
                        EndpointError::BufferOverflow => (),
                    }
                }
            }

            if receive_state == dcs_bios::ReceiveState::WaitingForSync {
                while write_offset > 4 && read_offset < (write_offset - 4) {
                    if check_for_start_of_frame(&input_buffer, read_offset, write_offset) {
                        receive_state = dcs_bios::ReceiveState::ReceivingAddress;
                        read_offset += 4;
                        break;
                    }
                    read_offset += 1;
                }
            }

            loop {
                match receive_state {
                    dcs_bios::ReceiveState::ReceivingAddress => {
                        // check for start of new frame
                        if check_for_start_of_frame(&input_buffer, read_offset, write_offset) {
                            //info!("sync frame during ReceivingAddress");
                            read_offset += 4;
                        }
                        if read_offset + 2 >= write_offset {
                            break;
                        }
                        let addr = u16::from_le_bytes([
                            input_buffer[read_offset],
                            input_buffer[read_offset + 1],
                        ]);
                        //info!("Received Address: 0x{:04x}", addr);
                        receive_state = dcs_bios::ReceiveState::ReceivingLength(addr);
                        read_offset += 2;
                    }
                    dcs_bios::ReceiveState::ReceivingLength(addr) => {
                        if check_for_start_of_frame(&input_buffer, read_offset, write_offset) {
                            error!("sync frame during ReceivingLength");
                        }
                        if read_offset + 2 >= write_offset {
                            break;
                        }
                        let len = u16::from_le_bytes([
                            input_buffer[read_offset],
                            input_buffer[read_offset + 1],
                        ]);
                        //info!("Received length: {} for address: 0x{:04x}", len, addr);
                        receive_state = dcs_bios::ReceiveState::ReceivingData((addr, len));
                        read_offset += 2;
                    }
                    dcs_bios::ReceiveState::ReceivingData((addr, len)) => {
                        if read_offset + len as usize >= write_offset {
                            break;
                        }
                        if (AJS37_DCS_BIOS_ADDRESS_START..AJS37_DCS_BIOS_ADDRESS_END)
                            .contains(&addr)
                        {
                            info!(
                                "Received AJS37DCS BIOS data: addr=0x{:04x}, len={}",
                                addr, len
                            );
                            let start_offset = match addr {
                                AJS37_NAV_INDICATOR_DATA_1 => Some(0),
                                AJS37_NAV_INDICATOR_DATA_2 => Some(1),
                                AJS37_NAV_INDICATOR_DATA_3 => Some(2),
                                AJS37_NAV_INDICATOR_DATA_4 => Some(3),
                                AJS37_NAV_INDICATOR_DATA_5 => Some(4),
                                AJS37_NAV_INDICATOR_DATA_6 => Some(5),
                                _ => None,
                            };
                            if let Some(offset) = start_offset {
                                for i in 0..len / 2 {
                                    let j = offset + i as usize;
                                    if j < data.len() {
                                        data[j] = get_figure_index(
                                            &input_buffer[read_offset + (i as usize) * 2],
                                        );
                                    }
                                }
                                FIGURE_SIGNAL.signal(data);
                                debug!("{:#?}", data);
                            }
                        }
                        receive_state = dcs_bios::ReceiveState::ReceivingAddress;
                        read_offset += len as usize;
                    }
                    dcs_bios::ReceiveState::WaitingForSync => break, // this is handled up front so we do not wait to receive another data package before processing data
                }
            }

            if read_offset > 0 {
                // shift the buffer to make room for the new data
                input_buffer.copy_within(read_offset..write_offset, 0);
                write_offset -= read_offset;
                read_offset = 0;
            }
        }
    }
}

fn get_figure_index(value: &u8) -> u8 {
    match value {
        0x30 => 0,
        0x31 => 1,
        0x32 => 2,
        0x33 => 3,
        0x34 => 4,
        0x35 => 5,
        0x36 => 6,
        0x37 => 7,
        0x38 => 8,
        0x39 => 9,
        _ => 10,
    }
}

fn check_for_start_of_frame(input_buffer: &[u8], read_offset: usize, write_offset: usize) -> bool {
    if read_offset < write_offset + 4
        && input_buffer[read_offset] == 0x55
        && input_buffer[read_offset + 1] == 0x55
        && input_buffer[read_offset + 2] == 0x55
        && input_buffer[read_offset + 3] == 0x55
    {
        return true;
    }
    false
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

#[allow(unused)]
fn draw_value(buffer: &mut [u8], value: u32) {
    let mut first = true;
    let mut current = value % 1000000;
    let mut i: usize = 0;
    while i < 6 {
        let divisor = 10_u32.pow(5 - i as u32);
        let digit = current / divisor;
        current -= digit * divisor;
        let glyph = {
            if digit == 0 {
                if first {
                    if i == 5 {
                        DP_FIGURES[0]
                    } else {
                        DP_FIGURES[10]
                    }
                } else {
                    DP_FIGURES[0]
                }
            } else {
                first = false;
                DP_FIGURES[digit as usize]
            }
        };
        draw_figure(buffer, i, glyph);
        i += 1;
    }
}
