#![no_std]
#![no_main]

use core::panic::PanicInfo;
use cortex_m::peripheral::MPU;

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

    #[cfg(feature = "external-macronix-octo-spi-flash")]
    {
        config.rcc.mux.octospisel = embassy_stm32::rcc::mux::Fmcsel::PLL1_Q;
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

    let Some(mut core_peri) = cortex_m::Peripherals::take() else {
        panic!("Failed to take core peripherals");
    };

    #[cfg(feature = "defmt")]
    {
        defmt::info!("Jumping to APP");
        defmt::flush();
        // NOTE: If you are debugging the bootloader and want it to halt here,
        // uncomment the loop below. Otherwise, it must be commented out to jump!
        // loop { cortex_m::asm::wfi(); }
    }

    // --- PREPARE FOR JUMP ---
    cortex_m::interrupt::disable();

    // 1. Reset specific peripherals if needed
    // embassy_stm32::rcc::enable_and_reset::<embassy_stm32::peripherals::TIM1>();
    // embassy_stm32::rcc::disable::<embassy_stm32::peripherals::TIM1>();

    // 2. Clear NVIC interrupts
    for i in 0..8 {
        unsafe {
            core_peri.NVIC.icer[i].write(0xFFFF_FFFF);
            core_peri.NVIC.icpr[i].write(0xFFFF_FFFF);
        }
    }

    // 3. Configure MPU and Caches for the OSPI region
    prepare_ospi_and_caches(&core_peri.MPU, &mut core_peri.SCB, &mut core_peri.CPUID);

    unsafe {
        // 4. Set Vector Table Offset Register
        core_peri.SCB.vtor.write(&_app_vector_table as *const _ as u32);

        cortex_m::asm::dsb();
        cortex_m::asm::isb();

        // 5. Final safety check
        if !is_executable(&_app_vector_table as *const _ as u32, &core_peri.MPU) {
            // If we hit this, the MPU configuration above failed to apply
            panic!("Target address is not executable!");
        }

        // 6. Enable interrupts and jump
        cortex_m::interrupt::enable();
        cortex_m::asm::bootload(&_app_vector_table as *const _ as *const u32);
    }
}

