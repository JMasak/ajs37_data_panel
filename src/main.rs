#![no_std]
#![no_main]

use crate::figures::FIGURES;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, Config};
use embassy_rp::peripherals::{I2C0, I2C1, PIO0, USB};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pio_programs::stepper::{PioStepper, PioStepperProgram};
use embassy_rp::usb::{self, Driver};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{self, CdcAcmClass};
use embassy_usb::driver::EndpointError;
use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*};
use static_cell::StaticCell;
//use uln2003::StepperMotor;

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
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    // initialize LED output
    let led = Output::new(p.PIN_25, Level::Low);

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
        draw_figure(&mut buffer, i, FIGURES[8]);
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

    let Pio {
        mut common,
        irq0,
        sm0,
        ..
    } = Pio::new(p.PIO0, Irqs);
    let prg = PioStepperProgram::new(&mut common);
    let mut stepper = PioStepper::new(
        &mut common,
        sm0,
        irq0,
        p.PIN_6,
        p.PIN_7,
        p.PIN_8,
        p.PIN_9,
        &prg,
    );

    let mut freq = 100;
    let mut steps = 2048;

    stepper.set_frequency(freq);

    stepper.step(steps).await;
    stepper.step_half(-steps).await;

    // let mut uln2003 = uln2003::ULN2003::new(
    //     Output::new(p.PIN_6, Level::Low),
    //     Output::new(p.PIN_7, Level::Low),
    //     Output::new(p.PIN_8, Level::Low),
    //     Output::new(p.PIN_9, Level::Low),
    //     Some(embassy_time::Delay {}),
    // );

    // uln2003.set_direction(uln2003::Direction::Normal);
    // let _ = uln2003.step_for(4096, 2);
    // uln2003.set_direction(uln2003::Direction::Reverse);
    // let _ = uln2003.step_for(4096, 2);

    // spawn tasks
    //spawner.spawn(led_task(led)).unwrap();
    spawner.spawn(usb_task(usb)).unwrap();
    spawner.spawn(serial_task(class)).unwrap();
    //spawner.spawn(waypoint_task(waypoint_display)).unwrap();

    // main loop
    loop {
        let data = FIGURE_SIGNAL.wait().await;
        for i in 0..6 {
            draw_figure(&mut buffer, i, FIGURES[data[i] as usize]);
        }
        display.draw(&buffer).await.unwrap();
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
        for i in 0..1024 {
            waypoint_buffer[i] = 0xAA;
            waypoint_display.draw(&waypoint_buffer).await.unwrap();
            if i % 128 == 0 {
                info!("Waypoint drawing new line: {}", i);
            }
        }
        waypoint_buffer.fill(0);
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
