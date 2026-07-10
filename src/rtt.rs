#[cfg(all(feature = "rtt-host-to-chip", not(feature = "defmt")))]
#[macro_export]
macro_rules! rtt_init {
    () => {% raw %}{{{% endraw %}
        let channels = ::rtt_target::rtt_init! {
            up: {
                1: { size: 512, name: "commands" }
            }
            down: {
                0: { size: 128, name: "commands" }
            }
        };

        (channels.up.0, channels.down.0, None::<()>)
    {% raw %}}}{% endraw %};
}

#[cfg(all(feature = "rtt-host-to-chip", feature = "defmt"))]
#[macro_export]
macro_rules! rtt_init {
    () => {% raw %}{{{% endraw %}
        let channels = ::rtt_target::rtt_init! {
            up: {
                0: { size: 1024, name: "defmt" }
                1: { size: 512, name: "commands" }
            }
            down: {
                0: { size: 128, name: "commands" }
            }
        };

        (channels.up.1, channels.down.0, channels.up.0)
    {% raw %}}}{% endraw %};
}

#[cfg(all(not(feature = "rtt-host-to-chip"), feature = "defmt"))]
#[macro_export]
macro_rules! rtt_init {
    () => {% raw %}{{{% endraw %}
        let channels = ::rtt_target::rtt_init! {
            up: {
                0: { size: 1024, name: "defmt" }
            }
        };

        (None::<()>, None::<()>, channels.up.0)
    {% raw %}}}{% endraw %};
}
