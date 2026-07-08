use embassy_stm32::{
    bind_interrupts,
    can::{
        self, CanConfigurator, OperatingMode,
        filter::{StandardFilter, StandardFilterSlot},
    },
    peripherals,
};
use embassy_time::Duration;

// Has to be templated
const BOARD_ID: u64 = {{ board-hash }};
const CAN_BITRATE: u32 = {{ can-baudrate }};
const BOOTLOADER_SIZE: usize = {{ flash-size }} * 1024;

const CAN_BOOTLOADER_TIMEOUT: Duration = Duration::from_millis(500);

// Has to be templated
bind_interrupts!(
    struct Irqs {
        {{ fdcan-it0-int-name }} => can::IT0InterruptHandler<peripherals::{{ fdcan }}>;
        {{ fdcan-it1-int-name }} => can::IT1InterruptHandler<peripherals::{{ fdcan }}>;
    }
);

pub async fn fdcan_flashing(peri: &mut embassy_stm32::Peripherals) {
    #[cfg(feature = "defmt")]
    defmt::info!("Entering fdcan flashing");

    // Has to be templated
    let mut can = CanConfigurator::new(
        peri.{{ fdcan }}.reborrow(),
        peri.{{ fdcan-rx }}.reborrow(),
        peri.{{ fdcan-tx }}.reborrow(),
        Irqs,
    );

    // Completar con los filtros de vd
    can.properties().set_standard_filter(
        StandardFilterSlot::_0,
        StandardFilter::accept_all_into_fifo0(),
    );
    can.set_bitrate(CAN_BITRATE);
    let mut can = can.start(OperatingMode::NormalOperationMode);

    // Way to scared to have anything functional here
    match can.read().await {
        Ok(msg) => {
            can.write(&msg.frame).await;
        }
        Err(e) => {
            #[cfg(feature = "defmt")]
            defmt::warn!("Error reading can: {}", e);
        }
    }
}
