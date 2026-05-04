pub mod interpolation;
pub mod prediction;
pub mod renderer;

use log::{error, info};
use macroquad::prelude::*;
use std::time::{Duration, Instant};

use shared::maze::Map;
use shared::network::UdpSocket;
use shared::physics::cast_ray_hit;
use shared::protocol::{decode_name, Packet, PlayerState, DEFAULT_SERVER_PORT};

use interpolation::InterpolationRemote;
use prediction::PredictionLocal;
use renderer::{BulletMark, Renderer};

const MAX_BULLET_MARKS: usize = 64;

enum GameLoopControl {
    Exit,
    Restart,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsSelector {
    Dashboard,
    Settings,
}

fn window_conf() -> macroquad::conf::Conf {
    macroquad::conf::Conf {
        miniquad_conf: macroquad::miniquad::conf::Conf {
            window_title: String::from("Maze Wars"),
            window_width: 960,
            window_height: 600,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn main() {
    macroquad::Window::from_config(window_conf(), game_main());
}

async fn game_main() {
    env_logger::init();

    loop {
        match run_session().await {
            GameLoopControl::Exit => {
                set_cursor_grab(false);
                show_mouse(true);
                break;
            }
            GameLoopControl::Restart => {
                set_cursor_grab(false);
                show_mouse(true);
                continue;
            }
        }
    }
}

// ── Game ─────────────────────────────────────────────────────────────────────

async fn run_session() -> GameLoopControl {
    let (server_addr, player_name) = match lobby_screen().await {
        Some(v) => v,
        None => return GameLoopControl::Exit,
    };
    let server_addr_str = server_addr.to_string();
    info!("Connecting to {} as '{}'", server_addr_str, player_name);

    let mut socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind socket");
    socket
        .send_packet(
            Packet::Connect {
                player_name: player_name.clone(),
            },
            &server_addr,
        )
        .expect("Send Connect failed");

    // Connecting screen — 5-second timeout
    let connect_start = Instant::now();
    let connect_result = loop {
        if connect_start.elapsed().as_secs() >= 5 {
            break None;
        }
        match socket.recv_packet() {
            Ok(Some((
                _hdr,
                Packet::Accept {
                    player_id,
                    seed,
                    difficulty,
                },
                _src,
            ))) => {
                info!("Connected! id={} seed={}", player_id, seed);
                break Some((player_id, seed, difficulty));
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                error!("recv: {}", e);
                break None;
            }
        }
        {
            let sw = screen_width();
            let sh = screen_height();
            let ccx = sw * 0.5;
            let ccy = sh * 0.5;
            let bp2 = 30.0_f32;
            let elapsed = connect_start.elapsed().as_secs_f32();
            // Sky
            draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.42, 0.72, 1.0, 1.0));
            // Grass
            draw_rectangle(0.0, sh * 0.66, sw, bp2, Color::new(0.37, 0.68, 0.27, 1.0));
            // Dirt
            draw_rectangle(
                0.0,
                sh * 0.66 + bp2,
                sw,
                sh,
                Color::new(0.48, 0.32, 0.18, 1.0),
            );
            // Title
            let conn_title = "MINE  MAZE";
            let cts = (sw * 0.10).clamp(44.0, 80.0);
            let ctw = measure_text(conn_title, None, cts as u16, 1.0).width;
            draw_text(
                conn_title,
                ccx - ctw * 0.5 + 4.0,
                ccy - 60.0 + 4.0,
                cts,
                Color::new(0.38, 0.28, 0.0, 0.90),
            );
            draw_text(
                conn_title,
                ccx - ctw * 0.5,
                ccy - 60.0,
                cts,
                Color::new(1.0, 0.88, 0.0, 1.0),
            );
            // Connecting text box
            let dots = ".".repeat(((elapsed * 2.0) as usize % 4) + 1);
            draw_rectangle(
                ccx - 220.0,
                ccy - 8.0,
                440.0,
                96.0,
                Color::new(0.12, 0.12, 0.15, 0.88),
            );
            draw_rectangle_lines(
                ccx - 222.0,
                ccy - 10.0,
                444.0,
                100.0,
                2.0,
                Color::new(0.50, 0.50, 0.55, 1.0),
            );
            let conn_str = format!("Connecting to {}{}", server_addr_str, dots);
            let csw = measure_text(&conn_str, None, 20, 1.0).width;
            draw_text(
                &conn_str,
                ccx - csw * 0.5,
                ccy + 16.0,
                20.0,
                Color::new(0.85, 0.85, 0.85, 1.0),
            );
            let pstr = format!("Player: {}", player_name);
            let psw = measure_text(&pstr, None, 18, 1.0).width;
            draw_text(
                &pstr,
                ccx - psw * 0.5,
                ccy + 44.0,
                18.0,
                Color::new(0.68, 0.68, 0.72, 1.0),
            );
            let remain = 5.0 - elapsed;
            let hint = format!("Timing out in {:.0}s  (Esc to cancel)", remain.ceil());
            let hw = measure_text(&hint, None, 15, 1.0).width;
            draw_text(
                &hint,
                ccx - hw * 0.5,
                ccy + 70.0,
                15.0,
                Color::new(0.55, 0.55, 0.60, 1.0),
            );
        }
        if is_key_pressed(KeyCode::Escape) {
            return GameLoopControl::Restart;
        }
        next_frame().await;
    };

    // Show "Connection failed" if timed out
    let (my_player_id, seed, difficulty) = match connect_result {
        Some(v) => v,
        None => loop {
            let sw = screen_width();
            let sh = screen_height();
            let ccx = sw * 0.5;
            let ccy = sh * 0.5;
            let bp2 = 30.0_f32;
            draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.42, 0.72, 1.0, 1.0));
            draw_rectangle(0.0, sh * 0.66, sw, bp2, Color::new(0.37, 0.68, 0.27, 1.0));
            draw_rectangle(
                0.0,
                sh * 0.66 + bp2,
                sw,
                sh,
                Color::new(0.48, 0.32, 0.18, 1.0),
            );
            let conn_title = "MINE  MAZE";
            let cts = (sw * 0.10).clamp(44.0, 80.0);
            let ctw = measure_text(conn_title, None, cts as u16, 1.0).width;
            draw_text(
                conn_title,
                ccx - ctw * 0.5 + 4.0,
                ccy - 60.0 + 4.0,
                cts,
                Color::new(0.38, 0.28, 0.0, 0.90),
            );
            draw_text(
                conn_title,
                ccx - ctw * 0.5,
                ccy - 60.0,
                cts,
                Color::new(1.0, 0.88, 0.0, 1.0),
            );
            draw_rectangle(
                ccx - 240.0,
                ccy - 20.0,
                480.0,
                120.0,
                Color::new(0.18, 0.06, 0.06, 0.95),
            );
            draw_rectangle_lines(
                ccx - 242.0,
                ccy - 22.0,
                484.0,
                124.0,
                2.5,
                Color::new(0.80, 0.20, 0.20, 1.0),
            );
            let e1 = "Connection failed!";
            let e1w = measure_text(e1, None, 26, 1.0).width;
            draw_text(
                e1,
                ccx - e1w * 0.5,
                ccy + 10.0,
                26.0,
                Color::new(1.0, 0.30, 0.30, 1.0),
            );
            let e2 = format!("Could not reach {}.", server_addr_str);
            let e2w = measure_text(&e2, None, 18, 1.0).width;
            draw_text(
                &e2,
                ccx - e2w * 0.5,
                ccy + 38.0,
                18.0,
                Color::new(0.80, 0.80, 0.80, 1.0),
            );
            let e3 = "Make sure the server is running, then try again.";
            let e3w = measure_text(e3, None, 15, 1.0).width;
            draw_text(
                e3,
                ccx - e3w * 0.5,
                ccy + 60.0,
                15.0,
                Color::new(0.60, 0.60, 0.65, 1.0),
            );
            let e4 = "Press Esc or Enter to return to lobby";
            let e4w = measure_text(e4, None, 15, 1.0).width;
            draw_text(
                e4,
                ccx - e4w * 0.5,
                ccy + 84.0,
                15.0,
                Color::new(0.45, 0.45, 0.50, 1.0),
            );
            next_frame().await;
            if is_key_pressed(KeyCode::Enter) {
                return GameLoopControl::Restart;
            }
            if is_key_pressed(KeyCode::Escape) {
                return GameLoopControl::Exit;
            }
        },
    };

