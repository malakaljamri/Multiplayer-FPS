mod game;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use log::{info, warn};

use shared::network::UdpSocket;
use shared::protocol::{Packet, DEFAULT_SERVER_PORT, TICK_DURATION};

use crate::game::GameState;

fn main() {
    env_logger::init();

    let bind_addr = format!("0.0.0.0:{}", DEFAULT_SERVER_PORT);
    info!("Starting server on {}", bind_addr);

    let mut socket = UdpSocket::bind(&bind_addr).expect("Failed to bind server socket");
    info!(
        "Server listening on {} (non-blocking, tick={:.1}ms)",
        socket.local_addr(),
        TICK_DURATION * 1000.0
    );

    use rand::Rng;
    use shared::maze::Difficulty;

    let seed: u64 = rand::thread_rng().gen();
    let difficulty = Difficulty::Easy;
    let mut game = GameState::new(seed, difficulty);
    // Map socket address → player id for quick lookup.
    let mut addr_to_id: HashMap<SocketAddr, u32> = HashMap::new();
    let mut next_player_id: u32 = 1;

    let tick_duration = Duration::from_secs_f64(TICK_DURATION);
    let mut last_tick = Instant::now();

    // --- Main loop ---
    loop {
        // ---- 1. Drain all incoming packets ----
        loop {
            match socket.recv_packet() {
                Ok(Some((header, packet, src))) => {
                    match packet {
                        // ---- Connect ----
                        Packet::Connect { player_name } => {
                            if addr_to_id.contains_key(&src) {
                                warn!("Duplicate connect from {} — ignoring", src);
                                continue;
                            }
                            let player_id = next_player_id;
                            next_player_id += 1;

                            game.add_player(player_id, player_name.clone(), src);
                            addr_to_id.insert(src, player_id);

                            info!(
                                "Player '{}' connected from {} → id {} (seq={})",
                                player_name, src, player_id, header.sequence
                            );

                            if let Err(e) = socket.send_packet(
                                Packet::Accept {
                                    player_id,
                                    seed: game.seed,
                                    difficulty: game.difficulty,
                                },
                                src,
                            ) {
                                warn!("Failed to send Accept to {}: {}", src, e);
                            }
                        }

                        // ---- Disconnect ----
                        Packet::Disconnect => {
                            if let Some(id) = addr_to_id.remove(&src) {
                                game.remove_player(id);
                                info!("Player id={} disconnected ({})", id, src);
                            }
                        }

                        // ---- Input ----
                        Packet::Input {
                            input,
                            input_sequence,
                        } => {
                            if let Some(&id) = addr_to_id.get(&src) {
                                game.process_input(id, &input, input_sequence);
                            }
                        }

                        // ---- Ping ----
                        Packet::Ping => {
                            let _ = socket.send_packet(Packet::Pong, src);
                        }

                        _ => {}
                    }
                }
                Ok(None) => break, // No more data.
                Err(e) => {
                    log::error!("recv error: {}", e);
                    break;
                }
            }
        }

        // ---- 2. Fixed-rate tick ----
        let now = Instant::now();
        if now.duration_since(last_tick) >= tick_duration {
            last_tick = now;
            game.tick();

            // ---- 3. Broadcast state snapshot ----
            let snapshot = game.snapshot();
            let tick = game.tick;
            let state_packet = Packet::StateSnapshot {
                tick,
                players: snapshot,
            };

            // Collect addresses first to avoid borrow conflict.
            let addrs: Vec<SocketAddr> = addr_to_id.keys().copied().collect();

            // Broadcast ServerMessages
            if !game.message_queue.is_empty() {
                for msg in game.message_queue.drain(..) {
                    let msg_packet = Packet::ServerMessage { text: msg };
                    for addr in &addrs {
                        let _ = socket.send_packet(msg_packet.clone(), *addr);
                    }
                }
            }

            for addr in addrs {
                if let Err(e) = socket.send_packet(state_packet.clone(), addr) {
                    warn!("Failed to send snapshot to {}: {}", addr, e);
                }
            }
        } else {
            // Sleep until next tick (avoid busy-spin).
            let remaining = tick_duration.saturating_sub(now.duration_since(last_tick));
            if remaining > Duration::from_micros(500) {
                std::thread::sleep(Duration::from_micros(500));
            }
        }
    }
}
