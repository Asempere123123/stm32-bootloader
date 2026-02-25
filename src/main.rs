#![no_std]
#![no_main]

use core::panic::PanicInfo;

use cortex_m::delay::Delay;
use embassy_stm32::{
    rcc::HSI_FREQ,
    usart::{Config, Uart},
};

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

fn bootloader() {
    let core_peri = unsafe { cortex_m::Peripherals::steal() };
    let peripherals = embassy_stm32::init(embassy_stm32::Config::default());
    let mut delay = Delay::new(core_peri.SYST, HSI_FREQ.0);

    #[cfg(feature = "defmt")]
    {
        defmt::info!("Running bootloader");
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
        let res = uart.blocking_read(&mut byte);
        defmt::info!("{:?}", res);
        defmt::info!("BYTE: RECV: {}", byte);
    }

    delay.free();
}

#[cortex_m_rt::entry]
fn main() -> ! {
    bootloader();

    let core_peri = unsafe { cortex_m::Peripherals::steal() };

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
        for i in 0..8 {
            unsafe {
                core_peri.NVIC.icer[i].write(0xFFFF_FFFF);
            }
            unsafe {
                core_peri.NVIC.icpr[i].write(0xFFFF_FFFF);
            }
        }
        cortex_m::asm::dsb();
        cortex_m::asm::isb();

        // Rcc?¿??
        // Do a bound check on isp and reset vector
        unsafe {
            core_peri
                .SCB
                .vtor
                .write(&_app_vector_table as *const _ as u32);

            cortex_m::asm::bootload(&_app_vector_table);
        }
        panic!("App returned");
    }
}
