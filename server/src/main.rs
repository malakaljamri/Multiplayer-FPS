mod game;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use log::{info, warn};
use rand::Rng;

use shared::maze::Difficulty;
use shared::network::UdpSocket;
use shared::protocol::{Packet, DEFAULT_SERVER_PORT, TICK_DURATION};

use crate::game::GameState;
fn get_local_ip() -> Option<String> {
    use std::net::{UdpSocket, Ipv4Addr};

    // Create a temporary socket to discover the local IP
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let local_addr = socket.local_addr().ok()?;

    match local_addr.ip() {
        std::net::IpAddr::V4(ip) if ip != Ipv4Addr::new(127, 0, 0, 1) => {
            Some(ip.to_string())
        }
            _ => None
    }
}
fn main() {
    env_logger::init();

    let bind_addr = format!("0.0.0.0:{}", DEFAULT_SERVER_PORT);
    info!("Starting Maze Wars server on {}", bind_addr);

    let mut socket = UdpSocket::bind(&bind_addr).expect("Failed to bind server socket");
    info!(
        "Server listening on {} (tick={:.1}ms)",
        socket.local_addr(),
        TICK_DURATION * 1000.0
    );
    // Print the IP address for other players to connect
    if let Some(local_ip) = get_local_ip() {
        println!("\n=== SERVER IP ADDRESS ===");
        println!("Other players can connect using: {}:{}", local_ip, DEFAULT_SERVER_PORT);
        println!("=========================\n");
    } else {
        println!("\n=== SERVER IP ADDRESS ===");
        println!("Could not detect local IP automatically.");
        println!("Use 'ipconfig' (Windows) or 'ifconfig' (Mac/Linux) to find your IP.");
        println!("Then connect using: YOUR_IP:{}", DEFAULT_SERVER_PORT);
        println!("=========================\n");
    }
    

    let seed: u64 = rand::thread_rng().gen();
    let difficulty = Difficulty::Easy; // Level 1 starts Easy
    let mut game = GameState::new(seed, difficulty);
    let mut addr_to_id: HashMap<SocketAddr, u32> = HashMap::new();
    let mut next_player_id: u32 = 1;

    let tick_duration = Duration::from_secs_f64(TICK_DURATION);
    let mut last_tick = Instant::now();

    loop {
        // ---- 1. Drain incoming packets ----
        loop {
            match socket.recv_packet() {
                Ok(Some((header, packet, src))) => {
                    match packet {
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
                                "Player '{}' connected from {} → id={} level={} (seq={})",
                                player_name, src, player_id, game.level, header.sequence
                            );

                            // Send Accept with current level's seed/difficulty
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

                            // If not on level 1, tell them the current level
                            if game.level > 1 {
                                let _ = socket.send_packet(
                                    Packet::LevelChange {
                                        seed: game.seed,
                                        difficulty: game.difficulty,
                                        level: game.level,
                                    },
                                    src,
                                );
                            }
                        }

                        Packet::Disconnect => {
                            if let Some(id) = addr_to_id.remove(&src) {
                                game.remove_player(id);
                                info!("Player id={} disconnected ({})", id, src);
                            }
                        }

                        Packet::Input {
                            input,
                            input_sequence,
                        } => {
                            if let Some(&id) = addr_to_id.get(&src) {
                                game.process_input(id, &input, input_sequence);
                            }
                        }

                        Packet::Ping => {
                            let _ = socket.send_packet(Packet::Pong, src);
                        }

                        Packet::Respawn => {
                            if let Some(&id) = addr_to_id.get(&src) {
                                if game.respawn_player(id) {
                                    info!("Player id={} respawned", id);
                                }
                            }
                        }

                        _ => {}
                    }
                }
                Ok(None) => break,
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

            let addrs: Vec<SocketAddr> = addr_to_id.keys().copied().collect();

            // ---- 3. Broadcast server messages ----
            if !game.message_queue.is_empty() {
                for msg in game.message_queue.drain(..) {
                    let msg_packet = Packet::ServerMessage { text: msg };
                    for addr in &addrs {
                        let _ = socket.send_packet(msg_packet.clone(), *addr);
                    }
                }
            }

            // ---- 4. Broadcast level change if needed ----
            if let Some(change) = game.pending_level_change.take() {
                info!(
                    "Level advance → level={} difficulty={:?} seed={}",
                    change.level, change.difficulty, change.seed
                );
                let lc = Packet::LevelChange {
                    seed: change.seed,
                    difficulty: change.difficulty,
                    level: change.level,
                };
                for addr in &addrs {
                    let _ = socket.send_packet(lc.clone(), *addr);
                }
            }

            // ---- 5. Broadcast state snapshot ----
            let snapshot = game.snapshot();
            let state_packet = Packet::StateSnapshot {
                tick: game.tick,
                players: snapshot,
            };

            for addr in addrs {
                if let Err(e) = socket.send_packet(state_packet.clone(), addr) {
                    warn!("Failed to send snapshot to {}: {}", addr, e);
                }
            }
        } else {
            let remaining = tick_duration.saturating_sub(now.duration_since(last_tick));
            if remaining > Duration::from_micros(500) {
                std::thread::sleep(Duration::from_micros(500));
            }
        }
    }
}
