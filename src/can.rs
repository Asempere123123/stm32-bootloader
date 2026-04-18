use embassy_stm32::{
    bind_interrupts,
    can::{self, Can, Frame, Id, StandardId},
    flash::{Blocking, FLASH_SIZE, Flash},
    peripherals,
};
use embassy_time::{Duration, WithTimeout};

// Has to be templated
const BOARD_ID: u64 = {{ board-hash }};
const CAN_BITRATE: u32 = {{ can-baudrate }};
const BOOTLOADER_SIZE: usize = {{ flash-size }} * 1024;

const CAN_BOOTLOADER_TIMEOUT: Duration = Duration::from_millis(500);

// Has to be templated
bind_interrupts!(
    struct IrqsCan1 {
        {{ can }}_TX => can::TxInterruptHandler<peripherals::{{ can }}>;
        {{ can }}_RX0 => can::Rx0InterruptHandler<peripherals::{{ can }}>;
        {{ can }}_RX1 => can::Rx1InterruptHandler<peripherals::{{ can }}>;
        {{ can }}_SCE => can::SceInterruptHandler<peripherals::{{ can }}>;
    }
);

// Has to be templated
#[cfg(feature = "can2")]
bind_interrupts!(
    struct IrqsCan2 {
        {{ can2 }}_TX => can::TxInterruptHandler<peripherals::{{ can2 }}>;
        {{ can2 }}_RX0 => can::Rx0InterruptHandler<peripherals::{{ can2 }}>;
        {{ can2 }}_RX1 => can::Rx1InterruptHandler<peripherals::{{ can2 }}>;
        {{ can2 }}_SCE => can::SceInterruptHandler<peripherals::{{ can2 }}>;
    }
);

{% raw %}
macro_rules! select_can {
    ($can1:expr, $can2:expr) => {{
        #[cfg(not(feature = "can2"))]
        {
            $can1
        }

        #[cfg(feature = "can2")]
        {
            $can2
        }
    }};
}

macro_rules! select_can_ref_mut {
    ($can1:expr, $can2:expr) => {{
        #[cfg(not(feature = "can2"))]
        {
            &mut $can1
        }

        #[cfg(feature = "can2")]
        {
            &mut $can2
        }
    }};
}
{% endraw %}

pub async fn can_flashing(peri: &mut embassy_stm32::Peripherals) {
    #[cfg(feature = "defmt")]
    defmt::info!("ENTERING CAN FLASHING");

    // Init peripherals
    // Has to be templated
    let mut can1 = can::Can::new(
        peri.{{ can }}.reborrow(),
        peri.{{ can-rx }}.reborrow(),
        peri.{{ can-tx }}.reborrow(),
        IrqsCan1,
    );

    // Has to be templated
    #[cfg(feature = "can2")]
    let mut can2 = can::Can::new(
        peri.{{ can2 }}.reborrow(),
        peri.{{ can2-rx }}.reborrow(),
        peri.{{ can2-tx }}.reborrow(),
        IrqsCan2,
    );

    #[cfg(not(feature = "can2"))]
    {
        can1.modify_filters()
            .enable_bank(0, can::Fifo::Fifo0, can::filter::BankConfig::List32([
                can::filter::ListEntry32::data_frames_with_id(BeginCanFlashingMessage::MESSAGE_ID),
                can::filter::ListEntry32::data_frames_with_id(AckMessage::MESSAGE_ID)
            ]))
            .enable_bank(1, can::Fifo::Fifo0, can::filter::BankConfig::List32([
                can::filter::ListEntry32::data_frames_with_id(FlashDataMessage::MESSAGE_ID),
                can::filter::ListEntry32::data_frames_with_id(BeginFlashInfoMessage::MESSAGE_ID)
            ]))
            .enable_bank(2, can::Fifo::Fifo0, can::filter::BankConfig::List32([
                can::filter::ListEntry32::data_frames_with_id(FlashFinishMessage::MESSAGE_ID),
                can::filter::ListEntry32::data_frames_with_id(RevertSectorMessage::MESSAGE_ID)
            ]));
    }
    #[cfg(feature = "can2")]
    {
        can1.modify_filters()
            .slave_filters()
            .enable_bank(14, can::Fifo::Fifo1, can::filter::BankConfig::List32([
                can::filter::ListEntry32::data_frames_with_id(BeginCanFlashingMessage::MESSAGE_ID),
                can::filter::ListEntry32::data_frames_with_id(AckMessage::MESSAGE_ID)
            ]))
            .enable_bank(15, can::Fifo::Fifo1, can::filter::BankConfig::List32([
                can::filter::ListEntry32::data_frames_with_id(FlashDataMessage::MESSAGE_ID),
                can::filter::ListEntry32::data_frames_with_id(BeginFlashInfoMessage::MESSAGE_ID)
            ]))
            .enable_bank(16, can::Fifo::Fifo1, can::filter::BankConfig::List32([
                can::filter::ListEntry32::data_frames_with_id(FlashFinishMessage::MESSAGE_ID),
                can::filter::ListEntry32::data_frames_with_id(RevertSectorMessage::MESSAGE_ID)
            ]));
    }

    select_can_ref_mut!(can1, can2).set_bitrate(CAN_BITRATE);
    select_can_ref_mut!(can1, can2).enable().await;

    // Check for flashing request
    if wait_can_begin_flashing_message(select_can_ref_mut!(can1, can2))
        .with_timeout(CAN_BOOTLOADER_TIMEOUT)
        .await
        .is_err()
    {
        #[cfg(feature = "defmt")]
        defmt::info!("Can Flashing timed out");
        return;
    }

    // Ack that we entered bootloader mode
    select_can_ref_mut!(can1, can2)
        .write(&AckMessage.to_frame())
        .await;

    // Receive flash info and ack
    let flash_info = wait_can_flashing_info_message(select_can_ref_mut!(can1, can2)).await;
    select_can_ref_mut!(can1, can2)
        .write(&AckMessage.to_frame())
        .await;

    // Erase and Ack
    let mut flash = Flash::new_blocking(peri.FLASH.reborrow());
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
            .blocking_erase(BOOTLOADER_SIZE as u32, FLASH_SIZE as u32)
            .unwrap();
    }

    // Start flashing
    #[cfg(feature = "defmt")]
    defmt::info!("Can Flashing ready to recv");
    flash_app(select_can!(can1, can2), flash, flash_info).await;

    #[cfg(feature = "defmt")]
    defmt::info!("FINISHED CAN FLASHING");
}

const FLASH_SECTOR_WRITE_SIZE: usize = 32 * 7;

struct FlashSectorToWrite {
    offset: u32,
    data: [u8; FLASH_SECTOR_WRITE_SIZE],
}

impl FlashSectorToWrite {
    pub fn empty() -> Self {
        Self {
            offset: u32::MAX,
            data: [0; FLASH_SECTOR_WRITE_SIZE],
        }
    }
}

async fn flash_app(mut can: Can<'_>, mut flash: Flash<'_, Blocking>, _info: BeginFlashInfoMessage) {
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

            if recv_message_count >= FLASH_SECTOR_WRITE_SIZE / 7 {
                break 'frame;
            }
        }

        can.write(&AckMessage.to_frame()).await;
        flash
            .blocking_write(
                BOOTLOADER_SIZE as u32 + received_sector.offset,
                &received_sector.data,
            )
            .unwrap();
        current_offset += FLASH_SECTOR_WRITE_SIZE as u32;
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

#[repr(C)]
#[derive(bytemuck::AnyBitPattern, bytemuck::NoUninit, Clone, Copy)]
pub struct BeginFlashInfoMessage {
    app_len: u32,
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
