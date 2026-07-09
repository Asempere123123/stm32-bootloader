use rtt_target::DownChannel;

pub struct HostToChip {
    down: DownChannel,
}

impl HostToChip {
    pub fn new(channel: DownChannel) -> Self {
        Self { down: channel }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        self.down.read(buf)
    }

    #[cfg(feature = "defmt")]
    pub fn echo_loop(&mut self) -> ! {
        let mut buf = [0];
        loop {
            self.read(&mut buf);
            defmt::info!("RECV FROM HOST: {}", buf[0]);
        }
    }
}
