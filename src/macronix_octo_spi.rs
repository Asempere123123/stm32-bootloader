/// BASED ON: https://github.com/STMicroelectronics/STM32CubeH7/blob/master/Projects/STM32H7B3I-DK/Examples/OSPI/OSPI_NOR_MemoryMapped_DTR/Src/main.c
use embassy_stm32::{
    mode::Blocking,
    ospi::{Instance, Ospi, *},
};
use embassy_time::Duration;

use crate::external_flash::ExternalFlash;

const READ_STATUS_REG_CMD: u32 = 0x05;
const WRITE_ENABLE_CMD: u32 = 0x06;
const WRITE_CFG_REG_2_CMD: u32 = 0x72;

const OCTAL_READ_STATUS_REG_CMD: u32 = 0x05FA;
const OCTAL_WRITE_ENABLE_CMD: u32 = 0x06F9;
const OCTAL_IO_DTR_READ_CMD: u32 = 0xEE11;
const OCTAL_32KB_BLOCK_ERASE_CMD: u32 = 0x52AD;
const OCTAL_PAGE_PROG_CMD: u32 = 0x12ED;

// TO BE TEMPLATED
const CHIP_ERASE_SIZE: usize = {{ external-flash-erase-size }};
const CHIP_WRITE_SIZE: usize = {{ external-flash-write-size }};
const DUMMY_CYCLES: DummyCycles = DummyCycles::_{{ octo-spi-dummy-cycles }};

#[macro_export]
macro_rules! create_external_flash {
    ($peri:expr) => {% raw %}{{{% endraw %}
        use embassy_stm32::ospi::{Ospi, *};

        let config = Config {
            fifo_threshold: FIFOThresholdLevel::_4Bytes,
            memory_type: MemoryType::Macronix,
            device_size: MemorySize::Other({{ octo-spi-device-size }}), // To template
            chip_select_high_time: ChipSelectHighTime::_2Cycle,
            free_running_clock: false,
            clock_mode: false,
            wrap_size: WrapSize::None,
            clock_prescaler: 0, // TEMPLATE ASWELL maybe not?
            sample_shifting: false,
            delay_hold_quarter_cycle: true,
            chip_select_boundary: 0,
            delay_block_bypass: false,
            max_transfer: 0,
            refresh: 0,
        };

        let ospi = Ospi::new_blocking_octospi_with_dqs(
            $peri.{{ octo-spi-peri }}.reborrow(),
            $peri.{{ octo-spi-sck }}.reborrow(),
            $peri.{{ octo-spi-d0 }}.reborrow(),
            $peri.{{ octo-spi-d1 }}.reborrow(),
            $peri.{{ octo-spi-d2 }}.reborrow(),
            $peri.{{ octo-spi-d3 }}.reborrow(),
            $peri.{{ octo-spi-d4 }}.reborrow(),
            $peri.{{ octo-spi-d5 }}.reborrow(),
            $peri.{{ octo-spi-d6 }}.reborrow(),
            $peri.{{ octo-spi-d7 }}.reborrow(),
            $peri.{{ octo-spi-cs }}.reborrow(), // CHIP SELECT
            $peri.{{ octo-spi-dqs }}.reborrow(),
            config,
        );

        $crate::macronix_octo_spi::OctoSpiFlash::_new(ospi)
    {% raw %}}}{% endraw %};
}

pub struct OctoSpiFlash<'d, T: Instance> {
    ospi: Ospi<'d, T, Blocking>,
}

impl<'d, T: Instance> OctoSpiFlash<'d, T> {
    pub fn _new(ospi: Ospi<'d, T, Blocking>) -> Result<Self, OspiError> {
        let mut flash = Self { ospi };
        flash.enable_octal_dtr_mode()?;

        Ok(flash)
    }

