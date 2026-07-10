use rtt_target::{DownChannel, UpChannel};

pub struct HostToChip {
    down: DownChannel,
    up: UpChannel,
}

impl HostToChip {
    pub fn new(down_channel: DownChannel, up_channel: UpChannel) -> Self {
        Self {
            down: down_channel,
            up: up_channel,
        }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        self.down.read(buf)
    }

    pub fn write(&mut self, buf: &[u8]) -> usize {
        self.up.write(buf)
    }
}