    let map = Map::generate(seed, difficulty);
    let mut prediction = PredictionLocal::new();
    // (0,0) is always a border wall — initialize at a real floor tile so
    // collision checks don't block movement before the first server snapshot.
    let (sx, sy) = map.spawn_points.first().copied().unwrap_or((1.5, 1.5));
    prediction.local_state.x = sx;
    prediction.local_state.y = sy;
    prediction.map = Some(map);
    let mut interpolation = InterpolationRemote::new(100);
    let mut renderer = Renderer::new();

    let mut messages: Vec<String> = vec![format!(
        "Welcome {}!  WASD=move  Mouse=aim  Click=shoot  Tab=settings  Q=quit",
        player_name
    )];

    let mut sensitivity: f32 = 0.15;
    let mut settings_open = false;
    let mut game_over = false;
    let mut was_game_over = false;
    let mut is_winner = false;
    let mut input_sequence: u32 = 0;
    let mut last_snapshot_tick: u32 = 0;
    let mut bullet_marks: Vec<BulletMark> = Vec::new();
    let mut player_last_shot = Instant::now();
    let mut settings_selector: SettingsSelector = SettingsSelector::Dashboard;

    set_cursor_grab(true);
    show_mouse(false);

    loop {
        // ── settings toggle ──
        if settings_open {
            if is_key_pressed(KeyCode::Tab) || is_key_pressed(KeyCode::Escape) {
                settings_open = false;
                set_cursor_grab(true);
                show_mouse(false);
            }
        } else if game_over {
            // Release cursor when game over state changes
            if !was_game_over {
                set_cursor_grab(false);
                show_mouse(true);
            }
            if is_key_pressed(KeyCode::R) {
                let _ = socket.send_packet(Packet::Respawn, &server_addr);
            }
            if is_key_pressed(KeyCode::Q) || is_key_pressed(KeyCode::Escape) {
                // Send disconnect packet to server before returning to lobby
                let _ = socket.send_packet(Packet::Disconnect, &server_addr);
                return GameLoopControl::Restart;
            }
        } else {
            if is_key_pressed(KeyCode::Q) || is_key_pressed(KeyCode::Escape) {
                // Send disconnect packet to server before exiting
                let _ = socket.send_packet(Packet::Disconnect, &server_addr);
                return GameLoopControl::Exit;
            }
            if is_key_pressed(KeyCode::Tab)
                || is_key_pressed(KeyCode::LeftAlt)
                || is_key_pressed(KeyCode::RightAlt)
            {
                settings_open = true;
                set_cursor_grab(false);
                show_mouse(true);
            }
        }

        // ── input (skipped while settings open or game over) ──
        let mut input = shared::protocol::InputData::default();
        if !settings_open && !game_over {
            let mdx = mouse_delta_position().x;

            if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
                input.forward = true;
            }
            if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
                input.backward = true;
            }
            if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
                input.strafe_left = true;
            }
            if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
                input.strafe_right = true;
            }
            if is_mouse_button_pressed(MouseButton::Left) {
                input.shoot = true;
            }
            // mouse_delta_position returns a 0..1 normalized fraction of screen width.
            // Multiply by 2π so a full screen-width swipe = 360° at sensitivity 1.0.
            input.turn_delta = -mdx * sensitivity * std::f32::consts::TAU;

            let has_input = input.forward
                || input.backward
                || input.strafe_left
                || input.strafe_right
                || input.shoot
                || input.turn_delta.abs() > 0.0001;

            if has_input {
                input_sequence += 1;
                let _ = socket.send_packet(
                    Packet::Input {
                        input,
                        input_sequence,
                    },
                    &server_addr,
                );
                prediction.add_input(input_sequence, input);
            }

            if input.shoot && player_last_shot.elapsed() >= Duration::from_secs_f32(0.49) {
                player_last_shot = Instant::now();
                if let Some(map_ref) = prediction.map.as_ref() {
                    let angle = prediction.local_state.angle;
                    if let Some((cx, cy, hys, wf, step_x, step_y)) = cast_ray_hit(
                        prediction.local_state.x,
                        prediction.local_state.y,
                        angle,
                        map_ref,
                    ) {
                        if bullet_marks.len() >= MAX_BULLET_MARKS {
                            bullet_marks.remove(0);
                        }
                        bullet_marks.push(BulletMark {
                            cell_x: cx,
                            cell_y: cy,
                            hit_y_side: hys,
                            wall_frac: wf,
                            step_x,
                            step_y,
                        });
                    }
                }
            } else {
                input.shoot = false;
            }
        }

        // ── network receive ──
        loop {
            match socket.recv_packet() {
                Ok(Some((hdr, Packet::StateSnapshot { tick, players }, _))) => {
                    if tick > last_snapshot_tick {
                        last_snapshot_tick = tick;
                        if let Some(ls) = players.iter().find(|p| p.id == my_player_id) {
                            // Check if player is the only one alive (winner)
                            let alive_count = players.iter().filter(|p| !p.is_game_over).count();
                            is_winner = !ls.is_game_over && alive_count == 1 && players.len() > 1;
                            // Both winners and eliminated players see overlays
                            game_over = ls.is_game_over || is_winner;
                            prediction.reconcile(ls, tick, hdr.ack);
                        }
                        interpolation.push_snapshot(tick, players, my_player_id);
                    }
                }
                Ok(Some((
                    _,
                    Packet::LevelChange {
                        seed,
                        difficulty,
                        level,
                    },
                    _,
                ))) => {
                    info!("Level {} seed={}", level, seed);
                    prediction.map = Some(Map::generate(seed, difficulty));
                    messages.push(format!("=== LEVEL {} - {:?} ===", level, difficulty));
                    bullet_marks.clear();
                    is_winner = false; // Reset winner state on new level
                }
                Ok(Some((_, Packet::ServerMessage { text }, _))) => {
                    messages.push(text);
                }
                Ok(Some((_, Packet::GameOver, _))) => {
                    messages.push("=== GAME OVER - Returning to lobby ===".to_string());
                    // Return to lobby after a brief delay
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    return GameLoopControl::Restart;
                }
                Ok(Some((_, Packet::Pong, _))) => {}
                Ok(Some(_)) | Ok(None) => break,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    error!("recv: {}", e);
                    break;
                }
            }
        }

        while messages.len() > 8 {
            messages.remove(0);
        }

        // ── render ──
        let mut remote_states: Vec<PlayerState> = interpolation.get_interpolated_state();
        let map_ref = prediction.map.as_ref().unwrap();

        renderer.render(
            &prediction.local_state,
            &remote_states,
            map_ref,
            &messages,
            input.shoot,
            &bullet_marks,
        );

        if settings_open {
            // Include the local player in the dashboard list (remote_states only contains others).
            let mut display_states = remote_states.clone();
            display_states.push(prediction.local_state.clone());
            // Ensure same ordering: highest frags first, tiebreaker by lowest id.
            display_states.sort_by_key(|p| (std::cmp::Reverse(p.frags), p.id));
            draw_dashboard_and_settings_overlay(
                &mut sensitivity,
                &mut settings_selector,
                &display_states,
                my_player_id,
            );
        }

        if game_over {
            if is_winner {
                draw_winner_overlay();
            } else {
                draw_game_over_overlay();
            }
        }

        was_game_over = game_over;
        next_frame().await;
    }

    set_cursor_grab(false);
    show_mouse(true);
    info!("Disconnecting");
    let _ = socket.send_packet(Packet::Disconnect, &server_addr);
    GameLoopControl::Exit
}

