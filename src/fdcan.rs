use embassy_stm32::{
    bind_interrupts,
    can::{
        self, Can, CanConfigurator, Frame, OperatingMode,
        filter::{StandardFilter, StandardFilterSlot},
    },
    peripherals,
};
use embassy_time::{Duration, WithTimeout};
use embedded_can::{Id, StandardId};

// Has to be templated
const BOARD_ID: u64 = {{ board-hash }};
const CAN_BITRATE: u32 = {{ can-baudrate }};
#[allow(unused)]
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

    // Check for flashing request
    if wait_can_begin_flashing_message(&mut can)
        .with_timeout(CAN_BOOTLOADER_TIMEOUT)
        .await
        .is_err()
    {
        #[cfg(feature = "defmt")]
        defmt::info!("Can Flashing timed out");
        return;
    }

    // Ack that we entered bootloader mode
    can.write(&AckMessage.to_frame()).await;

    // Receive flash info and ack
    let flash_info = wait_can_flashing_info_message(&mut can).await;
    can.write(&AckMessage.to_frame()).await;

    // Erase and Ack
    #[cfg(not(feature = "external-flash"))]
    {
        let mut flash = embassy_stm32::flash::Flash::new_blocking(peri.FLASH.reborrow());
        if flash
            .blocking_erase(
                BOOTLOADER_SIZE as u32,
                (BOOTLOADER_SIZE as u32 + flash_info.app_len).div_ceil(BOOTLOADER_SIZE as u32)
                    * (BOOTLOADER_SIZE as u32),
            )
            .is_err()
        {
            #[cfg(feature = "defmt")]
            defmt::info!("Erasing flash failed, falling back to erasing the complete flash");
            flash
                .blocking_erase(
                    BOOTLOADER_SIZE as u32,
                    embassy_stm32::flash::FLASH_SIZE as u32,
                )
                .unwrap();
        }

        // Start flashing
        #[cfg(feature = "defmt")]
        defmt::info!("Can Flashing ready to recv");
        flash_app(can, flash, flash_info).await;

        #[cfg(feature = "defmt")]
        defmt::info!("FINISHED CAN FLASHING");
    }
    #[cfg(feature = "external-flash")]
    {
        let Ok(mut flash) = crate::create_external_flash!(peri) else {
            panic!();
        };
        let flash: &mut dyn crate::external_flash::ExternalFlash = &mut flash;

        let mut curr_erased_addr = 0;
        while curr_erased_addr < flash_info.app_len {
            flash.erase(curr_erased_addr);
            curr_erased_addr += flash.erase_size() as u32;
        }

        // Start flashing
        #[cfg(feature = "defmt")]
        defmt::info!("Can Flashing ready to recv");
        flash_app(can, flash, flash_info).await;

        #[cfg(feature = "defmt")]
        defmt::info!("FINISHED CAN FLASHING");
    }
}

const MAX_FLASH_SECTOR_WRITE_SIZE: usize = 32 * 7;

struct FlashSectorToWrite {
    offset: u32,
    data: [u8; MAX_FLASH_SECTOR_WRITE_SIZE],
}

impl FlashSectorToWrite {
    pub fn empty() -> Self {
        Self {
            offset: u32::MAX,
            data: [0; MAX_FLASH_SECTOR_WRITE_SIZE],
        }
    }
}

#[cfg(not(feature = "external-flash"))]
async fn flash_app(
    mut can: Can<'_>,
    mut flash: embassy_stm32::flash::Flash<'_, embassy_stm32::flash::Blocking>,
    info: BeginFlashInfoMessage,
) {
    let sector_size_bytes = info.sector_size as usize * 7;
    let mut current_offset = 0;
    'app: loop {
        let mut received_sector = FlashSectorToWrite::empty();
        received_sector.offset = current_offset;
        can.write(&AckMessage.to_frame()).await;

        // It is guranteed that each one is received only once, thus counting and matching is enough
        let mut recv_message_count = 0;
        'frame: loop {
            let Ok(can_frame) = can.read().await else {
                continue 'frame;
            };

            let Some(flash_data) = FlashDataMessage::try_from_frame(&can_frame.frame) else {
                if RevertSectorMessage::try_from_frame(&can_frame.frame).is_some() {
                    continue 'app;
                }

                if FlashFinishMessage::try_from_frame(&can_frame.frame).is_some() {
                    break 'app;
                }
                continue 'frame;
            };

            recv_message_count += 1;
            let writen_sector = &mut received_sector.data
                [(flash_data.index as usize)..(flash_data.index as usize + 7)];
            writen_sector.copy_from_slice(&flash_data.data);

            if recv_message_count >= info.sector_size as usize {
                break 'frame;
            }
        }

        can.write(&AckMessage.to_frame()).await;
        flash
            .blocking_write(
                BOOTLOADER_SIZE as u32 + received_sector.offset,
                &received_sector.data[0..sector_size_bytes],
            )
            .unwrap();
        current_offset += sector_size_bytes as u32;
    }

    // Notify done
    can.write(&AckMessage.to_frame()).await;
}

