use embassy_time::{Duration, Timer, WithTimeout};

use crate::host_to_chip::HostToChip;

pub trait ExternalFlash {
    fn erase_size(&self) -> usize;
    fn write_size(&self) -> usize;

    fn erase(&mut self, addr: u32);
    fn write(&mut self, addr: u32, buf: &[u8]);
    fn enabled_memory_mapped_mode(&mut self);
}

const INIT_FLASH_CMD: u8 = 0xD;
const INIT_FLASH_TIMEOUT: Duration = Duration::from_millis(50);

pub async fn flash_from_debugger(
    peripherals: &mut embassy_stm32::Peripherals,
    host_to_chip: &mut HostToChip,
) {
    let Ok(mut flash) = crate::create_external_flash!(peripherals) else {
        panic!();
    };
    let mut flash: &mut dyn ExternalFlash = &mut flash;

    let Ok(byte) = receive_byte(host_to_chip)
        .with_timeout(INIT_FLASH_TIMEOUT)
        .await
    else {
        #[cfg(feature = "defmt")]
        defmt::info!("Timed out flashing from debugger");
        return;
    };

    if byte != INIT_FLASH_CMD {
        #[cfg(feature = "defmt")]
        defmt::info!("Debugger did not request to write to external flash");
        return;
    }

    host_to_chip.write(&[1]);
}

async fn receive_byte(host_to_chip: &mut HostToChip) -> u8 {
    let mut buf = [0u8];
    while host_to_chip.read(&mut buf) == 0 {
        Timer::after_millis(1).await;
    }
    buf[0]
}