// ── Lobby ─────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Field {
    Ip,
    Name,
}

async fn lobby_screen() -> Option<(std::net::SocketAddr, String)> {
    let default_ip = format!("127.0.0.1:{}", DEFAULT_SERVER_PORT);
    let mut ip = default_ip.clone();
    let mut name = String::new();
    let mut focused = Field::Name;
    let mut anim: f32 = 0.0;
    let mut show_name_error: bool = false;
    let mut show_ip_error: bool = false;
    let mut error_timer: f32 = 0.0;
    let mut backspace_init: f32 = 0.4;
    let mut backspace_held: f32 = 0.0;
    let mut backspace_hold = false;
    let mut delete_text = false;

    // Clear any buffered input from previous screens
    while let Some(_) = get_char_pressed() {}

    loop {
        anim += get_frame_time();

        while let Some(c) = get_char_pressed() {
            if c.is_control() {
                continue;
            }
            match focused {
                Field::Ip => {
                    ip.push(c);
                    show_ip_error = false; // Reset error when user starts typing
                }
                Field::Name => {
                    name.push(c);
                    show_name_error = false;
                }
            }
        }

        // Update error timer
        if show_name_error || show_ip_error {
            error_timer += get_frame_time();
            if error_timer > 3.0 {
                // Show error for 3 seconds
                show_name_error = false;
                show_ip_error = false;
                error_timer = 0.0;
            }
        }

        if is_key_down(KeyCode::Backspace) {
            if backspace_hold == true {
                backspace_held += get_frame_time();
                if backspace_held <= 0.1 {
                    delete_text = false;
                } else {
                    delete_text = true;
                    backspace_held = 0.0;
                }
            } else {
                if backspace_init == 0.4 {
                    delete_text = true;
                }
                backspace_init -= get_frame_time();
                if backspace_init <= 0.0 {
                    backspace_hold = true;
                    delete_text = true;
                }
            }
            
        } else {
            backspace_hold = false;
            backspace_init = 0.4;
        }

        if delete_text {
            match focused {
                Field::Ip => {
                    ip.pop();
                    show_ip_error = false; // Reset error when user deletes characters
                }
                Field::Name => {
                    name.pop();
                    show_name_error = false;
                }
            }
            delete_text = false;
        }
        if is_key_pressed(KeyCode::Tab) {
            focused = if focused == Field::Ip {
                Field::Name
            } else {
                Field::Ip
            };
        }
        if is_key_pressed(KeyCode::Escape) {
            return None;
        }

        let w = screen_width();
        let h = screen_height();
        let cx = w * 0.5;

        // ── SKY ──────────────────────────────────────────────────────────────
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.42, 0.72, 1.0, 1.0));
        // lighter horizon strip
        draw_rectangle(0.0, h * 0.45, w, h * 0.22, Color::new(0.58, 0.82, 1.0, 1.0));

        // ── ANIMATED BLOCKY CLOUDS ────────────────────────────────────────────
        let cloud_data: [(f32, f32, f32, f32); 5] = [
            (0.08, 0.07, 170.0, 28.0),
            (0.36, 0.12, 230.0, 24.0),
            (0.62, 0.06, 195.0, 32.0),
            (0.80, 0.16, 150.0, 22.0),
            (0.50, 0.20, 120.0, 18.0),
        ];
        for (ox, oy_f, cw, ch) in cloud_data {
            let cx_c = ((ox * w + anim * 16.0) % (w + 300.0)) - 150.0;
            let cy_c = oy_f * h;
            // Blocky cloud silhouette
            draw_rectangle(cx_c, cy_c + ch * 0.40, cw, ch * 0.60, WHITE);
            draw_rectangle(cx_c + cw * 0.18, cy_c, cw * 0.64, ch, WHITE);
            draw_rectangle(
                cx_c + cw * 0.05,
                cy_c + ch * 0.18,
                cw * 0.30,
                ch * 0.60,
                WHITE,
            );
            draw_rectangle(
                cx_c + cw * 0.66,
                cy_c + ch * 0.18,
                cw * 0.30,
                ch * 0.60,
                WHITE,
            );
        }

        // ── GROUND LAYERS ─────────────────────────────────────────────────────
        let ground_y = h * 0.66;
        let bp = 30.0_f32; // block pixel size

        // Grass layer (top-face lighter, side darker)
        draw_rectangle(0.0, ground_y, w, bp, Color::new(0.37, 0.68, 0.27, 1.0));
        let n_grass = (w / bp) as i32 + 2;
        for ci in 0..n_grass {
            let bx = ci as f32 * bp;
            draw_rectangle(
                bx + 1.0,
                ground_y,
                bp - 2.0,
                bp * 0.28,
                Color::new(0.46, 0.78, 0.34, 1.0),
            );
            draw_line(
                bx,
                ground_y,
                bx,
                ground_y + bp,
                1.0,
                Color::new(0.0, 0.0, 0.0, 0.12),
            );
        }

        // Dirt rows beneath grass
        let n_rows = ((h - ground_y - bp) / bp) as i32 + 2;
        for row in 0..n_rows {
            let dy = ground_y + bp + row as f32 * bp;
            let base_col = if row % 2 == 0 {
                Color::new(0.50, 0.34, 0.19, 1.0)
            } else {
                Color::new(0.44, 0.29, 0.15, 1.0)
            };
            draw_rectangle(0.0, dy, w, bp.min(h - dy), base_col);
            let offset = if row % 2 == 0 { 0.0 } else { bp * 0.5 };
            let n_cols = (w / bp) as i32 + 2;
            for col in 0..n_cols {
                let bx = col as f32 * bp - offset;
                if ((row * 7 + col * 11) % 5) < 2 {
                    draw_rectangle(
                        bx + bp * 0.25,
                        dy + bp * 0.30,
                        bp * 0.18,
                        bp * 0.18,
                        Color::new(0.36, 0.24, 0.11, 1.0),
                    );
                }
                draw_line(
                    bx + bp,
                    dy,
                    bx + bp,
                    (dy + bp).min(h),
                    1.0,
                    Color::new(0.0, 0.0, 0.0, 0.12),
                );
            }
        }

        // ── DECORATIVE BLOCKS ON GROUND LINE ─────────────────────────────────
        let dec_blocks: [(f32, f32); 6] = [
            (0.05, ground_y - bp),
            (0.12, ground_y - bp),
            (0.88, ground_y - bp),
            (0.95, ground_y - bp),
            (0.02, ground_y - bp * 2.0),
            (0.97, ground_y - bp * 2.0),
        ];
        let block_colors = [
            Color::new(0.37, 0.68, 0.27, 1.0), // grass
            Color::new(0.48, 0.32, 0.18, 1.0), // dirt
            Color::new(0.50, 0.50, 0.54, 1.0), // cobblestone
            Color::new(0.62, 0.26, 0.23, 1.0), // brick
            Color::new(0.37, 0.68, 0.27, 1.0), // grass
            Color::new(0.75, 0.68, 0.45, 1.0), // sandstone
        ];
        for (i, (xf, by)) in dec_blocks.iter().enumerate() {
            let bx = xf * w;
            draw_rectangle(bx, *by, bp, bp, block_colors[i]);
            draw_rectangle_lines(bx, *by, bp, bp, 1.0, Color::new(0.0, 0.0, 0.0, 0.30));
        }

        // ── TITLE "MINE MAZE" (gold, blocky shadow) ────────────────────────────
        let title = "MINE MAZE";
        let pw = (w * 0.44).clamp(300.0, 440.0); //panel width used later
        let ts = (pw * 0.22).clamp(52.0, 90.0);
        let m = measure_text(title, None, ts as u16, 1.0);
        let tw = m.width;
        let th = m.height;
        let tx = cx - tw * 0.5;
        let ty = h * 0.264;

        // Backdrop block
        draw_rectangle(
            tx - 20.0,
            ty - th - 20.0,
            tw + 40.0,
            th + 34.0,
            Color::new(0.0, 0.0, 0.0, 0.52),
        );
        draw_rectangle_lines(
            tx - 23.0,
            ty - th - 23.0,
            tw + 46.0,
            th + 40.0,
            3.0,
            Color::new(0.58, 0.46, 0.0, 0.88),
        );
        draw_rectangle_lines(
            tx - 20.75,
            ty - th - 20.75,
            tw + 41.5,
            th + 35.5,
            1.5,
            Color::new(0.28, 0.22, 0.0, 0.60),
        );

        // Dark gold shadow
        draw_text(
            title,
            tx + 5.0,
            ty + 5.0,
            ts,
            Color::new(0.40, 0.28, 0.0, 0.92),
        );
        // Bright gold main text
        draw_text(title, tx, ty, ts, Color::new(1.0, 0.88, 0.0, 1.0));

        // Subtitle (Minecraft edition style)
        let sub = "Multiplayer Minecraft-Style Maze Shooter";
        let ss = (w * 0.025).clamp(13.0, 18.0);
        let sm = measure_text(sub, None, ss as u16, 1.0);
        let sw2 = sm.width;
        let sh2 = sm.height;
        draw_text(
            sub,
            cx - sw2 * 0.5 + 2.0,
            ty + 17.0 + sh2,
            ss,
            Color::new(0.0, 0.0, 0.0, 0.55),
        );
        draw_text(
            sub,
            cx - sw2 * 0.5,
            ty + 15.0 + sh2,
            ss,
            Color::new(0.85, 0.85, 0.85, 1.0),
        );

        // ── INPUT PANEL (dark stone GUI) ──────────────────────────────────────

        //let pw = (w * 0.44).clamp(300.0, 440.0); used up
        let ph = 244.0_f32;
        let ppx = cx - pw * 0.5;
        let ppy = ty + 38.0;

        // Stone tile background
        draw_rectangle(ppx, ppy, pw, ph, Color::new(0.20, 0.20, 0.23, 0.96));
        let bsz = 20.0_f32; //panel block size
        let p_rows = (ph / bsz) as i32 + 1;
        let p_cols = (pw / bsz) as i32 + 1;
        for pr in 0..p_rows {
            for pc in 0..p_cols {
                if (pr + pc) % 2 == 0 {
                    let bx2 = ppx + pc as f32 * bsz;
                    let by2 = ppy + pr as f32 * bsz;
                    let rw2 = bsz.min(ppx + pw - bx2);
                    let rh2 = bsz.min(ppy + ph - by2);
                    if rw2 > 0.0 && rh2 > 0.0 {
                        draw_rectangle(bx2, by2, rw2, rh2, Color::new(0.23, 0.23, 0.27, 1.0));
                    }
                }
            }
        }
        // Minecraft GUI double-border
        draw_rectangle_lines(
            ppx - 3.0,
            ppy - 3.0,
            pw + 6.0,
            ph + 6.0,
            2.5,
            Color::new(0.60, 0.60, 0.65, 1.0),
        );
        draw_rectangle_lines(
            ppx - 1.0,
            ppy - 1.0,
            pw + 2.0,
            ph + 2.0,
            1.5,
            Color::new(0.08, 0.08, 0.10, 1.0),
        );

        // Fields
        let field_h = 36.0;
        let field_w = pw - 32.0;
        let field_x = ppx + 16.0;
        let ip_y = ppy + 46.0;
        let name_y = ppy + 128.0;

        let (mx, my) = mouse_position();
        let clicked = is_mouse_button_pressed(MouseButton::Left);

        if clicked && mx >= field_x && mx <= field_x + field_w {
            if my >= ip_y && my <= ip_y + field_h {
                focused = Field::Ip;
            }
            if my >= name_y && my <= name_y + field_h {
                focused = Field::Name;
            }
        }

        let btn_w = field_w;
        let btn_h = 42.0_f32;
        let btn_x = field_x;
        let btn_y = ppy + ph - btn_h - 14.0;

        let start_hovered =
            mx >= btn_x && mx <= btn_x + btn_w && my >= btn_y && my <= btn_y + btn_h;
        let start = (clicked && start_hovered) || is_key_pressed(KeyCode::Enter);

        let lbl_p = 10.0;
        mc_label("Server IP", field_x, ip_y - lbl_p);
        mc_field(&ip, field_x, ip_y, field_w, field_h, focused == Field::Ip);
        mc_label("Player Name", field_x, name_y - lbl_p);
        mc_field(
            &name,
            field_x,
            name_y,
            field_w,
            field_h,
            focused == Field::Name,
        );

        // Minecraft grass/green button
        let (btn_top, btn_bot) = if start_hovered {
            (
                Color::new(0.56, 0.84, 0.34, 1.0),
                Color::new(0.38, 0.64, 0.20, 1.0),
            )
        } else {
            (
                Color::new(0.44, 0.72, 0.26, 1.0),
                Color::new(0.28, 0.52, 0.14, 1.0),
            )
        };
        draw_rectangle(btn_x, btn_y, btn_w, btn_h, btn_bot);
        draw_rectangle(btn_x, btn_y, btn_w, btn_h * 0.48, btn_top);
        // Inner highlight + dark border
        draw_rectangle_lines(
            btn_x,
            btn_y,
            btn_w,
            btn_h,
            2.0,
            Color::new(0.0, 0.0, 0.0, 0.60),
        );
        draw_rectangle_lines(
            btn_x + 2.0,
            btn_y + 2.0,
            btn_w - 4.0,
            btn_h - 4.0,
            1.0,
            Color::new(1.0, 1.0, 1.0, 0.20),
        );
        let lbl = "PLAY GAME";
        let lw = measure_text(lbl, None, 24, 1.0).width;
        draw_text(
            lbl,
            btn_x + (btn_w - lw) * 0.5 + 2.0,
            btn_y + btn_h * 0.65 + 2.0,
            24.0,
            Color::new(0.0, 0.0, 0.0, 0.60),
        );
        draw_text(
            lbl,
            btn_x + (btn_w - lw) * 0.5,
            btn_y + btn_h * 0.65,
            24.0,
            WHITE,
        );

        // Error message
        if show_name_error || show_ip_error {
            let error_alpha = if error_timer < 0.5 {
                error_timer * 2.0 // Fade in
            } else if error_timer > 2.5 {
                (3.0 - error_timer) * 2.0 // Fade out
            } else {
                1.0 // Full opacity
            };

            let error_msg = if show_ip_error {
                "Please enter a valid IP!"
            } else {
                if name.trim().len() > 0 {
                    "Name must be more than 2 characters!"
                } else {
                    "Please enter your name to continue!"
                }
            };
            let error_w = measure_text(error_msg, None, 18, 1.0).width;
            let error_x = cx - error_w * 0.5;
            let error_y = name_y + field_h + 15.0;

            // Error background
            draw_rectangle(
                error_x - 8.0,
                error_y - 2.0,
                error_w + 16.0,
                24.0,
                Color::new(0.8, 0.2, 0.2, error_alpha * 0.9),
            );
            draw_rectangle_lines(
                error_x - 8.0,
                error_y - 2.0,
                error_w + 16.0,
                24.0,
                1.5,
                Color::new(1.0, 0.4, 0.4, error_alpha),
            );

            // Error text
            draw_text(
                error_msg,
                error_x,
                error_y + 16.0,
                18.0,
                Color::new(1.0, 1.0, 1.0, error_alpha),
            );
        }

        // Footer hint
        draw_text(
            "Tab = switch field   Enter = play   Esc = quit",
            10.0,
            h - 10.0,
            14.0,
            Color::new(0.88, 0.88, 0.92, 1.0),
        );

        next_frame().await;

        if start {
            //IP
            let rip = if ip.trim().is_empty() {
                default_ip.clone()
            } else {
                ip.trim().to_string()
            };
            let addr_str = if rip.contains(':') {
                rip.to_string()
            } else {
                format!("{}:{}", rip, DEFAULT_SERVER_PORT)
            };

            let server_addr: std::net::SocketAddr = match addr_str.parse() {
                Ok(a) => a,
                Err(e) => {
                    show_ip_error = true;
                    error_timer = 0.0;
                    focused = Field::Ip; // Focus back on name field
                    continue;
                }
            };

            //Name
            if name.trim().len() < 3 {
                show_name_error = true;
                error_timer = 0.0;
                focused = Field::Name; // Focus back on name field
                continue;
            }
            let rname = name.trim().to_string();
            return Some((server_addr, rname));
        }
    }
}

