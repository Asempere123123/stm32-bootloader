#[cfg(all(feature = "rtt-host-to-chip", not(feature = "defmt")))]
#[macro_export]
macro_rules! rtt_init {
    () => {
        ::rtt_target::rtt_init! {
            down: {
                0: { size: 128, name: "commands" }
            }
        }
    };
}

#[cfg(all(feature = "rtt-host-to-chip", feature = "defmt"))]
#[macro_export]
macro_rules! rtt_init {
    () => {
        ::rtt_target::rtt_init! {
            up: {
                0: { size: 1024, name: "defmt" }
            }
            down: {
                0: { size: 128, name: "commands" }
            }
        }
    };
}

#[cfg(all(not(feature = "rtt-host-to-chip"), feature = "defmt"))]
#[macro_export]
macro_rules! rtt_init {
    () => {
        ::rtt_target::rtt_init! {
            up: {
                0: { size: 1024, name: "defmt" }
            }
        }
    };
}
