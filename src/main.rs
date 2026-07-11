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

#[cfg(feature = "external-flash")]
mod external_flash;

#[cfg(feature = "external-macronix-octo-spi-flash")]
mod macronix_octo_spi;

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

fn get_default_rcc_cfg() -> embassy_stm32::rcc::Config {
    #[allow(unused)]
    let mut cfg = embassy_stm32::rcc::Config::default();

    {% assign chip-family = chip-hal-name | slice: 0, 7 %}
    {% if chip-family == "stm32h7" -%}
    #[cfg(feature = "external-macronix-octo-spi-flash")]
    {
        cfg.mux.octospisel = embassy_stm32::rcc::mux::Fmcsel::PLL1_Q;
        cfg.pll1 = Some(embassy_stm32::rcc::Pll {
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
        cfg.supply_config = embassy_stm32::rcc::SupplyConfig::DirectSMPS;
    }

    cfg
}

fn get_hal_config() -> embassy_stm32::Config {
    let mut config = embassy_stm32::Config::default();
    config.rcc = get_default_rcc_cfg();

    #[cfg(feature = "hse")]
    {
        config.rcc.hse = Some(embassy_stm32::rcc::Hse {
            #[rustfmt::skip]
            freq: embassy_stm32::time::Hertz({{ hse-freq }}),
            mode: embassy_stm32::rcc::HseMode::Oscillator,
        });

        {% assign chip-family = chip-hal-name | slice: 0, 7 %}
        {% if chip-family != "stm32h7" -%}
        config.rcc.sys = embassy_stm32::rcc::Sysclk::HSE;
        {% endif -%}
    }

    {% assign chip-family = chip-hal-name | slice: 0, 7 %}
    {% if chip-family == "stm32h7" -%}
    #[cfg(all(not(feature = "hse"), feature = "fdcan"))]
    {
        config.rcc.mux.fdcansel = embassy_stm32::rcc::mux::Fdcansel::PLL1_Q;
        if config.rcc.pll1.is_none() {
            config.rcc.pll1 = Some(embassy_stm32::rcc::Pll {
                source: embassy_stm32::rcc::PllSource::HSI,
                prediv: embassy_stm32::rcc::PllPreDiv::DIV8,
                mul: embassy_stm32::rcc::PllMul::MUL50,
                divp: Some(embassy_stm32::rcc::PllDiv::DIV2),
                divq: Some(embassy_stm32::rcc::PllDiv::DIV10),
                divr: None,
            });
        }
    }
    {% endif -%}

    config
}

fn bootloader() {
    #[cfg(feature = "rtt")]
    let rtt = crate::rtt_init!();
    #[cfg(feature = "defmt")]
    rtt_target::set_defmt_channel(rtt.2);
    #[cfg(feature = "rtt-host-to-chip")]
    #[allow(unused)]
    let mut host_to_chip = host_to_chip::HostToChip::new(rtt.1, rtt.0);

    #[allow(unused)]
    let mut peripherals = embassy_stm32::init(get_hal_config());

    #[cfg(feature = "external-flash")]
    embassy_futures::block_on(external_flash::flash_from_debugger(
        &mut peripherals,
        &mut host_to_chip,
    ));

    #[cfg(feature = "can")]
    embassy_futures::block_on(can::can_flashing(&mut peripherals));

    #[cfg(feature = "fdcan")]
    embassy_futures::block_on(fdcan::fdcan_flashing(&mut peripherals));

    #[cfg(feature = "external-flash")]
    external_flash::enable_memory_mapped_mode(&mut peripherals);

    let _ = peripherals;
}

#[cortex_m_rt::entry]
fn main() -> ! {
    bootloader();

    #[allow(unused_mut)]
    let Some(mut core_peri) = cortex_m::Peripherals::take() else {
        panic!("Failed to take core peripherals");
    };

    cortex_m::interrupt::disable();

    // Reset RCC
    let mut rcc = unsafe { embassy_stm32::peripherals::RCC::steal() };
    embassy_stm32::rcc::reinit(get_default_rcc_cfg(), &mut rcc);
    embassy_stm32::rcc::enable_and_reset::<embassy_stm32::peripherals::TIM1>();
    embassy_stm32::rcc::disable::<embassy_stm32::peripherals::TIM1>();

    // Clear NVIC interrupts
    for i in 0..8 {
        unsafe {
            core_peri.NVIC.icer[i].write(0xFFFF_FFFF);
            core_peri.NVIC.icpr[i].write(0xFFFF_FFFF);
        }
    }

    #[cfg(feature = "external-flash")]
    configure_mpu_external_flash(&mut core_peri);

    unsafe {
        core_peri.SCB.vtor.write(&_app_vector_table as *const _ as u32);

        cortex_m::asm::dsb();
        cortex_m::asm::isb();

        cortex_m::interrupt::enable();

        #[cfg(feature = "defmt")]
        {
            defmt::info!("Jumping to APP");
            defmt::flush();
            loop { cortex_m::asm::wfi(); }
        }

        #[allow(unreachable_code)]
        {
            cortex_m::asm::bootload(&_app_vector_table as *const _ as *const u32);
            panic!("App returned");
        }
    }
}

#[cfg(feature = "external-flash")]
fn configure_mpu_external_flash(core_peri: &mut cortex_m::Peripherals) {
    let mpu = &core_peri.MPU;
    let scb = &mut core_peri.SCB;
    let cpuid = &mut core_peri.CPUID;

    const FLASH_ADDR: u32 = {{ external-flash-addr }};
    const SIZE_CODE: u32 = 27; // 256MiB region

    unsafe {
        mpu.rnr.write(0); // Select MPU Region 0
        mpu.rbar.write(FLASH_ADDR); // Base address

        // Construct the Region Attribute and Size Register (RASR)
        let rasr = (0 << 28) |        // XN = 0: Executable (Allows Execute-In-Place / XIP)
                    (0b011 << 24) |   // AP = 011: Full Access (Privileged & Unprivileged Read/Write)
                    (0b000 << 19) |   // TEX = 000: Base attributes
                    (0 << 18) |       // S = 0: Non-shareable (Standard for single-core normal memory)
                    (1 << 17) |       // C = 1: Cacheable (Massive read performance boost)
                    (0 << 16) |       // B = 0: Write-Through (Prevents cache eviction faults on flash)
                    (0 << 8)  |       // SRD = 0: Sub-Region Disable
                    (SIZE_CODE << 1)| // SIZE
                    1;                // ENABLE = 1: Turn this region on

        mpu.rasr.write(rasr);

        // Enable the MPU
        // Bit 2 (PRIVDEFENA) = 1: Enables default memory map for privileged access when no region matches.
        // Bit 0 (ENABLE) = 1: Enables the MPU globally.
        mpu.ctrl.modify(|r| r | 0x5);

        cortex_m::asm::dsb();
        cortex_m::asm::isb();
    }

    scb.enable_icache();
    scb.enable_dcache(cpuid);

    scb.clean_invalidate_dcache(cpuid);
    scb.invalidate_icache();

    cortex_m::asm::dsb();
    cortex_m::asm::isb();
}
