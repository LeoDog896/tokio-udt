use super::socket::{SocketId, SocketType};
use crate::common::ip_to_bytes;
use crate::seq_number::{AckSeqNumber, MsgNumber, SeqNumber};
use std::net::IpAddr;
use tokio::io::{Error, ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct UdtControlPacket {
    // bit 0 = 1
    pub packet_type: ControlPacketType, // bits 1-15 + Control Information Field (bits 128+)
    pub reserved: u16,                  // bits 16-31
    pub additional_info: u32,           // bits 32-63
    pub timestamp: u32,                 // bits 64-95
    pub dest_socket_id: SocketId,       // bits 96-127
}

impl UdtControlPacket {
    pub fn new_handshake(hs: HandShakeInfo, dest_socket_id: SocketId) -> Self {
        Self {
            packet_type: ControlPacketType::Handshake(hs),
            reserved: 0,
            additional_info: 0,
            timestamp: 0, // TODO set timestamp here ?
            dest_socket_id,
        }
    }

    pub fn new_nak(loss_list: Vec<u32>, dest_socket_id: SocketId) -> Self {
        Self {
            packet_type: ControlPacketType::Nak(NakInfo {
                loss_info: loss_list,
            }),
            reserved: 0,
            additional_info: 0,
            timestamp: 0,
            dest_socket_id,
        }
    }

    pub fn new_ack2(seq: AckSeqNumber, dest_socket_id: SocketId) -> Self {
        Self {
            packet_type: ControlPacketType::Ack2,
            additional_info: seq.number(),
            dest_socket_id,
            reserved: 0,
            timestamp: 0,
        }
    }

    pub fn new_drop(
        msg_id: MsgNumber,
        first: SeqNumber,
        last: SeqNumber,
        dest_socket_id: SocketId,
    ) -> Self {
        Self {
            packet_type: ControlPacketType::MsgDropRequest(DropRequestInfo {
                first_seq_number: first,
                last_seq_number: last,
            }),
            additional_info: msg_id.number(),
            dest_socket_id,
            reserved: 0,
            timestamp: 0,
        }
    }

    pub fn new_keep_alive(dest_socket_id: SocketId) -> Self {
        Self {
            packet_type: ControlPacketType::KeepAlive,
            dest_socket_id,
            additional_info: 0,
            reserved: 0,
            timestamp: 0,
        }
    }

    pub fn new_shutdown(dest_socket_id: SocketId) -> Self {
        Self {
            packet_type: ControlPacketType::Shutdown,
            dest_socket_id,
            additional_info: 0,
            reserved: 0,
            timestamp: 0,
        }
    }

    pub fn new_ack(
        ack_number: AckSeqNumber,
        next_seq_number: SeqNumber,
        dest_socket_id: SocketId,
        info: Option<AckOptionalInfo>,
    ) -> Self {
        Self {
            packet_type: ControlPacketType::Ack(AckInfo {
                next_seq_number,
                info,
            }),
            dest_socket_id,
            additional_info: ack_number.number(),
            reserved: 0,
            timestamp: 0,
        }
    }

    pub fn ack_seq_number(&self) -> Option<AckSeqNumber> {
        match self.packet_type {
            ControlPacketType::Ack(_) | ControlPacketType::Ack2 => Some(self.additional_info.into()),
            _ => None,
        }
    }

    pub fn msg_seq_number(&self) -> Option<MsgNumber> {
        match self.packet_type {
            ControlPacketType::MsgDropRequest(_) => {
                Some((self.additional_info & MsgNumber::MAX_NUMBER).into())
            }
            _ => None,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::with_capacity(8);
        buffer.extend_from_slice(&(0x8000 + self.packet_type.type_as_u15()).to_be_bytes());
        buffer.extend_from_slice(&self.reserved.to_be_bytes());
        buffer.extend_from_slice(&self.additional_info.to_be_bytes());
        buffer.extend_from_slice(&self.timestamp.to_be_bytes());
        buffer.extend_from_slice(&self.dest_socket_id.to_be_bytes());
        buffer.extend_from_slice(&self.packet_type.control_info_field());
        buffer
    }

    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        if raw.len() < 16 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "control packet header is too short",
            ));
        }
        let reserved = u16::from_be_bytes(raw[2..4].try_into().unwrap());
        let additional_info = u32::from_be_bytes(raw[4..8].try_into().unwrap());
        let timestamp = u32::from_be_bytes(raw[8..12].try_into().unwrap());
        let dest_socket_id = u32::from_be_bytes(raw[12..16].try_into().unwrap());

        let packet_type = ControlPacketType::deserialize(raw)?;
        Ok(Self {
            packet_type,
            reserved,
            additional_info,
            timestamp,
            dest_socket_id,
        })
    }
}

#[derive(Debug)]
pub(crate) enum ControlPacketType {
    Handshake(HandShakeInfo),
    KeepAlive,
    Ack(AckInfo),
    Nak(NakInfo),
    Shutdown,
    Ack2,
    MsgDropRequest(DropRequestInfo),
    UserDefined,
}

impl ControlPacketType {
    pub fn type_as_u15(&self) -> u16 {
        match self {
            Self::Handshake(_) => 0x0000,
            Self::KeepAlive => 0x0001,
            Self::Ack(_) => 0x0002,
            Self::Nak(_) => 0x0003,
            Self::Shutdown => 0x0005,
            Self::Ack2 => 0x0006,
            Self::MsgDropRequest(_) => 0x0007,
            Self::UserDefined => 0x7fff,
        }
    }

    pub fn control_info_field(&self) -> Vec<u8> {
        match self {
            Self::Handshake(hs) => hs.serialize(),
            Self::Ack(ack) => ack.serialize(),
            Self::Nak(nak) => nak.serialize(),
            Self::MsgDropRequest(drop) => drop.serialize(),
            _ => vec![],
        }
    }

