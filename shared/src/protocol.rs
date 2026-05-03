use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum size of a single UDP packet payload in bytes.
pub const MAX_PACKET_SIZE: usize = 2048;

/// Default server port.
pub const DEFAULT_SERVER_PORT: u16 = 7878;

/// Default server address.
pub const DEFAULT_SERVER_ADDR: &str = "127.0.0.1";

/// Server tick rate (ticks per second).
pub const TICK_RATE: u32 = 60;

/// Duration of a single tick in seconds.
pub const TICK_DURATION: f64 = 1.0 / TICK_RATE as f64;

/// Player movement speed (units per second).
pub const MOVE_SPEED: f32 = 1.2;

/// Player turning speed (radians per second).
pub const TURN_SPEED: f32 = 3.0;

/// Starting health.
pub const MAX_HEALTH: u8 = 100;

/// Damage dealt by a single hitscan shot.
pub const WEAPON_DAMAGE: u8 = 25;

/// Number of ticks between allowed shots (e.g. 30 ticks = 0.5 sec at 60Hz).
pub const WEAPON_COOLDOWN_TICKS: u32 = 30;

// ---------------------------------------------------------------------------
// Shared game types
// ---------------------------------------------------------------------------

/// Snapshot of one player's state, broadcast by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: u32,
    pub name: [u8; 24],
    pub x: f32,
    pub y: f32,
    pub angle: f32,
    pub health: u8,
    pub frags: u32,
    pub is_game_over: bool,
}

/// Client input intent for a single frame.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct InputData {
    pub forward: bool,
    pub backward: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub shoot: bool,
    /// Angle delta in radians from mouse movement this frame.
    pub turn_delta: f32,
}

// ---------------------------------------------------------------------------
// Packet header
// ---------------------------------------------------------------------------

use crate::maze::Difficulty;

/// Every packet carries a header for ordering & acknowledgement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketHeader {
    /// Monotonically increasing sequence number set by the sender.
    pub sequence: u32,
    /// The last sequence number the sender received from the remote side.
    pub ack: u32,
    /// Wall-clock timestamp (seconds) for RTT estimation.
    pub timestamp: f64,
}