/// Configures the MPU for OSPI execution and ensures caches handle unaligned access properly.
fn prepare_ospi_and_caches(
    mpu: &MPU,
    scb: &mut cortex_m::peripheral::SCB,
    cpuid: &mut cortex_m::peripheral::CPUID
) {
    const OSPI_ADDR: u32 = 0x90000000;
    const SIZE_CODE: u32 = 27; // 256MiB region

    unsafe {
        // A. Allow unaligned accesses in the hardware (clear UNALIGN_TRP bit 3)
        let mut ccr = scb.ccr.read();
        ccr &= !(1 << 3);
        scb.ccr.write(ccr);

        // B. Ensure caches are ENABLED.
        // The Cortex-M7 cache controller automatically aligns and burst-fetches OSPI data,
        // completely preventing "unaligned access" UsageFaults.
        scb.enable_icache();
        scb.enable_dcache(cpuid);

        // C. Clean and Invalidate caches globally to ensure no stale data from bootloading
        scb.clean_invalidate_dcache(cpuid);
        scb.invalidate_icache();

        cortex_m::asm::dsb();
        cortex_m::asm::isb();

        // D. Configure the MPU Region
        mpu.rnr.write(0); // Region 0
        mpu.rbar.write(OSPI_ADDR);

        // TEX=000, C=1, B=1 translates to Normal Memory, Cacheable, Write-Back.
        // Normal memory is REQUIRED to prevent UsageFaults on unaligned accesses.
        let rasr = (0 << 28) |       // XN = 0 (Executable)
                   (0b011 << 24) |   // AP = 011 (Full Access)
                   (0b000 << 19) |   // TEX = 000
                   (0 << 18) |       // S = 0 (Non-shareable)
                   (1 << 17) |       // C = 1 (Cacheable)
                   (1 << 16) |       // B = 1 (Bufferable)
                   (SIZE_CODE << 1) |
                   1;                // ENABLE = 1

        mpu.rasr.write(rasr);

        // E. Enable the MPU (PRIVDEFENA = 1, ENABLE = 1)
        mpu.ctrl.modify(|r| r | 0x5);

        // F. Final sync to ensure MPU rules are active before we return
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
    }

    unsafe {
            // 1. Hardware-level Unaligned Access allowance
            let mut ccr = scb.ccr.read();
            ccr &= !(1 << 3); // Clear UNALIGN_TRP bit
            scb.ccr.write(ccr);

            // 2. Enable Caches to handle alignment translations automatically
            scb.enable_icache();
            scb.enable_dcache(cpuid);

            // 3. Global Cache Invalidation before memory mapping changes
            scb.clean_invalidate_dcache(cpuid);
            scb.invalidate_icache();
            cortex_m::asm::dsb();
            cortex_m::asm::isb();

            // --- GENERIC REGION HELPER ---
            // size_code calculation: Size in bytes = 2^(size_code + 1)
            let configure_region = |rnr: u32, rbar: u32, size_code: u32, xn: u32, ap: u32, tex: u32, c: u32, b: u32| {
                mpu.rnr.write(rnr);
                mpu.rbar.write(rbar);
                let rasr = (xn << 28)
                    | (ap << 24)
                    | (tex << 19)
                    | (0 << 18)  // S = 0 (Non-shareable)
                    | (c << 17)
                    | (b << 16)
                    | (size_code << 1)
                    | 1;         // ENABLE = 1
                mpu.rasr.write(rasr);
            };

            // --- REGION 0: OSPI External Flash ---
            // Base: 0x90000000, Size: 256MB (Code 27)
            // Normal Memory, Cacheable Write-Back (TEX=0, C=1, B=1), Executable (XN=0)
            configure_region(0, 0x90000000, 27, 0, 0b011, 0, 1, 1);

            // --- REGION 1: AXI SRAM ---
            // Base: 0x24000000, Size: 512KB (Code 18)
            // Normal Memory, Cacheable Write-Back (TEX=0, C=1, B=1), Executable (XN=0)
            // XN=0 allows running flash routines from RAM.
            configure_region(1, 0x24000000, 18, 0, 0b011, 0, 1, 1);

            // --- REGION 2: DTCM RAM ---
            // Base: 0x20000000, Size: 128KB (Code 16)
            // Normal Memory, Non-Cacheable (TEX=1, C=0, B=0), Executable (XN=0)
            // Note: DTCM bypasses the L1 cache in hardware anyway, but formally
            // defining it as Normal memory (TEX=1/C=0/B=0) ensures unaligned access is permitted.
            configure_region(2, 0x20000000, 16, 0, 0b011, 1, 0, 0);

            // Enable MPU with default background region active (bit 2 = PRIVDEFENA)
            mpu.ctrl.modify(|r| r | 0x5);

            // Final synchronization
            cortex_m::asm::dsb();
            cortex_m::asm::isb();
        }
}

/// Checks if a memory address is executable based on current MPU configuration.
pub fn is_executable(address: u32, mpu: &MPU) -> bool {
    // Iterate from 15 down to 0 (highest priority takes precedence)
    for i in (0..16).rev() {
        unsafe {
            mpu.rnr.write(i as u32);

            let rbar = mpu.rbar.read();
            let rasr = mpu.rasr.read();

            // Check if region is enabled (ENA bit)
            if (rasr & 0x1) != 0 {
                let base = rbar & 0xFFFFFFE0;
                let size_bits = (rasr >> 1) & 0x1F;
                let size = 1 << (size_bits + 1);
                let limit = base + size;

                if address >= base && address < limit {
                    // Check the XN (Execute Never) bit (bit 28 of RASR)
                    let xn = (rasr >> 28) & 0x1;
                    return xn == 0;
                }
            }
        }
    }
    false
}
