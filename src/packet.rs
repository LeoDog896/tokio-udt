use super::control_packet::{ControlPacketType, HandShakeInfo, UdtControlPacket};
use super::data_packet::UdtDataPacket;
use tokio::io::{Error, ErrorKind, Result};

#[derive(Debug)]
pub(crate) enum UdtPacket {
    Control(UdtControlPacket),
    Data(UdtDataPacket),
}

/// Defines a packet that cna be sent in a UDT protocol
impl UdtPacket {
    pub fn get_dest_socket_id(&self) -> u32 {
        match self {
            Self::Control(p) => p.dest_socket_id,
            Self::Data(p) => p.header.dest_socket_id,
        }
    }

    /// Serializes this packet into a buffer
    pub fn serialize(&self) -> Vec<u8> {
        match self {
            Self::Control(p) => p.serialize(),
            Self::Data(p) => p.serialize(),
        }
    }

    /// Deserializes this buffer into a packet.
    ///
    /// This cannot deserialize empty packets.
    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        if raw.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "cannot deserialize empty packet",
            ));
        }
        let first_bit = (raw[0] >> 7) != 0;
        let packet = match first_bit {
            false => Self::Data(UdtDataPacket::deserialize(raw)?),
            true => Self::Control(UdtControlPacket::deserialize(raw)?),
        };
        Ok(packet)
    }

    pub fn handshake(&self) -> Option<&HandShakeInfo> {
        match self {
            Self::Control(ctrl) => match &ctrl.packet_type {
                ControlPacketType::Handshake(info) => Some(info),
                _ => None,
            },
            _ => None,
        }
    }
}

impl From<UdtControlPacket> for UdtPacket {
    fn from(ctrl: UdtControlPacket) -> Self {
        Self::Control(ctrl)
    }
}

impl From<UdtDataPacket> for UdtPacket {
    fn from(data_packet: UdtDataPacket) -> Self {
        Self::Data(data_packet)
    }
}
