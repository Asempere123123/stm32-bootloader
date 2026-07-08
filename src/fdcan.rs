pub async fn fdcan_flashing(_peri: &mut embassy_stm32::Peripherals) {
    #[cfg(feature = "defmt")]
    defmt::info!("Entering fdcan flashing");
}
