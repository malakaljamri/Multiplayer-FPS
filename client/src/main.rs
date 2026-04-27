pub mod interpolation;
pub mod prediction;
pub mod renderer;

use log::{error, info};
use macroquad::prelude::*;

use shared::maze::Map;
use shared::network::UdpSocket;
use shared::physics::cast_ray_hit;
use shared::protocol::{Packet, DEFAULT_SERVER_PORT};

use interpolation::InterpolationRemote;
use prediction::PredictionLocal;
use renderer::{BulletMark, Renderer};

const MAX_BULLET_MARKS: usize = 64;

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

// ── Lobby ─────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Field { Ip, Name }

async fn lobby_screen() -> Option<(String, String)> {
    let default_ip = format!("127.0.0.1:{}", DEFAULT_SERVER_PORT);
    let mut ip = default_ip.clone();
    let mut name = String::new();
    let mut focused = Field::Name;

    loop {
        while let Some(c) = get_char_pressed() {
            if c.is_control() { continue; }
            match focused {
                Field::Ip   => ip.push(c),
                Field::Name => name.push(c),
            }
        }
        if is_key_pressed(KeyCode::Backspace) {
            match focused {
                Field::Ip   => { ip.pop(); }
                Field::Name => { name.pop(); }
            }
        }
        if is_key_pressed(KeyCode::Tab) {
            focused = if focused == Field::Ip { Field::Name } else { Field::Ip };
        }
        if is_key_pressed(KeyCode::Escape) { return None; }

        let w  = screen_width();
        let h  = screen_height();
        let cx = w * 0.5;
        let cy = h * 0.5;

        let field_w = 320.0_f32;
        let field_h = 38.0_f32;
        let field_x = cx - field_w * 0.5;
        let ip_y    = cy - 60.0;
        let name_y  = cy + 10.0;
        let btn_w   = 160.0_f32;
        let btn_h   = 44.0_f32;
        let btn_x   = cx - btn_w * 0.5;
        let btn_y   = cy + 90.0;

        let (mx, my) = mouse_position();
        let clicked  = is_mouse_button_pressed(MouseButton::Left);

        if clicked && mx >= field_x && mx <= field_x + field_w {
            if my >= ip_y && my <= ip_y + field_h       { focused = Field::Ip;   }
            else if my >= name_y && my <= name_y + field_h { focused = Field::Name; }
        }

        let start_hovered = mx >= btn_x && mx <= btn_x + btn_w
                         && my >= btn_y && my <= btn_y + btn_h;
        let start = (clicked && start_hovered) || is_key_pressed(KeyCode::Enter);

        clear_background(BLACK);

        let title = "MAZE WARS";
        let tw = measure_text(title, None, 56, 1.0).width;
        draw_text(title, cx - tw * 0.5, cy - 140.0, 56.0, WHITE);

        let sub = "Enter your details to join";
        let sw2 = measure_text(sub, None, 18, 1.0).width;
        draw_text(sub, cx - sw2 * 0.5, cy - 100.0, 18.0, DARKGRAY);

        lobby_label("Server IP",    field_x, ip_y   - 20.0);
        lobby_field(&ip,   field_x, ip_y,   field_w, field_h, focused == Field::Ip);
        lobby_label("Player Name",  field_x, name_y - 20.0);
        lobby_field(&name, field_x, name_y, field_w, field_h, focused == Field::Name);

        let btn_col = if start_hovered {
            Color::new(0.25, 0.75, 0.35, 1.0)
        } else {
            Color::new(0.15, 0.55, 0.25, 1.0)
        };
        draw_rectangle(btn_x, btn_y, btn_w, btn_h, btn_col);
        draw_rectangle_lines(btn_x, btn_y, btn_w, btn_h, 1.5, WHITE);
        let lbl = "START";
        let lw = measure_text(lbl, None, 26, 1.0).width;
        draw_text(lbl, btn_x + (btn_w - lw) * 0.5, btn_y + btn_h * 0.66, 26.0, WHITE);

        draw_text(
            "Tab = switch field   Enter = start   Esc = quit",
            10.0, h - 12.0, 15.0, Color::new(0.35, 0.35, 0.35, 1.0),
        );

        next_frame().await;

        if start {
            let rip   = if ip.trim().is_empty()   { default_ip.clone() } else { ip.trim().to_string() };
            let rname = if name.trim().is_empty()  { "Player".to_string() } else { name.trim().to_string() };
            return Some((rip, rname));
        }
    }
}

fn lobby_label(text: &str, x: f32, y: f32) {
    draw_text(text, x, y, 16.0, GRAY);
}

