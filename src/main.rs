#![no_std]
#![no_main]

use core::panic::PanicInfo;

use embassy_stm32::{
    usart::{Config, Uart},
    Peripherals,
};

use embassy_time::{Duration, Timer};

#[cfg(feature = "defmt")]
use defmt_rtt as _;

unsafe extern "C" {
    static _app_vector_table: u32;
}

#[panic_handler]
#[allow(unused)]
fn panic(info: &PanicInfo) -> ! {
    // TODO: En un futuro esto usa la implementacion por can
    #[cfg(feature = "defmt")]
    defmt::error!("BOOTLOADER PANIC: {}", info);
    // (hardfault)
    cortex_m::asm::udf()
}

#[cfg(not(feature = "hse"))]
fn init_hal() -> Peripherals {
    let peripherals = embassy_stm32::init(embassy_stm32::Config::default());
    peripherals
}

#[cfg(feature = "hse")]
fn init_hal() -> Peripherals {
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(embassy_stm32::rcc::Hse {
        #[rustfmt::skip]
        freq: embassy_stm32::time::Hertz({{ hse-freq }}),
        mode: embassy_stm32::rcc::HseMode::Oscillator,
    });
    config.rcc.sys = embassy_stm32::rcc::Sysclk::HSE;
    let peripherals = embassy_stm32::init(config);
    peripherals
}

fn bootloader() {
    let peripherals = init_hal();

    #[cfg(feature = "defmt")]
    {
        defmt::info!("Running bootloader");
        embassy_time::block_for(Duration::from_millis(500));
        let mut uart_cfg = Config::default();
        uart_cfg.baudrate = 9600;
        let mut uart = Uart::new_blocking(
            peripherals.USART1,
            peripherals.PA10,
            peripherals.PA9,
            uart_cfg,
        )
        .unwrap();

        defmt::info!("WAITING");
        let mut byte = [0];
        //let res = uart.blocking_read(&mut byte);
        //defmt::info!("{:?}", res);
        defmt::info!("BYTE: RECV: {}", byte);
    }
}

#[cortex_m_rt::entry]
fn main() -> ! {
    bootloader();

    let core_peri = cortex_m::Peripherals::take().unwrap();

    #[cfg(feature = "defmt")]
    {
        defmt::info!("Jumping to APP");
        defmt::flush();
        // On defmt mode we want to not jump to the app and only debug the bootloader
        loop {
            cortex_m::asm::wfi();
        }
    }
    #[allow(unreachable_code)]
    {
        cortex_m::interrupt::disable();

        embassy_stm32::rcc::reinit(embassy_stm32::rcc::Config::default(), unsafe {
            &mut embassy_stm32::peripherals::RCC::steal()
        });
        embassy_stm32::rcc::enable_and_reset::<embassy_stm32::peripherals::TIM1>();
        embassy_stm32::rcc::disable::<embassy_stm32::peripherals::TIM1>();

        // NVIC
        for i in 0..8 {
            unsafe {
                core_peri.NVIC.icer[i].write(0xFFFF_FFFF);
            }
            unsafe {
                core_peri.NVIC.icpr[i].write(0xFFFF_FFFF);
            }
        }
        // Barriers
        cortex_m::asm::dsb();
        cortex_m::asm::isb();

        // Do a bound check on isp and reset vector
        unsafe {
            core_peri
                .SCB
                .vtor
                .write(&_app_vector_table as *const _ as u32);
            cortex_m::asm::dsb();
            cortex_m::asm::isb();

            cortex_m::asm::bootload(&_app_vector_table);
        }
        panic!("App returned");
    }
}
