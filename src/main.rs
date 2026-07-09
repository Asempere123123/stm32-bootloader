#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[cfg(feature = "can")]
mod can;

#[cfg(feature = "fdcan")]
mod fdcan;

#[cfg(feature = "rtt")]
mod rtt;

#[cfg(feature = "rtt-host-to-chip")]
mod host_to_chip;

unsafe extern "C" {
    static _app_vector_table: u32;
}

#[panic_handler]
#[allow(unused)]
fn panic(info: &PanicInfo) -> ! {
    #[cfg(feature = "defmt")]
    defmt::error!("BOOTLOADER PANIC: {}", info);
    // (hardfault)
    cortex_m::asm::udf()
}

fn get_hal_config() -> embassy_stm32::Config {
    #[allow(unused)]
    let mut config = embassy_stm32::Config::default();

    #[cfg(feature = "hse")]
    {
        config.rcc.hse = Some(embassy_stm32::rcc::Hse {
            #[rustfmt::skip]
            freq: embassy_stm32::time::Hertz({{ hse-freq }}),
            mode: embassy_stm32::rcc::HseMode::Oscillator,
        });
        config.rcc.sys = embassy_stm32::rcc::Sysclk::HSE;
    }

    {% assign chip-family = chip-hal-name | slice: 0, 7 %}
    {% if chip-family == "stm32h7" -%}
    #[cfg(all(not(feature = "hse"), feature = "fdcan"))]
    {
        config.rcc.mux.fdcansel = embassy_stm32::rcc::mux::Fdcansel::PLL1_Q;
        config.rcc.pll1 = Some(embassy_stm32::rcc::Pll {
            source: embassy_stm32::rcc::PllSource::HSI,
            prediv: embassy_stm32::rcc::PllPreDiv::DIV8,
            mul: embassy_stm32::rcc::PllMul::MUL50,
            divp: Some(embassy_stm32::rcc::PllDiv::DIV2),
            divq: Some(embassy_stm32::rcc::PllDiv::DIV10),
            divr: None,
        });
    }
    {% endif -%}

    #[cfg(feature = "smps_power")]
    {
        config.rcc.supply_config = embassy_stm32::rcc::SupplyConfig::DirectSMPS;
    }

    config
}

fn bootloader() {
    #[cfg(feature = "rtt")]
    let rtt = crate::rtt_init!();
    #[cfg(feature = "defmt")]
    rtt_target::set_defmt_channel(rtt.up.0);
    #[cfg(feature = "rtt-host-to-chip")]
    #[allow(unused)]
    let mut host_to_chip = host_to_chip::HostToChip::new(rtt.down.0);

    #[cfg(feature = "rtt-host-to-chip")]
    host_to_chip.echo_loop();

    #[allow(unused)]
    let mut peripherals = embassy_stm32::init(get_hal_config());

    #[cfg(feature = "can")]
    embassy_futures::block_on(can::can_flashing(&mut peripherals));

    #[cfg(feature = "fdcan")]
    embassy_futures::block_on(fdcan::fdcan_flashing(&mut peripherals));

    let _ = peripherals;
}

#[cortex_m_rt::entry]
fn main() -> ! {
    bootloader();

    let Some(core_peri) = cortex_m::Peripherals::take() else {
        panic!();
    };

    #[cfg(feature = "defmt")]
    {
        let _ = core_peri;
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

        let mut rcc = unsafe { embassy_stm32::peripherals::RCC::steal() };
        embassy_stm32::rcc::reinit(embassy_stm32::rcc::Config::default(), &mut rcc);
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

        // TODO: Do a bound check on isp and reset vector
        unsafe {
            core_peri
                .SCB
                .vtor
                .write(&_app_vector_table as *const _ as u32);
            cortex_m::asm::dsb();
            cortex_m::asm::isb();

            cortex_m::interrupt::enable();
            cortex_m::asm::bootload(&_app_vector_table);
        }
        panic!("App returned");
    }
}
