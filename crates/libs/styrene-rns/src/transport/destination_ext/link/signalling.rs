#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkSignalling {
    mode: u8,
    mtu: usize,
}

impl LinkSignalling {
    const AES_256_CBC_MODE: u8 = 1;
    const MTU_MASK: usize = 0x1f_ffff;

    pub(crate) fn base() -> Self {
        Self { mode: Self::AES_256_CBC_MODE, mtu: crate::packet::MTU }
    }

    pub(crate) fn for_mtu(mtu: usize) -> Self {
        Self {
            mode: Self::AES_256_CBC_MODE,
            mtu: mtu.clamp(crate::packet::MTU, crate::packet::MAX_LINK_MTU),
        }
    }

    pub(crate) fn decode(bytes: [u8; LINK_MTU_SIZE]) -> Result<Self, RnsError> {
        let mode = bytes[0] >> 5;
        let mtu = ((((bytes[0] & 0x1f) as usize) << 16)
            | ((bytes[1] as usize) << 8)
            | bytes[2] as usize)
            & Self::MTU_MASK;
        if mode != Self::AES_256_CBC_MODE || !(crate::packet::MTU..=Self::MTU_MASK).contains(&mtu) {
            return Err(RnsError::PacketError);
        }
        Ok(Self { mode, mtu })
    }

    pub(crate) fn encode(self) -> [u8; LINK_MTU_SIZE] {
        [
            (self.mode << 5) | ((self.mtu >> 16) as u8 & 0x1f),
            (self.mtu >> 8) as u8,
            self.mtu as u8,
        ]
    }

    pub(crate) fn mtu(self) -> usize {
        self.mtu
    }

    pub(crate) fn mode(self) -> u8 {
        self.mode
    }

    pub(crate) fn clamp(self, mtu: usize) -> Self {
        Self { mtu: self.mtu.min(mtu.max(crate::packet::MTU)), ..self }
    }
}

pub(crate) fn link_packet_mdu(mtu: usize) -> usize {
    mtu.saturating_sub(IFAC_MIN_SIZE + (2 + 1 + ADDRESS_HASH_SIZE) + 48) / 16 * 16 - 1
}

pub(crate) fn channel_mdu(mtu: usize) -> usize {
    link_packet_mdu(mtu).saturating_sub(6).min(u16::MAX as usize)
}

pub(crate) fn resource_sdu(mtu: usize) -> usize {
    mtu.saturating_sub(HEADER_MAXSIZE + IFAC_MIN_SIZE)
}
