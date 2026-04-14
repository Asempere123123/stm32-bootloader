use core::u32;

use embassy_stm32::{
    bind_interrupts,
    can::{self, Can, Frame, Id, StandardId},
    flash::{self, Flash},
    peripherals,
};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    zerocopy_channel::{Channel, Sender},
};
use embassy_time::{Duration, WithTimeout};
use static_cell::StaticCell;

// Has to be templated
const BOARD_ID: u64 = {{ board-hash }};
const CAN_BITRATE: u32 = {{ can-baudrate }};
const BOOTLOADER_SIZE: usize = {{ flash-size }} * 1024;

const CAN_BOOTLOADER_TIMEOUT: Duration = Duration::from_millis(500);

static FLASH_CHANNEL_BUFFER: StaticCell<[FlashSectorToWrite; 2]> = StaticCell::new();
static FLASH_CHANNEL: StaticCell<Channel<'static, NoopRawMutex, FlashSectorToWrite>> =
    StaticCell::new();

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

bind_interrupts!(
    struct IrqsFlash {
        FLASH => flash::InterruptHandler;
    }
);

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

pub async fn can_flashing(
    peri: embassy_stm32::Peripherals,
    spawner: &mut embassy_executor::Spawner,
) {
    #[cfg(feature = "defmt")]
    defmt::info!("ENTERING CAN FLASHING");

    // Init peripherals
    // Has to be templated
    let mut can1 = can::Can::new(peri.{{ can }}, peri.{{ can-rx }}, peri.{{ can-tx }}, IrqsCan1);

    // Has to be templated
    #[cfg(feature = "can2")]
    let mut can2 = can::Can::new(peri.{{ can2 }}, peri.{{ can2-rx }}, peri.{{ can2-tx }}, IrqsCan2);

    #[cfg(not(feature = "can2"))]
    can1.modify_filters()
        .enable_bank(0, can::Fifo::Fifo0, can::filter::Mask32::accept_all());
    #[cfg(feature = "can2")]
    can1.modify_filters().slave_filters().enable_bank(
        14,
        can::Fifo::Fifo1,
        can::filter::Mask32::accept_all(),
    );

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
    let mut flash = Flash::new(peri.FLASH, IrqsFlash);
    if flash
        .erase(
            BOOTLOADER_SIZE as u32,
            BOOTLOADER_SIZE as u32 + flash_info.app_len,
        )
        .await
        .is_err()
    {
        panic!();
    }

    // Start flashing
    let (sender, mut receiver) = FLASH_CHANNEL
        .init(Channel::new(FLASH_CHANNEL_BUFFER.init([
            FlashSectorToWrite::empty(),
            FlashSectorToWrite::empty(),
        ])))
        .split();

    #[cfg(feature = "defmt")]
    defmt::info!("Can Flashing ready to recv");
    let Ok(can_task) = can_task(select_can!(can1, can2), sender, flash_info) else {
        panic!();
    };
    spawner.spawn(can_task);

    loop {
        let received_sector = receiver.receive().await;
        if received_sector.offset == u32::MAX {
            break;
        }

        if flash
            .write(
                BOOTLOADER_SIZE as u32 + received_sector.offset,
                &received_sector.data,
            )
            .await
            .is_err()
        {
            panic!()
        };

        receiver.receive_done();
    }

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

#[embassy_executor::task]
async fn can_task(
    mut can: Can<'static>,
    mut sender: Sender<'static, NoopRawMutex, FlashSectorToWrite>,
    _info: BeginFlashInfoMessage,
) {
    let mut current_offset = 0;
    'app: loop {
        let sector = sender.send().await;
        sector.offset = current_offset;
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
            let writen_sector =
                &mut sector.data[(flash_data.index as usize)..(flash_data.index as usize + 7)];
            writen_sector.copy_from_slice(&flash_data.data);

            if recv_message_count >= FLASH_SECTOR_WRITE_SIZE / 7 {
                break 'frame;
            }
        }

        can.write(&AckMessage.to_frame()).await;
        sender.send_done();
        current_offset += FLASH_SECTOR_WRITE_SIZE as u32;
    }

    // Notify done
    sender.send().await.offset = u32::MAX;
    sender.send_done();
    can.write(&AckMessage.to_frame()).await;
}

async fn wait_can_begin_flashing_message(can: &mut Can<'static>) {
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

async fn wait_can_flashing_info_message(can: &mut Can<'static>) -> BeginFlashInfoMessage {
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
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(0) });

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
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(1) });

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
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(2) });

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
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(3) });

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
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(4) });

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
    const MESSAGE_ID: Id = Id::Standard(unsafe { StandardId::new_unchecked(5) });

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
