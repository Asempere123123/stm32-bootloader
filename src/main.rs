#![no_std]
#![no_main]

use core::panic::PanicInfo;

use embassy_stm32::{
    pac,
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

        let rcc = pac::RCC;
        // Enable HSI
        rcc.cr().modify(|w| w.set_hsion(true));
        while !rcc.cr().read().hsirdy() {}

        // Switch SYSCLK to HSI
        rcc.cfgr().modify(|w| w.set_sw(pac::rcc::vals::Sw::HSI));
        while rcc.cfgr().read().sws() != pac::rcc::vals::Sw::HSI {}

        // Disable PLL, HSE, CSS
        rcc.cr().modify(|w| {
            w.set_pllon(false);
            w.set_hseon(false);
            w.set_csson(false);
        });

        // Reset peripheral clock registers (AHB/APB enable registers)
        // This turns off the clocks for all GPIOs, DMAs, etc.
        rcc.ahb1enr().write(|w| w.0 = 0);
        rcc.ahb2enr().write(|w| w.0 = 0);
        rcc.apb1enr().write(|w| w.0 = 0);
        rcc.apb2enr().write(|w| w.0 = 0);

        // TIM1
        pac::TIM1.cr1().modify(|w| w.set_cen(false));
        pac::TIM1.bdtr().modify(|w| w.set_moe(false));
        rcc.apb2rstr().modify(|w| w.set_tim1rst(true));
        rcc.apb2rstr().modify(|w| w.set_tim1rst(false));
        rcc.apb2enr().modify(|w| w.set_tim1en(false));

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
        unsafe { cortex_m::interrupt::enable() };
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