fn mc_label(text: &str, x: f32, y: f32) {
    // Shadow then main
    draw_text(
        text,
        x + 1.5,
        y + 1.5,
        16.0,
        Color::new(0.0, 0.0, 0.0, 0.55),
    );
    draw_text(text, x, y, 16.0, Color::new(0.85, 0.85, 0.80, 1.0));
}

fn mc_field(value: &str, x: f32, y: f32, w: f32, h: f32, focused: bool) {
    // Outer dark border, inner black bg
    draw_rectangle(x, y, w, h, Color::new(0.0, 0.0, 0.0, 0.85));
    draw_rectangle(
        x + 2.0,
        y + 2.0,
        w - 4.0,
        h - 4.0,
        Color::new(0.08, 0.08, 0.10, 1.0),
    );
    draw_rectangle_lines(
        x,
        y,
        w,
        h,
        if focused { 2.5 } else { 1.5 },
        if focused {
            Color::new(1.0, 0.88, 0.0, 1.0)
        } else {
            Color::new(0.38, 0.38, 0.45, 1.0)
        },
    );
    let display = if focused && (get_time() * 2.0) as u32 % 2 == 0 {
        format!("{}_", value)
    } else {
        value.to_string()
    };
    draw_text(&display, x + 10.0, y + h * 0.67, 20.0, WHITE);
}

