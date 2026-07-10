use embassy_time::{Duration, WithTimeout};

use crate::host_to_chip::HostToChip;

pub trait ExternalFlash {
    fn erase_size(&self) -> usize;
    fn write_size(&self) -> usize;

    fn erase(&mut self, addr: u32);
    fn write(&mut self, addr: u32, buf: &[u8]);
    fn enabled_memory_mapped_mode(&mut self);
}

const INIT_FLASH_CMD: u8 = 0xD;
const ACCEPT_FLASH_CMD: u8 = 0xDD;
const ERASE_FINISHED_CMD: u8 = 0xA;
const INIT_FLASH_TIMEOUT: Duration = Duration::from_millis(50);
const FINISH_FLASH_TIMEOUT: Duration = Duration::from_millis(10);

pub async fn flash_from_debugger(
    peripherals: &mut embassy_stm32::Peripherals,
    host_to_chip: &mut HostToChip,
) {
    #[cfg(feature = "defmt")]
    defmt::info!("Starting flashing external flash");

    let Ok(mut flash) = crate::create_external_flash!(peripherals) else {
        panic!();
    };
    let flash: &mut dyn ExternalFlash = &mut flash;

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

    host_to_chip.write(&[ACCEPT_FLASH_CMD]);
    host_to_chip.write(&(flash.erase_size() as u32).to_le_bytes());
    host_to_chip.write(&(flash.write_size() as u32).to_le_bytes());

    let mut app_len = [0; 4];
    receive_buf(host_to_chip, &mut app_len).await;
    let app_len = u32::from_le_bytes(app_len);

    let mut curr_erased_addr = 0;
    while curr_erased_addr < app_len {
        flash.erase(curr_erased_addr);
        curr_erased_addr += flash.erase_size() as u32;
    }

    host_to_chip.write(&[ERASE_FINISHED_CMD]);

    let mut data_buf = [0; 512];
    let mut curr_addr = 0;
    loop {
        if receive_buf(host_to_chip, &mut data_buf[0..(flash.write_size())])
            .with_timeout(FINISH_FLASH_TIMEOUT)
            .await
            .is_err()
        {
            break;
        }

        flash.write(curr_addr, &data_buf[0..(flash.write_size())]);
        curr_addr += flash.write_size() as u32;
        host_to_chip.write(&[0xB]);
    }

    host_to_chip.write(&[0xC]);
    #[cfg(feature = "defmt")]
    defmt::info!("Finished flashing external flash");
}

async fn receive_byte(host_to_chip: &mut HostToChip) -> u8 {
    let mut buf = [0u8];
    receive_buf(host_to_chip, &mut buf).await;
    buf[0]
}

async fn receive_buf(host_to_chip: &mut HostToChip, buf: &mut [u8]) {
    let mut amount_read = 0;
    while amount_read != buf.len() {
        amount_read += host_to_chip.read(&mut buf[amount_read..]);
        embassy_futures::yield_now().await;
    }
}