    pub fn deserialize(raw_control_packet: &[u8]) -> Result<Self> {
        let type_id = u16::from_be_bytes(raw_control_packet[0..2].try_into().unwrap()) & 0x7FFF;
        let packet = match type_id {
            0x0000 => Self::Handshake(HandShakeInfo::deserialize(&raw_control_packet[16..])?),
            0x0001 => Self::KeepAlive,
            0x0002 => Self::Ack(AckInfo::deserialize(&raw_control_packet[16..])),
            0x0003 => Self::Nak(NakInfo::deserialize(&raw_control_packet[16..])),
            0x0005 => Self::Shutdown,
            0x0006 => Self::Ack2,
            0x0007 => {
                Self::MsgDropRequest(DropRequestInfo::deserialize(&raw_control_packet[16..]))
            }
            0x7fff => Self::UserDefined,
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "unknown control packet type",
                ));
            }
        };
        Ok(packet)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HandShakeInfo {
    pub udt_version: u32,
    pub socket_type: SocketType,
    pub initial_seq_number: SeqNumber,
    pub max_packet_size: u32,
    pub max_window_size: u32,
    pub connection_type: i32, // regular or rendezvous
    pub socket_id: SocketId,
    pub syn_cookie: u32,
    pub ip_address: IpAddr,
}

impl HandShakeInfo {
    pub fn serialize(&self) -> Vec<u8> {
        [
            self.udt_version,
            self.socket_type as u32,
            self.initial_seq_number.number(),
            self.max_packet_size,
            self.max_window_size,
        ]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .chain(self.connection_type.to_be_bytes().into_iter())
        .chain(self.socket_id.to_be_bytes().into_iter())
        .chain(self.syn_cookie.to_be_bytes().into_iter())
        .chain(ip_to_bytes(self.ip_address))
        .collect()
    }

    pub fn deserialize(raw: &[u8]) -> Result<Self> {
        let get_u32 =
            |idx: usize| u32::from_be_bytes(raw[(idx * 4)..(idx + 1) * 4].try_into().unwrap());
        let addr: IpAddr = {
            if raw[36..48].iter().all(|b| *b == 0) {
                // IPv4
                let octets: [u8; 4] = raw[32..36].try_into().unwrap();
                octets.into()
            } else {
                // IPv6
                let octets: [u8; 16] = raw[32..48].try_into().unwrap();
                octets.into()
            }
        };

        Ok(Self {
            udt_version: get_u32(0),
            socket_type: get_u32(1).try_into()?,
            initial_seq_number: get_u32(2).into(),
            max_packet_size: get_u32(3),
            max_window_size: get_u32(4),
            connection_type: i32::from_be_bytes(raw[20..24].try_into().unwrap()),
            socket_id: get_u32(6),
            syn_cookie: get_u32(7),
            ip_address: addr,
        })
    }
}

#[derive(Debug)]
pub(crate) struct AckInfo {
    /// The packet sequence number to which all the
    /// previous packets have been received (excluding)
    pub next_seq_number: SeqNumber,
    pub info: Option<AckOptionalInfo>,
}

impl AckInfo {
    pub fn deserialize(raw: &[u8]) -> Self {
        let get_u32 =
            |idx: usize| u32::from_be_bytes(raw[(idx * 4)..(idx + 1) * 4].try_into().unwrap());

        let next_seq_number: SeqNumber = get_u32(0).into();

        if raw.len() <= 4 {
            return Self {
                next_seq_number,
                info: None,
            };
        }
        let info = AckOptionalInfo {
            rtt: get_u32(1),
            rtt_variance: get_u32(2),
            available_buf_size: get_u32(3),
            pack_recv_rate: get_u32(4),
            link_capacity: get_u32(5),
        };
        Self {
            next_seq_number,
            info: Some(info),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        match &self.info {
            None => self.next_seq_number.number().to_be_bytes().to_vec(),
            Some(extra) => [
                self.next_seq_number.number(),
                extra.rtt,
                extra.rtt_variance,
                extra.available_buf_size,
                extra.pack_recv_rate,
                extra.link_capacity,
            ]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct AckOptionalInfo {
    /// RTT in microseconds
    pub rtt: u32,
    pub rtt_variance: u32,
    pub available_buf_size: u32,
    pub pack_recv_rate: u32,
    pub link_capacity: u32,
}

#[derive(Debug)]
pub(crate) struct NakInfo {
    pub loss_info: Vec<u32>,
}

impl NakInfo {
    pub fn deserialize(raw: &[u8]) -> Self {
        let losses: Vec<u32> = raw
            .chunks(4)
            .filter_map(|chunk| {
                if chunk.len() < 4 {
                    return None;
                }
                Some(u32::from_be_bytes(chunk.try_into().unwrap()))
            })
            .collect();
        Self { loss_info: losses }
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.loss_info
            .iter()
            .flat_map(|x| x.to_be_bytes())
            .collect()
    }
}

#[derive(Debug)]
pub(crate) struct DropRequestInfo {
    pub first_seq_number: SeqNumber,
    pub last_seq_number: SeqNumber,
}

impl DropRequestInfo {
    pub fn deserialize(raw: &[u8]) -> Self {
        let get_u32 =
            |idx: usize| u32::from_be_bytes(raw[(idx * 4)..(idx + 1) * 4].try_into().unwrap());

        Self {
            first_seq_number: get_u32(0).into(),
            last_seq_number: get_u32(1).into(),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        [
            self.first_seq_number.number(),
            self.last_seq_number.number(),
        ]
        .iter()
        .flat_map(|x| x.to_be_bytes())
        .collect()
    }
}