// ── Settings overlay ──────────────────────────────────────────────────────────

fn draw_game_over_overlay() {
    let sw = screen_width();
    let sh = screen_height();

    // Dim background
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.70));

    let pw = 440.0_f32;
    let ph = 260.0_f32;
    let px = (sw - pw) * 0.5;
    let py = (sh - ph) * 0.5;

    draw_rectangle(px, py, pw, ph, Color::new(0.25, 0.10, 0.10, 0.97));
    draw_rectangle_lines(px, py, pw, ph, 2.0, Color::new(0.80, 0.20, 0.20, 1.0));

    // Title
    let title = "GAME OVER";
    let tm = measure_text(title, None, 48, 1.0);
    draw_text(
        title,
        px + (pw - tm.width) * 0.5,
        py + 56.0,
        48.0,
        Color::new(1.0, 0.30, 0.30, 1.0),
    );
    draw_line(
        px + 20.0,
        py + 68.0,
        px + pw - 20.0,
        py + 68.0,
        1.0,
        Color::new(0.50, 0.20, 0.20, 1.0),
    );

    // Instructions
    let instr1 = "You have been eliminated!";
    let i1w = measure_text(instr1, None, 20, 1.0).width;
    draw_text(
        instr1,
        px + (pw - i1w) * 0.5,
        py + 110.0,
        20.0,
        Color::new(0.85, 0.85, 0.85, 1.0),
    );

    let instr3 = "Press Q or Esc to quit";
    let i3w = measure_text(instr3, None, 16, 1.0).width;
    draw_text(
        instr3,
        px + (pw - i3w) * 0.5,
        py + 190.0,
        16.0,
        Color::new(0.60, 0.60, 0.65, 1.0),
    );
}

