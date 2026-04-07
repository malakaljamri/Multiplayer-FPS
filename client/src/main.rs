use std::net::SocketAddr;

use log::{error, info};
use macroquad::prelude::*;

use shared::maze::Map;
use shared::network::UdpSocket;
use shared::protocol::{InputData, Packet, DEFAULT_SERVER_ADDR, DEFAULT_SERVER_PORT};

pub mod interpolation;
pub mod prediction;
pub mod renderer;

use interpolation::InterpolationRemote;
use prediction::PredictionLocal;
use renderer::Renderer;

#[macroquad::main("Maze Wars 3D")]
async fn main() {
    env_logger::init();
    info!("Starting graphical client...");

    let server_addr: SocketAddr = format!("{}:{}", DEFAULT_SERVER_ADDR, DEFAULT_SERVER_PORT)
        .parse()
        .expect("Invalid server address format");

    let mut socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind client socket");

    let player_name = format!("Player_{}", macroquad::rand::rand() % 10000);
    info!("Connecting as {}...", player_name);

    let connect_packet = Packet::Connect { player_name };
    socket
        .send_packet(connect_packet, &server_addr)
        .expect("Send Connect failed");

    // --- Wait for Accept Packet ---
    // Since socket is non-blocking, we loop with a small async sleep until connected
    let (my_player_id, seed, difficulty) = loop {
        match socket.recv_packet() {
            Ok(Some((
                _header,
                Packet::Accept {
                    player_id,
                    seed,
                    difficulty,
                },
                _src,
            ))) => {
                info!("Connected! Assigned Player ID: {}", player_id);
                break (player_id, seed, difficulty);
            }
            Ok(Some(_)) => {} // Ignore other packets
            Ok(None) => {}    // No packet yet
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                error!("recv error: {}", e);
                std::process::exit(1);
            }
        }

        // Draw a simple loading screen
        clear_background(BLACK);
        draw_text(
            "Connecting to server...",
            screen_width() / 2.0 - 100.0,
            screen_height() / 2.0,
            30.0,
            WHITE,
        );
        macroquad::window::next_frame().await;
    };

    let map = Map::generate(seed, difficulty);
    let mut prediction = PredictionLocal::new();
    prediction.map = Some(map);
    let mut interpolation = InterpolationRemote::new(100);

    let mut renderer = Renderer::new();
    let mut messages: Vec<String> =
        vec!["Connected to server. Controls: WASD to move, Q to quit.".to_string()];

    // --- Main loop ---
    let mut input_sequence: u32 = 0;
    let mut last_snapshot_tick: u32 = 0;

    // Macroquad uses `next_frame().await` to yield to the OS and cap at Vsync (~60fps)
    loop {
        // Handle quitting
        if is_key_pressed(KeyCode::Q) || is_key_pressed(KeyCode::Escape) {
            break;
        }

        // ---- 1. Process Local Input ----
        let mut input = InputData::default();
        if is_key_down(KeyCode::W) {
            input.forward = true;
        }
        if is_key_down(KeyCode::S) {
            input.backward = true;
        }
        if is_key_down(KeyCode::A) {
            input.turn_left = true;
        }
        if is_key_down(KeyCode::D) {
            input.turn_right = true;
        }
        if is_key_pressed(KeyCode::Space) {
            input.shoot = true;
        } // Use is_key_pressed so it doesn't spam every frame

        let has_input =
            input.forward || input.backward || input.turn_left || input.turn_right || input.shoot;

        if has_input {
            input_sequence += 1;

            // Send to server
            let _ = socket.send_packet(
                Packet::Input {
                    input,
                    input_sequence,
                },
                &server_addr,
            );

            // Apply locally (Prediction)
            prediction.add_input(input_sequence, input);
        }

        // ---- 2. Receive Network Packets ----
        loop {
            match socket.recv_packet() {
                Ok(Some((header, Packet::StateSnapshot { tick, players }, _src))) => {
                    if tick > last_snapshot_tick {
                        last_snapshot_tick = tick;

                        // Reconcile local player
                        if let Some(local_server_state) =
                            players.iter().find(|p| p.id == my_player_id)
                        {
                            prediction.reconcile(local_server_state, tick, header.ack);
                        }

                        // Feed remote players into interpolation buffer
                        interpolation.push_snapshot(tick, players, my_player_id);
                    }
                }
                Ok(Some((_h, Packet::Pong, _src))) => {
                    info!("Pong received");
                }
                Ok(Some((_h, Packet::ServerMessage { text }, _src))) => {
                    messages.push(text);
                }
                Ok(Some(_)) => {}
                Ok(None) => break, // Socket is non-blocking, break when empty
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break, // Also empty
                Err(e) => {
                    error!("recv error: {}", e);
                    break;
                }
            }
        }

        // Keep message queue short
        if messages.len() > 6 {
            messages.remove(0);
        }

        // ---- 3. Render Output ----
        let remote_states = interpolation.get_interpolated_state();
        let map_ref = prediction.map.as_ref().unwrap();

        renderer.render(
            &prediction.local_state,
            &remote_states,
            map_ref,
            &messages,
            input.shoot,
        );

        // Yield to OS, swap buffers, and wait for vsync
        macroquad::window::next_frame().await;
    }

    // Graceful disconnect.
    info!("Disconnecting...");
    let _ = socket.send_packet(Packet::Disconnect, &server_addr);
}