#[cfg(feature = "external-flash")]
async fn flash_app(
    mut can: Can<'_>,
    flash: &mut dyn crate::external_flash::ExternalFlash,
    info: BeginFlashInfoMessage,
) {
    let sector_size_bytes = info.sector_size as usize * 7;
    let mut current_offset = 0;
    'app: loop {
        let mut received_sector = FlashSectorToWrite::empty();
        received_sector.offset = current_offset;
        can.write(&AckMessage.to_frame()).await;

        // It is guranteed that each one is received only once, thus counting and matching is enough
        let mut recv_message_count = 0;
        'frame: loop {
            let Ok(can_frame) = can.read().await else {
                continue 'frame;
            };

            let Some(flash_data) = FlashDataMessage::try_from_frame(&can_frame.frame) else {
                if RevertSectorMessage::try_from_frame(&can_frame.frame).is_some() {
                    continue 'app;
                }

                if FlashFinishMessage::try_from_frame(&can_frame.frame).is_some() {
                    break 'app;
                }
                continue 'frame;
            };

            recv_message_count += 1;
            let writen_sector = &mut received_sector.data
                [(flash_data.index as usize)..(flash_data.index as usize + 7)];
            writen_sector.copy_from_slice(&flash_data.data);

            if recv_message_count >= info.sector_size as usize {
                break 'frame;
            }
        }

        can.write(&AckMessage.to_frame()).await;
        flash.write(
            received_sector.offset,
            &received_sector.data[0..sector_size_bytes],
        );
        current_offset += sector_size_bytes as u32;
    }

    // Notify done
    can.write(&AckMessage.to_frame()).await;
}

async fn wait_can_begin_flashing_message(can: &mut Can<'_>) {
    loop {
        let Ok(message) = can.read().await else {
            continue;
        };

        let Some(message) = BeginCanFlashingMessage::try_from_frame(&message.frame) else {
            continue;
        };

        if message.board_id == BOARD_ID {
            break;
        }
    }
}

async fn wait_can_flashing_info_message(can: &mut Can<'_>) -> BeginFlashInfoMessage {
    loop {
        let Ok(message) = can.read().await else {
            continue;
        };

        let Some(message) = BeginFlashInfoMessage::try_from_frame(&message.frame) else {
            continue;
        };

        return message;
    }
}

//// Frame kind types

pub struct BeginCanFlashingMessage {
    board_id: u64,
}

impl BeginCanFlashingMessage {
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(0x303) });

    #[allow(unused)]
    pub fn try_from_frame(frame: &Frame) -> Option<Self> {
        if *frame.id() == Self::MESSAGE_ID {
            Some(Self {
                board_id: bytemuck::pod_read_unaligned(frame.data()),
            })
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn to_frame(self) -> Frame {
        let Ok(frame) = Frame::new_data(Self::MESSAGE_ID, bytemuck::bytes_of(&self.board_id))
        else {
            panic!()
        };

        frame
    }
}

pub struct AckMessage;

impl AckMessage {
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(0x304) });

    #[allow(unused)]
    pub fn try_from_frame(frame: &Frame) -> Option<Self> {
        if *frame.id() == Self::MESSAGE_ID {
            Some(Self)
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn to_frame(self) -> Frame {
        let Ok(frame) = Frame::new_data(Self::MESSAGE_ID, &[]) else {
            panic!()
        };

        frame
    }
}

#[repr(C)]
#[derive(bytemuck::AnyBitPattern, bytemuck::NoUninit, Clone, Copy)]
pub struct FlashDataMessage {
    index: u8,
    data: [u8; 7],
}

impl FlashDataMessage {
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(0x305) });

    #[allow(unused)]
    pub fn try_from_frame(frame: &Frame) -> Option<Self> {
        if *frame.id() == Self::MESSAGE_ID {
            Some(bytemuck::pod_read_unaligned(frame.data()))
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn to_frame(self) -> Frame {
        let Ok(frame) = Frame::new_data(Self::MESSAGE_ID, bytemuck::bytes_of(&self)) else {
            panic!()
        };

        frame
    }
}

#[repr(C, packed)]
#[derive(bytemuck::AnyBitPattern, bytemuck::NoUninit, Clone, Copy)]
pub struct BeginFlashInfoMessage {
    app_len: u32,
    sector_size: u8,
}

impl BeginFlashInfoMessage {
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(0x306) });

    #[allow(unused)]
    pub fn try_from_frame(frame: &Frame) -> Option<Self> {
        if *frame.id() == Self::MESSAGE_ID {
            Some(bytemuck::pod_read_unaligned(frame.data()))
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn to_frame(self) -> Frame {
        let Ok(frame) = Frame::new_data(Self::MESSAGE_ID, bytemuck::bytes_of(&self)) else {
            panic!()
        };

        frame
    }
}

pub struct FlashFinishMessage;

impl FlashFinishMessage {
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(0x307) });

    #[allow(unused)]
    pub fn try_from_frame(frame: &Frame) -> Option<Self> {
        if *frame.id() == Self::MESSAGE_ID {
            Some(Self)
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn to_frame(self) -> Frame {
        let Ok(frame) = Frame::new_data(Self::MESSAGE_ID, &[]) else {
            panic!()
        };

        frame
    }
}

pub struct RevertSectorMessage;

impl RevertSectorMessage {
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(0x308) });

    #[allow(unused)]
    pub fn try_from_frame(frame: &Frame) -> Option<Self> {
        if *frame.id() == Self::MESSAGE_ID {
            Some(Self)
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn to_frame(self) -> Frame {
        let Ok(frame) = Frame::new_data(Self::MESSAGE_ID, &[]) else {
            panic!()
        };

        frame
    }
}