fn draw_winner_overlay() {
    let sw = screen_width();
    let sh = screen_height();

    // Dim background with golden tint
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.1, 0.08, 0.0, 0.70));

    let pw = 440.0_f32;
    let ph = 260.0_f32;
    let px = (sw - pw) * 0.5;
    let py = (sh - ph) * 0.5;

    draw_rectangle(px, py, pw, ph, Color::new(0.15, 0.12, 0.0, 0.97));
    draw_rectangle_lines(px, py, pw, ph, 2.0, Color::new(1.0, 0.88, 0.0, 1.0));

    // Title
    let title = "YOU WON!";
    let tm = measure_text(title, None, 48, 1.0);
    draw_text(
        title,
        px + (pw - tm.width) * 0.5,
        py + 56.0,
        48.0,
        Color::new(1.0, 0.88, 0.0, 1.0),
    );
    draw_line(
        px + 20.0,
        py + 68.0,
        px + pw - 20.0,
        py + 68.0,
        1.0,
        Color::new(0.8, 0.6, 0.0, 1.0),
    );

    // Instructions
    let instr1 = "You are the last one standing!";
    let i1w = measure_text(instr1, None, 20, 1.0).width;
    draw_text(
        instr1,
        px + (pw - i1w) * 0.5,
        py + 110.0,
        20.0,
        Color::new(0.85, 0.85, 0.85, 1.0),
    );

    let instr3 = "Press Q or Esc to quit";
    let i3w = measure_text(instr3, None, 16, 1.0).width;
    draw_text(
        instr3,
        px + (pw - i3w) * 0.5,
        py + 190.0,
        16.0,
        Color::new(0.60, 0.60, 0.65, 1.0),
    );
}