fn lobby_field(value: &str, x: f32, y: f32, w: f32, h: f32, focused: bool) {
    draw_rectangle(x, y, w, h, Color::new(0.08, 0.08, 0.10, 1.0));
    draw_rectangle_lines(
        x, y, w, h,
        if focused { 2.0 } else { 1.5 },
        if focused { WHITE } else { Color::new(0.35, 0.35, 0.40, 1.0) },
    );
    let display = if focused && (get_time() * 2.0) as u32 % 2 == 0 {
        format!("{}_", value)
    } else {
        value.to_string()
    };
    draw_text(&display, x + 10.0, y + h * 0.67, 20.0, WHITE);
}

// ── Settings overlay ──────────────────────────────────────────────────────────

fn draw_settings_overlay(sensitivity: &mut f32) {
    let sw = screen_width();
    let sh = screen_height();

    // Dim background
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.58));

    let pw = 460.0_f32;
    let ph = 270.0_f32;
    let px = (sw - pw) * 0.5;
    let py = (sh - ph) * 0.5;

    draw_rectangle(px, py, pw, ph, Color::new(0.09, 0.09, 0.12, 0.97));
    draw_rectangle_lines(px, py, pw, ph, 2.0, Color::new(0.38, 0.38, 0.44, 1.0));

    // Title
    let title = "SETTINGS";
    let tm = measure_text(title, None, 34, 1.0);
    draw_text(title, px + (pw - tm.width) * 0.5, py + 46.0, 34.0, WHITE);
    draw_line(px + 20.0, py + 58.0, px + pw - 20.0, py + 58.0, 1.0,
        Color::new(0.25, 0.25, 0.30, 1.0));

    // Sensitivity label
    let label = format!("Mouse Sensitivity:  {:.2}x", sensitivity);
    draw_text(&label, px + 28.0, py + 98.0, 20.0, LIGHTGRAY);

    // Slider track
    let slx = px + 28.0;
    let sly = py + 112.0;
    let slw = pw - 56.0;
    let slh = 26.0_f32;
    let t   = (*sensitivity - 0.1) / 4.9;

    draw_rectangle(slx, sly, slw, slh, Color::new(0.16, 0.16, 0.20, 1.0));
    draw_rectangle(slx, sly, slw * t, slh, Color::new(0.18, 0.64, 0.34, 1.0));
    draw_rectangle_lines(slx, sly, slw, slh, 1.5, Color::new(0.30, 0.30, 0.38, 1.0));

    // Slider handle
    let hx = (slx + slw * t - 7.0).clamp(slx, slx + slw - 14.0);
    draw_rectangle(hx, sly - 4.0, 14.0, slh + 8.0, WHITE);

    // Labels: low / high
    draw_text("0.1", slx, sly + slh + 16.0, 14.0, DARKGRAY);
    let hi = "5.0";
    let hiw = measure_text(hi, None, 14, 1.0).width;
    draw_text(hi, slx + slw - hiw, sly + slh + 16.0, 14.0, DARKGRAY);

    // Mouse drag on slider
    let (mx, my) = mouse_position();
    if is_mouse_button_down(MouseButton::Left)
        && mx >= slx && mx <= slx + slw
        && my >= sly - 6.0 && my <= sly + slh + 6.0
    {
        *sensitivity = (((mx - slx) / slw) * 4.9 + 0.1).clamp(0.1, 5.0);
    }

    // Keyboard fine-tune (Left / Right arrows)
    if is_key_pressed(KeyCode::Left) {
        *sensitivity = ((*sensitivity - 0.1) * 10.0).round() / 10.0;
        *sensitivity = sensitivity.max(0.1);
    }
    if is_key_pressed(KeyCode::Right) {
        *sensitivity = ((*sensitivity + 0.1) * 10.0).round() / 10.0;
        *sensitivity = sensitivity.min(5.0);
    }

    // Hints
    let hint1 = "Drag slider  or  Left / Right arrows to adjust";
    let h1w = measure_text(hint1, None, 14, 1.0).width;
    draw_text(hint1, px + (pw - h1w) * 0.5, sly + slh + 34.0, 14.0,
        Color::new(0.38, 0.38, 0.42, 1.0));

    let hint2 = "Tab / Esc  --  resume game";
    let h2w = measure_text(hint2, None, 16, 1.0).width;
    draw_text(hint2, px + (pw - h2w) * 0.5, py + ph - 16.0, 16.0,
        Color::new(0.35, 0.35, 0.40, 1.0));
}

// ── Game ─────────────────────────────────────────────────────────────────────

