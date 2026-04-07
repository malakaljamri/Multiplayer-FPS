use std::io;
use std::net::{SocketAddr, ToSocketAddrs};

use log::{debug, info, warn};

use crate::protocol::{self, Packet, PacketHeader, WirePacket, MAX_PACKET_SIZE};

/// A thin wrapper around `std::net::UdpSocket` that enforces
/// non-blocking mode and adds typed packet send/receive.
pub struct UdpSocket {
    inner: std::net::UdpSocket,
    /// Local address the socket is bound to.
    local_addr: SocketAddr,
    /// Next outgoing sequence number.
    next_sequence: u32,
    /// Last sequence number received from the remote side (for ack).
    last_remote_sequence: u32,
}

impl UdpSocket {
    /// Bind to the given address and set the socket to non-blocking mode.
    pub fn bind<A: ToSocketAddrs>(addr: A) -> io::Result<Self> {
        let socket = std::net::UdpSocket::bind(&addr)?;
        socket.set_nonblocking(true)?;
        let local_addr = socket.local_addr()?;
        info!("UDP socket bound to {}", local_addr);
        Ok(Self {
            inner: socket,
            local_addr,
            next_sequence: 0,
            last_remote_sequence: 0,
        })
    }

    // -------------------------------------------------------------------
    // Raw byte API (still available for low-level use)
    // -------------------------------------------------------------------

    /// Send raw bytes to the given destination.
    pub fn send_to<A: ToSocketAddrs>(&self, data: &[u8], addr: A) -> io::Result<usize> {
        self.inner.send_to(data, addr)
    }

    /// Non-blocking raw receive. Returns `Ok(None)` when no data is ready.
    pub fn recv_from_raw(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        match self.inner.recv_from(buf) {
            Ok((n, addr)) => Ok(Some((n, addr))),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    // -------------------------------------------------------------------
    // Typed packet API
    // -------------------------------------------------------------------

    /// Serialize and send a `Packet` to the given address.
    ///
    /// Automatically assigns the next sequence number and fills the ack field.
    pub fn send_packet<A: ToSocketAddrs>(&mut self, payload: Packet, addr: A) -> io::Result<()> {
        let header = PacketHeader {
            sequence: self.next_sequence,
            ack: self.last_remote_sequence,
            timestamp: 0.0, // placeholder — real wall-clock added later
        };
        self.next_sequence = self.next_sequence.wrapping_add(1);

        let wire = WirePacket { header, payload };
        match protocol::serialize(&wire) {
            Some(bytes) => {
                debug!("TX seq={} ({} bytes)", wire.header.sequence, bytes.len());
                self.inner.send_to(&bytes, addr)?;
                Ok(())
            }
            None => {
                warn!("Failed to serialize packet — dropped");
                Ok(())
            }
        }
    }

    /// Non-blocking receive of a typed packet.
    ///
    /// Returns `Ok(Some((header, payload, source_addr)))` if a valid packet
    /// was available, `Ok(None)` if the socket would block, or an error.
    pub fn recv_packet(&mut self) -> io::Result<Option<(PacketHeader, Packet, SocketAddr)>> {
        let mut buf = [0u8; MAX_PACKET_SIZE];
        match self.inner.recv_from(&mut buf) {
            Ok((n, addr)) => match protocol::deserialize(&buf[..n]) {
                Some(wire) => {
                    let seq = wire.header.sequence;
                    let last = self.last_remote_sequence;
                    if seq.wrapping_sub(last) < u32::MAX / 2 {
                        self.last_remote_sequence = seq;
                    }
                    debug!(
                        "RX seq={} from {} ({} bytes)",
                        wire.header.sequence, addr, n
                    );
                    Ok(Some((wire.header, wire.payload, addr)))
                }
                None => {
                    warn!(
                        "Received {} bytes from {} but deserialization failed",
                        n, addr
                    );
                    Ok(None)
                }
            },
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    // -------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------

    /// Returns the local address this socket is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Allocate a receive buffer of `MAX_PACKET_SIZE`.
    pub fn new_recv_buffer() -> [u8; MAX_PACKET_SIZE] {
        [0u8; MAX_PACKET_SIZE]
    }
}