    fn enable_octal_dtr_mode(&mut self) -> Result<(), OspiError> {
        self.enable_write_single_mode()?;

        let write_reg_cmd = TransferConfig {
            iwidth: OspiWidth::SING,
            instruction: Some(WRITE_CFG_REG_2_CMD),
            isize: AddressSize::_8Bit,
            idtr: false,
            adwidth: OspiWidth::SING,
            address: Some(0),
            adsize: AddressSize::_32bit,
            addtr: false,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::SING,
            ddtr: false,
            dummy: DummyCycles::_0,
            dqse: false,
            sioo: false,
        };

        self.ospi.blocking_write(&[0x2u8], write_reg_cmd)?;
        embassy_time::block_for(Duration::from_millis(40));
        Ok(())
    }

    fn enable_write_single_mode(&mut self) -> Result<(), OspiError> {
        let write_enable_cmd = TransferConfig {
            iwidth: OspiWidth::SING,
            instruction: Some(WRITE_ENABLE_CMD),
            isize: AddressSize::_8Bit,
            idtr: false,
            adwidth: OspiWidth::NONE,
            address: None,
            adsize: AddressSize::_8Bit,
            addtr: false,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::NONE,
            ddtr: false,
            dummy: DummyCycles::_0,
            dqse: false,
            sioo: false,
        };

        self.ospi.blocking_command(&write_enable_cmd)?;
        self.poll_for_write_enable_single_mode()
    }

    fn poll_for_write_enable_single_mode(&mut self) -> Result<(), OspiError> {
        let mut status = [0u8; 1];
        loop {
            let poll_cmd = TransferConfig {
                iwidth: OspiWidth::SING,
                instruction: Some(READ_STATUS_REG_CMD),
                isize: AddressSize::_8Bit,
                idtr: false,
                adwidth: OspiWidth::NONE,
                address: None,
                adsize: AddressSize::_8Bit,
                addtr: false,
                abwidth: OspiWidth::NONE,
                alternate_bytes: None,
                absize: AddressSize::_8Bit,
                abdtr: false,
                dwidth: OspiWidth::SING,
                ddtr: false,
                dummy: DummyCycles::_0,
                dqse: false,
                sioo: false,
            };

            self.ospi.blocking_read(&mut status, poll_cmd)?;

            // Write enable requires that bit to be set
            if (status[0] & 0x02) == 0x02 {
                break;
            }

            embassy_time::block_for(Duration::from_millis(1));
        }

        Ok(())
    }

    fn enable_write(&mut self) -> Result<(), OspiError> {
        let write_enable_cmd = TransferConfig {
            iwidth: OspiWidth::OCTO,
            instruction: Some(OCTAL_WRITE_ENABLE_CMD),
            isize: AddressSize::_16Bit,
            idtr: true,
            adwidth: OspiWidth::NONE,
            address: None,
            adsize: AddressSize::_8Bit,
            addtr: false,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::NONE,
            ddtr: false,
            dummy: DummyCycles::_0,
            dqse: false,
            sioo: false,
        };

        self.ospi.blocking_command(&write_enable_cmd)?;
        self.poll_for_write_enable()
    }

    fn poll_for_write_enable(&mut self) -> Result<(), OspiError> {
        let we_poll_cmd = TransferConfig {
            iwidth: OspiWidth::OCTO,
            instruction: Some(OCTAL_READ_STATUS_REG_CMD),
            isize: AddressSize::_16Bit,
            idtr: true,
            adwidth: OspiWidth::OCTO,
            address: Some(0x0),
            adsize: AddressSize::_32bit,
            addtr: true,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::OCTO,
            ddtr: true,
            dummy: DUMMY_CYCLES,
            dqse: true,
            sioo: false,
        };

        let mut status = [0u8; 2];
        loop {
            self.ospi.blocking_read(&mut status, we_poll_cmd)?;

            // Write enable requires that bit to be set
            if (status[0] & 0x02) == 0x02 {
                break;
            }

            embassy_time::block_for(Duration::from_millis(1));
        }

        Ok(())
    }