fn draw_dashboard_and_settings_overlay(
    sensitivity: &mut f32,
    settings_selector: &mut SettingsSelector,
    players: &[PlayerState],
    my_id: u32,
) {
    let sw = screen_width();
    let sh = screen_height();

    // Dim background
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.58));
    let pw = 460.0_f32;
    let ph = 270.0_f32;
    let px = (sw - pw) * 0.5;
    let py = (sh - ph) * 0.5;

    let selectorw = pw;
    let selectorh = ph / 4.0;
    let selectorx = px;
    let selectory = py - selectorh - 20.0;

    draw_rectangle(
        selectorx,
        selectory,
        selectorw,
        selectorh,
        Color::new(0.20, 0.15, 0.10, 0.97),
    );
    draw_rectangle_lines(
        selectorx,
        selectory,
        selectorw,
        selectorh,
        2.0,
        Color::new(0.42, 0.32, 0.24, 1.0),
    );
    //Dashboard/Settings select
    let dss1 = "DASHBOARD";
    let dss2 = "SETTINGS";

    let font_size = 28.0;
    let dss1m = measure_text(dss1, None, font_size as u16, 1.0);
    let dss2m = measure_text(dss2, None, font_size as u16, 1.0);
    let dash_x = selectorx + selectorw * 0.25 - dss1m.width * 0.5;
    let settings_x = selectorx + selectorw * 0.75 - dss2m.width * 0.5;

    let text_y = selectory + selectorh * 0.5 + dss1m.height * 0.35;

    let dash_color = match settings_selector {
        SettingsSelector::Dashboard => YELLOW,
        SettingsSelector::Settings => WHITE,
    };

    let settings_color = match settings_selector {
        SettingsSelector::Dashboard => WHITE,
        SettingsSelector::Settings => YELLOW,
    };

    draw_text(dss1, dash_x, text_y, font_size, dash_color);
    draw_text(dss2, settings_x, text_y, font_size, settings_color);

    // Underline DASHBOARD
    draw_line(
        dash_x,
        text_y + 6.0,
        dash_x + dss1m.width,
        text_y + 6.0,
        2.0,
        dash_color,
    );

    // Underline SETTINGS
    draw_line(
        settings_x,
        text_y + 6.0,
        settings_x + dss2m.width,
        text_y + 6.0,
        2.0,
        settings_color,
    );

    // Center divider
    draw_line(
        selectorx + selectorw / 2.0,
        selectory + 5.0,
        selectorx + selectorw / 2.0,
        selectory + selectorh - 5.0,
        1.0,
        Color::new(0.35, 0.28, 0.20, 1.0),
    );

    draw_rectangle(px, py, pw, ph, Color::new(0.20, 0.15, 0.10, 0.97));
    draw_rectangle_lines(px, py, pw, ph, 2.0, Color::new(0.42, 0.32, 0.24, 1.0));

    // Title
    // let title = "MINECRAFT SETTINGS";
    // let tm = measure_text(title, None, 34, 1.0);
    // draw_text(title, px + (pw - tm.width) * 0.5, py + 46.0, 34.0, WHITE);
    // draw_line(
    //     px + 20.0,
    //     py + 58.0,
    //     px + pw - 20.0,
    //     py + 58.0,
    //     1.0,
    //     Color::new(0.35, 0.28, 0.20, 1.0),
    // );

    // Mouse drag on slider & selector detect
    let (mx, my) = mouse_position();

    if is_mouse_button_down(MouseButton::Left)
        && mx >= selectorx
        && mx <= selectorx + selectorw / 2.0
        && my >= selectory
        && my <= selectory + selectorh
    {
        *settings_selector = SettingsSelector::Dashboard;
    }
    if is_mouse_button_down(MouseButton::Left)
        && mx >= selectorx + selectorw / 2.0
        && mx <= selectorx + selectorw
        && my >= selectory
        && my <= selectory + selectorh
    {
        *settings_selector = SettingsSelector::Settings;
    }

    if *settings_selector == SettingsSelector::Dashboard {
        let thead = "Top Players";
        let tdim = measure_text(thead, None, 20, 1.0);

        let startx = px + 20.0;
        let starty = py + 10.0;
        let entryw = pw - 40.0;

        // Header
        draw_text(
            thead,
            startx + entryw / 2.0 - tdim.width / 2.0,
            starty + tdim.height,
            20.0,
            WHITE,
        );

        // Line under header
        let mut y = starty + tdim.height + 10.0;
        draw_line(startx, y, startx + entryw, y, 1.0, WHITE);

        y += 10.0;

        for player in players.iter().take(7) {
            let row_h = 18.0;

            let name_text = decode_name(&player.name);
            let frag_text = &player.frags.to_string();

            let name_dim = measure_text(name_text, None, 18, 1.0);
            let frag_dim = measure_text(frag_text, None, 18, 1.0);

            let left_x = startx;
            let right_x = startx + entryw;

            let name_y = y + row_h;

            // Name (left)
            draw_text(name_text, left_x, name_y, 18.0, if my_id == player.id { YELLOW} else {WHITE});

            // Frags (right-aligned)
            draw_text(frag_text, right_x - frag_dim.width, name_y, 18.0, YELLOW);

            // Divider line
            y += row_h + 6.0;
            draw_line(
                startx,
                y,
                startx + entryw,
                y,
                0.8,
                Color::new(0.3, 0.3, 0.3, 1.0),
            );

            y += 6.0;
        }

        if players.len() > 7 {
            draw_text(&format!("...{} other players", players.len() - 7), startx, py + ph - 5.0, 15.0, GRAY);
        }
    }

    if *settings_selector == SettingsSelector::Settings {
        // Sensitivity label
        let label = format!("Mouse Sensitivity:  {:.2}", sensitivity);
        draw_text(&label, px + 28.0, py + 98.0, 20.0, LIGHTGRAY);

        // Slider track
        let slx = px + 28.0;
        let sly = py + 112.0;
        let slw = pw - 56.0;
        let slh = 26.0_f32;
        let t = *sensitivity;

        draw_rectangle(slx, sly, slw, slh, Color::new(0.16, 0.16, 0.20, 1.0));
        draw_rectangle(slx, sly, slw * t, slh, Color::new(0.18, 0.64, 0.34, 1.0));
        draw_rectangle_lines(slx, sly, slw, slh, 1.5, Color::new(0.30, 0.30, 0.38, 1.0));

        // Slider handle
        let hx = (slx + slw * t - 7.0).clamp(slx, slx + slw - 14.0);
        draw_rectangle(hx, sly - 4.0, 14.0, slh + 8.0, WHITE);

        // Labels: low / high
        draw_text("0.0", slx, sly + slh + 16.0, 14.0, DARKGRAY);
        let hi = "1.0";
        let hiw = measure_text(hi, None, 14, 1.0).width;
        draw_text(hi, slx + slw - hiw, sly + slh + 16.0, 14.0, DARKGRAY);
        if is_mouse_button_down(MouseButton::Left)
            && mx >= slx
            && mx <= slx + slw
            && my >= sly - 6.0
            && my <= sly + slh + 6.0
        {
            *sensitivity = ((mx - slx) / slw).clamp(0.0, 1.0);
        }

        // Keyboard fine-tune (Left / Right arrows)
        if is_key_pressed(KeyCode::Left) {
            *sensitivity = ((*sensitivity - 0.05) * 20.0).round() / 20.0;
            *sensitivity = sensitivity.max(0.0);
        }
        if is_key_pressed(KeyCode::Right) {
            *sensitivity = ((*sensitivity + 0.05) * 20.0).round() / 20.0;
            *sensitivity = sensitivity.min(1.0);
        }

        // Hints
        let hint1 = "Drag slider  or  Left / Right arrows to adjust";
        let h1w = measure_text(hint1, None, 14, 1.0).width;
        draw_text(
            hint1,
            px + (pw - h1w) * 0.5,
            sly + slh + 34.0,
            14.0,
            Color::new(0.38, 0.38, 0.42, 1.0),
        );

        let hint2 = "Tab / Esc  --  resume game";
        let h2w = measure_text(hint2, None, 16, 1.0).width;
        draw_text(
            hint2,
            px + (pw - h2w) * 0.5,
            py + ph - 16.0,
            16.0,
            Color::new(0.35, 0.35, 0.40, 1.0),
        );
    }
}