async fn game_main() {
    env_logger::init();

    let (server_addr_str, player_name) = match lobby_screen().await {
        Some(v) => v,
        None    => return,
    };

    info!("Connecting to {} as '{}'", server_addr_str, player_name);

    let server_addr: std::net::SocketAddr = if server_addr_str.contains(':') {
        match server_addr_str.parse() {
            Ok(a)  => a,
            Err(e) => { error!("Invalid address '{}': {}", server_addr_str, e); return; }
        }
    } else {
        format!("{}:{}", server_addr_str, DEFAULT_SERVER_PORT)
            .parse().expect("Invalid IP")
    };

    let mut socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind socket");
    socket.send_packet(Packet::Connect { player_name: player_name.clone() }, &server_addr)
        .expect("Send Connect failed");

    // Connecting screen
    let (my_player_id, seed, difficulty) = loop {
        match socket.recv_packet() {
            Ok(Some((_hdr, Packet::Accept { player_id, seed, difficulty }, _src))) => {
                info!("Connected! id={} seed={}", player_id, seed);
                break (player_id, seed, difficulty);
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => { error!("recv: {}", e); std::process::exit(1); }
        }
        clear_background(BLACK);
        let cx = screen_width() / 2.0;
        let cy = screen_height() / 2.0;
        draw_text("MAZE WARS", cx - 80.0, cy - 50.0, 48.0, WHITE);
        draw_text(&format!("Connecting to {}...", server_addr_str), cx - 150.0, cy,      22.0, GRAY);
        draw_text(&format!("Player: {}", player_name),              cx -  60.0, cy + 32.0, 20.0, DARKGRAY);
        next_frame().await;
    };

    let map = Map::generate(seed, difficulty);
    let mut prediction   = PredictionLocal::new();
    prediction.map       = Some(map);
    let mut interpolation = InterpolationRemote::new(100);
    let mut renderer     = Renderer::new();

    let mut messages: Vec<String> = vec![format!(
        "Welcome {}!  WASD=move  Mouse=aim  Click=shoot  Tab=settings  Q=quit",
        player_name
    )];

    let mut sensitivity: f32  = 1.0;
    let mut settings_open     = false;
    let mut input_sequence: u32      = 0;
    let mut last_snapshot_tick: u32  = 0;
    let mut bullet_marks: Vec<BulletMark> = Vec::new();

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
        } else {
            if is_key_pressed(KeyCode::Q) || is_key_pressed(KeyCode::Escape) {
                break;
            }
            if is_key_pressed(KeyCode::Tab) {
                settings_open = true;
                set_cursor_grab(false);
                show_mouse(true);
            }
        }

        // ── input (skipped while settings open) ──
        let mut input = shared::protocol::InputData::default();
        if !settings_open {
            let mdx = mouse_delta_position().x;

            if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up)    { input.forward      = true; }
            if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down)   { input.backward     = true; }
            if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left)   { input.strafe_left  = true; }
            if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right)  { input.strafe_right = true; }
            if is_mouse_button_pressed(MouseButton::Left)               { input.shoot        = true; }
            input.turn_delta = -mdx * sensitivity;

            let has_input = input.forward || input.backward || input.strafe_left
                || input.strafe_right || input.shoot || input.turn_delta.abs() > 0.0001;

            if has_input {
                input_sequence += 1;
                let _ = socket.send_packet(
                    Packet::Input { input, input_sequence },
                    &server_addr,
                );
                prediction.add_input(input_sequence, input);
            }

            if input.shoot {
                if let Some(map_ref) = prediction.map.as_ref() {
                    let angle = prediction.local_state.angle;
                    if let Some((cx, cy, hys, wf)) = cast_ray_hit(
                        prediction.local_state.x, prediction.local_state.y, angle, map_ref,
                    ) {
                        if bullet_marks.len() >= MAX_BULLET_MARKS { bullet_marks.remove(0); }
                        bullet_marks.push(BulletMark { cell_x: cx, cell_y: cy, hit_y_side: hys, wall_frac: wf });
                    }
                }
            }
        }

        // ── network receive ──
        loop {
            match socket.recv_packet() {
                Ok(Some((hdr, Packet::StateSnapshot { tick, players }, _))) => {
                    if tick > last_snapshot_tick {
                        last_snapshot_tick = tick;
                        if let Some(ls) = players.iter().find(|p| p.id == my_player_id) {
                            prediction.reconcile(ls, tick, hdr.ack);
                        }
                        interpolation.push_snapshot(tick, players, my_player_id);
                    }
                }
                Ok(Some((_, Packet::LevelChange { seed, difficulty, level }, _))) => {
                    info!("Level {} seed={}", level, seed);
                    prediction.map = Some(Map::generate(seed, difficulty));
                    messages.push(format!("=== LEVEL {} - {:?} ===", level, difficulty));
                    bullet_marks.clear();
                }
                Ok(Some((_, Packet::ServerMessage { text }, _))) => { messages.push(text); }
                Ok(Some((_, Packet::Pong, _))) => {}
                Ok(Some(_)) | Ok(None) => break,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => { error!("recv: {}", e); break; }
            }
        }

        while messages.len() > 8 { messages.remove(0); }

        // ── render ──
        let remote_states = interpolation.get_interpolated_state();
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
            draw_settings_overlay(&mut sensitivity);
        }

        next_frame().await;
    }

    set_cursor_grab(false);
    show_mouse(true);
    info!("Disconnecting");
    let _ = socket.send_packet(Packet::Disconnect, &server_addr);
}