impl PacketHeader {
    pub fn new(sequence: u32) -> Self {
        Self {
            sequence,
            ack: 0,
            timestamp: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Packet — the top-level protocol message
// ---------------------------------------------------------------------------

/// All packet types exchanged between client and server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Packet {
    // -- Connection lifecycle --
    /// Client → Server: request to join.
    Connect {
        player_name: String,
    },
    /// Server → Client: accepted with assigned player id.
    Accept {
        player_id: u32,
        seed: u64,
        difficulty: Difficulty,
    },
    /// Either direction: graceful disconnect.
    Disconnect,

    // -- Keep-alive --
    Ping,
    Pong,

    // -- Level progression --
    /// Server → Client: advance to a new level with a new maze.
    LevelChange {
        seed: u64,
        difficulty: Difficulty,
        level: u8,
    },

    // -- Gameplay --
    /// Client → Server: movement input for a frame.
    Input {
        input: InputData,
        /// Client-side input sequence for reconciliation.
        input_sequence: u32,
    },
    /// Server → Client: authoritative world state.
    StateSnapshot {
        tick: u32,
        players: Vec<PlayerState>,
    },
    /// Server → Client: global message (kill feed, notifications).
    ServerMessage {
        text: String,
    },
    /// Client → Server: request to respawn after death.
    Respawn,
}

// ---------------------------------------------------------------------------
// Wire format helpers
// ---------------------------------------------------------------------------

/// A complete wire message: header + payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WirePacket {
    pub header: PacketHeader,
    pub payload: Packet,
}

/// Serialize a `WirePacket` into bytes (returns `None` if too large).
pub fn serialize(packet: &WirePacket) -> Option<Vec<u8>> {
    let bytes = bincode::serialize(packet).ok()?;
    if bytes.len() > MAX_PACKET_SIZE {
        log::warn!(
            "Packet too large ({} bytes, max {})",
            bytes.len(),
            MAX_PACKET_SIZE
        );
        return None;
    }
    Some(bytes)
}

/// Deserialize bytes into a `WirePacket`.
pub fn deserialize(bytes: &[u8]) -> Option<WirePacket> {
    bincode::deserialize(bytes).ok()
}
pub fn encode_name(name: &str) -> [u8; 24] {
    let mut buf = [0u8; 24];
    let bytes = name.as_bytes();

    let len = bytes.len().min(24);
    buf[..len].copy_from_slice(&bytes[..len]);

    buf
}

pub fn decode_name(buf: &[u8; 24]) -> &str {
    let s = std::str::from_utf8(buf).unwrap_or("");
    s.trim_end_matches('\0')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_connect() {
        let wp = WirePacket {
            header: PacketHeader::new(1),
            payload: Packet::Connect {
                player_name: "Alice".into(),
            },
        };
        let bytes = serialize(&wp).expect("serialize");
        let decoded = deserialize(&bytes).expect("deserialize");
        assert_eq!(decoded.header.sequence, 1);
        match decoded.payload {
            Packet::Connect { player_name } => assert_eq!(player_name, "Alice"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_accept() {
        let wp = WirePacket {
            header: PacketHeader::new(42),
            payload: Packet::Accept {
                player_id: 7,
                seed: 12345,
                difficulty: Difficulty::Medium,
            },
        };
        let bytes = serialize(&wp).expect("serialize");
        let decoded = deserialize(&bytes).expect("deserialize");
        match decoded.payload {
            Packet::Accept {
                player_id,
                seed,
                difficulty,
            } => {
                assert_eq!(player_id, 7);
                assert_eq!(seed, 12345);
                assert_eq!(difficulty, Difficulty::Medium);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_disconnect() {
        let wp = WirePacket {
            header: PacketHeader::new(0),
            payload: Packet::Disconnect,
        };
        let bytes = serialize(&wp).expect("serialize");
        let decoded = deserialize(&bytes).expect("deserialize");
        assert!(matches!(decoded.payload, Packet::Disconnect));
    }

    #[test]
    fn round_trip_ping_pong() {
        for payload in [Packet::Ping, Packet::Pong] {
            let wp = WirePacket {
                header: PacketHeader::new(99),
                payload,
            };
            let bytes = serialize(&wp).expect("serialize");
            let _ = deserialize(&bytes).expect("deserialize");
        }
    }

    #[test]
    fn round_trip_input() {
        let wp = WirePacket {
            header: PacketHeader::new(5),
            payload: Packet::Input {
                input: InputData {
                    forward: true,
                    backward: false,
                    strafe_left: false,
                    strafe_right: true,
                    shoot: true,
                    turn_delta: 0.05,
                },
                input_sequence: 10,
            },
        };
        let bytes = serialize(&wp).expect("serialize");
        let decoded = deserialize(&bytes).expect("deserialize");
        match decoded.payload {
            Packet::Input {
                input,
                input_sequence,
            } => {
                assert!(input.forward);
                assert!(!input.backward);
                assert!(input.strafe_right);
                assert!(input.shoot);
                assert!((input.turn_delta - 0.05).abs() < 0.0001);
                assert_eq!(input_sequence, 10);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_state_snapshot() {
        let wp = WirePacket {
            header: PacketHeader::new(100),
            payload: Packet::StateSnapshot {
                tick: 42,
                players: vec![
                    PlayerState {
                        id: 1,
                        x: 1.0,
                        y: 2.0,
                        name: [0; 24],
                        angle: 0.5,
                        frags: 0,
                        health: 100,
                        is_game_over: false,
                    },
                    PlayerState {
                        id: 2,
                        x: 5.0,
                        y: 3.0,
                        name: [0; 24],
                        angle: 1.2,
                        frags: 0,
                        health: 80,
                        is_game_over: false,
                    },
                ],
            },
        };
        let bytes = serialize(&wp).expect("serialize");
        let decoded = deserialize(&bytes).expect("deserialize");
        match decoded.payload {
            Packet::StateSnapshot { tick, players } => {
                assert_eq!(tick, 42);
                assert_eq!(players.len(), 2);
                assert_eq!(players[0].id, 1);
                assert_eq!(players[1].health, 80);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_server_message() {
        let wp = WirePacket {
            header: PacketHeader::new(101),
            payload: Packet::ServerMessage {
                text: "Hello World".into(),
            },
        };
        let bytes = serialize(&wp).expect("serialize");
        let decoded = deserialize(&bytes).expect("deserialize");
        match decoded.payload {
            Packet::ServerMessage { text } => {
                assert_eq!(text, "Hello World");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn round_trip_level_change() {
        let wp = WirePacket {
            header: PacketHeader::new(200),
            payload: Packet::LevelChange {
                seed: 99999,
                difficulty: Difficulty::Hard,
                level: 3,
            },
        };
        let bytes = serialize(&wp).expect("serialize");
        let decoded = deserialize(&bytes).expect("deserialize");
        match decoded.payload {
            Packet::LevelChange { seed, difficulty, level } => {
                assert_eq!(seed, 99999);
                assert_eq!(difficulty, Difficulty::Hard);
                assert_eq!(level, 3);
            }
            _ => panic!("wrong variant"),
        }
    }
}