    fn poll_for_mem_ready(&mut self) -> Result<(), OspiError> {
        let mem_ready_poll_cmd = TransferConfig {
            iwidth: OspiWidth::OCTO,
            instruction: Some(OCTAL_READ_STATUS_REG_CMD),
            isize: AddressSize::_16Bit,
            idtr: true,
            adwidth: OspiWidth::OCTO,
            address: Some(0x0),
            adsize: AddressSize::_32bit,
            addtr: true,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::OCTO,
            ddtr: true,
            dummy: DUMMY_CYCLES,
            dqse: true,
            sioo: false,
        };

        let mut status = [0u8; 2];
        loop {
            self.ospi.blocking_read(&mut status, mem_ready_poll_cmd)?;
            // Bit has to be unset for memory to be ready
            if (status[0] & 0x01) == 0x00 {
                break;
            }

            embassy_time::block_for(Duration::from_millis(1));
        }

        Ok(())
    }

    pub fn erase(&mut self, addr: u32) -> Result<(), OspiError> {
        self.enable_write()?;

        let erase_cmd = TransferConfig {
            iwidth: OspiWidth::OCTO,
            instruction: Some(OCTAL_32KB_BLOCK_ERASE_CMD),
            isize: AddressSize::_16Bit,
            idtr: true,
            adwidth: OspiWidth::OCTO,
            address: Some(addr),
            adsize: AddressSize::_32bit,
            addtr: true,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::NONE,
            ddtr: false,
            dummy: DummyCycles::_0,
            dqse: false,
            sioo: false,
        };

        self.ospi.blocking_command(&erase_cmd)?;
        self.poll_for_mem_ready()
    }

    pub fn write(&mut self, addr: u32, buf: &[u8]) -> Result<(), OspiError> {
        self.enable_write()?;

        let write_cmd = TransferConfig {
            iwidth: OspiWidth::OCTO,
            instruction: Some(OCTAL_PAGE_PROG_CMD),
            isize: AddressSize::_16Bit,
            idtr: true,
            adwidth: OspiWidth::OCTO,
            address: Some(addr),
            adsize: AddressSize::_32bit,
            addtr: true,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::OCTO,
            ddtr: true,
            dummy: DummyCycles::_0,
            dqse: true,
            sioo: false,
        };
        self.ospi.blocking_write(buf, write_cmd)?;
        self.poll_for_mem_ready()
    }

    pub fn enabled_memory_mapped_mode(&mut self) -> Result<(), OspiError> {
        let write_cfg = TransferConfig {
            iwidth: OspiWidth::OCTO,
            instruction: None,
            isize: AddressSize::_16Bit,
            idtr: true,
            adwidth: OspiWidth::NONE,
            address: None,
            adsize: AddressSize::_32bit,
            addtr: true,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::OCTO,
            ddtr: true,
            dummy: DummyCycles::_0,
            dqse: true,
            sioo: false,
        };

        let read_cfg = TransferConfig {
            iwidth: OspiWidth::OCTO,
            instruction: Some(OCTAL_IO_DTR_READ_CMD),
            isize: AddressSize::_16Bit,
            idtr: true,
            adwidth: OspiWidth::OCTO,
            address: None,
            adsize: AddressSize::_32bit,
            addtr: true,
            abwidth: OspiWidth::NONE,
            alternate_bytes: None,
            absize: AddressSize::_8Bit,
            abdtr: false,
            dwidth: OspiWidth::OCTO,
            ddtr: true,
            dummy: DUMMY_CYCLES,
            dqse: true,
            sioo: false,
        };

        self.ospi.enable_memory_mapped_mode(read_cfg, write_cfg)?;
        embassy_time::block_for(embassy_time::Duration::from_millis(1));

        Ok(())
    }
}

impl<'d, T: Instance> ExternalFlash for OctoSpiFlash<'d, T> {
    fn erase_size(&self) -> usize {
        CHIP_ERASE_SIZE
    }
    fn write_size(&self) -> usize {
        CHIP_WRITE_SIZE
    }

    fn erase(&mut self, addr: u32) {
        let _ = self.erase(addr);
    }
    fn write(&mut self, addr: u32, buf: &[u8]) {
        let _ = self.write(addr, buf);
    }
    fn enabled_memory_mapped_mode(&mut self) {
        let _ = self.enabled_memory_mapped_mode();
    }
}
