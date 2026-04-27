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
    let mut anim: f32 = 0.0;

    loop {
        anim += get_frame_time();

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
            draw_rectangle(cx_c,            cy_c + ch*0.40, cw,       ch*0.60, WHITE);
            draw_rectangle(cx_c + cw*0.18,  cy_c,           cw*0.64,  ch,      WHITE);
            draw_rectangle(cx_c + cw*0.05,  cy_c + ch*0.18, cw*0.30,  ch*0.60, WHITE);
            draw_rectangle(cx_c + cw*0.66,  cy_c + ch*0.18, cw*0.30,  ch*0.60, WHITE);
        }

        // ── GROUND LAYERS ─────────────────────────────────────────────────────
        let ground_y = h * 0.66;
        let bp = 30.0_f32; // block pixel size

        // Grass layer (top-face lighter, side darker)
        draw_rectangle(0.0, ground_y, w, bp, Color::new(0.37, 0.68, 0.27, 1.0));
        let n_grass = (w / bp) as i32 + 2;
        for ci in 0..n_grass {
            let bx = ci as f32 * bp;
            draw_rectangle(bx + 1.0, ground_y, bp - 2.0, bp * 0.28, Color::new(0.46, 0.78, 0.34, 1.0));
            draw_line(bx, ground_y, bx, ground_y + bp, 1.0, Color::new(0.0, 0.0, 0.0, 0.12));
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
                    draw_rectangle(bx + bp*0.25, dy + bp*0.30, bp*0.18, bp*0.18,
                        Color::new(0.36, 0.24, 0.11, 1.0));
                }
                draw_line(bx + bp, dy, bx + bp, (dy + bp).min(h), 1.0, Color::new(0.0, 0.0, 0.0, 0.12));
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
        let title = "MINE  MAZE";
        let ts = (w * 0.115).clamp(52.0, 90.0);
        let tw = measure_text(title, None, ts as u16, 1.0).width;
        let tx = cx - tw * 0.5;
        let ty = ground_y * 0.40;

        // Backdrop block
        draw_rectangle(tx - 20.0, ty - ts + 2.0, tw + 40.0, ts + 14.0,
            Color::new(0.0, 0.0, 0.0, 0.52));
        draw_rectangle_lines(tx - 22.0, ty - ts, tw + 44.0, ts + 18.0, 3.0,
            Color::new(0.58, 0.46, 0.0, 0.88));
        draw_rectangle_lines(tx - 18.0, ty - ts + 4.0, tw + 36.0, ts + 10.0, 1.5,
            Color::new(0.28, 0.22, 0.0, 0.60));

        // Dark gold shadow
        draw_text(title, tx + 5.0, ty + 5.0, ts, Color::new(0.40, 0.28, 0.0, 0.92));
        // Bright gold main text
        draw_text(title, tx, ty, ts, Color::new(1.0, 0.88, 0.0, 1.0));

        // Subtitle (Minecraft edition style)
        let sub = "Multiplayer Minecraft-Style Maze Shooter";
        let ss = (w * 0.025).clamp(13.0, 18.0);
        let sw2 = measure_text(sub, None, ss as u16, 1.0).width;
        draw_text(sub, cx - sw2*0.5 + 2.0, ty + 22.0 + 2.0, ss, Color::new(0.0, 0.0, 0.0, 0.55));
        draw_text(sub, cx - sw2*0.5,        ty + 22.0,       ss, Color::new(0.85, 0.85, 0.85, 1.0));

        // ── INPUT PANEL (dark stone GUI) ──────────────────────────────────────
        let pw = (w * 0.44).clamp(300.0, 440.0);
        let ph = 228.0_f32;
        let ppx = cx - pw * 0.5;
        let ppy = ty + 38.0;

        // Stone tile background
        draw_rectangle(ppx, ppy, pw, ph, Color::new(0.20, 0.20, 0.23, 0.96));
        let bsz = 20.0_f32;
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
        draw_rectangle_lines(ppx - 3.0, ppy - 3.0, pw + 6.0, ph + 6.0, 2.5,
            Color::new(0.60, 0.60, 0.65, 1.0));
        draw_rectangle_lines(ppx - 1.0, ppy - 1.0, pw + 2.0, ph + 2.0, 1.5,
            Color::new(0.08, 0.08, 0.10, 1.0));

        // Fields
        let field_w  = pw - 32.0;
        let field_h  = 36.0_f32;
        let field_x  = ppx + 16.0;
        let ip_y     = ppy + 30.0;
        let name_y   = ppy + 112.0;

        let (mx, my) = mouse_position();
        let clicked  = is_mouse_button_pressed(MouseButton::Left);

        if clicked && mx >= field_x && mx <= field_x + field_w {
            if my >= ip_y   && my <= ip_y + field_h   { focused = Field::Ip;   }
            if my >= name_y && my <= name_y + field_h { focused = Field::Name; }
        }

        let btn_w  = field_w;
        let btn_h  = 42.0_f32;
        let btn_x  = field_x;
        let btn_y  = ppy + ph - btn_h - 14.0;

        let start_hovered = mx >= btn_x && mx <= btn_x + btn_w
                         && my >= btn_y && my <= btn_y + btn_h;
        let start = (clicked && start_hovered) || is_key_pressed(KeyCode::Enter);

        mc_label("Server IP",    field_x, ip_y   - 20.0);
        mc_field(&ip,   field_x, ip_y,   field_w, field_h, focused == Field::Ip);
        mc_label("Player Name",  field_x, name_y - 20.0);
        mc_field(&name, field_x, name_y, field_w, field_h, focused == Field::Name);

        // Minecraft grass/green button
        let (btn_top, btn_bot) = if start_hovered {
            (Color::new(0.56, 0.84, 0.34, 1.0), Color::new(0.38, 0.64, 0.20, 1.0))
        } else {
            (Color::new(0.44, 0.72, 0.26, 1.0), Color::new(0.28, 0.52, 0.14, 1.0))
        };
        draw_rectangle(btn_x, btn_y, btn_w, btn_h, btn_bot);
        draw_rectangle(btn_x, btn_y, btn_w, btn_h * 0.48, btn_top);
        // Inner highlight + dark border
        draw_rectangle_lines(btn_x, btn_y, btn_w, btn_h, 2.0, Color::new(0.0, 0.0, 0.0, 0.60));
        draw_rectangle_lines(btn_x+2.0, btn_y+2.0, btn_w-4.0, btn_h-4.0, 1.0,
            Color::new(1.0, 1.0, 1.0, 0.20));
        let lbl = "PLAY GAME";
        let lw = measure_text(lbl, None, 24, 1.0).width;
        draw_text(lbl, btn_x + (btn_w - lw)*0.5 + 2.0, btn_y + btn_h*0.65 + 2.0, 24.0,
            Color::new(0.0, 0.0, 0.0, 0.60));
        draw_text(lbl, btn_x + (btn_w - lw)*0.5, btn_y + btn_h*0.65, 24.0, WHITE);

        // Footer hint
        draw_text(
            "Tab = switch field   Enter = play   Esc = quit",
            10.0, h - 10.0, 14.0, Color::new(0.88, 0.88, 0.92, 1.0),
        );

        next_frame().await;

        if start {
            let rip   = if ip.trim().is_empty()   { default_ip.clone() } else { ip.trim().to_string() };
            let rname = if name.trim().is_empty()  { "Steve".to_string() } else { name.trim().to_string() };
            return Some((rip, rname));
        }
    }
}

fn mc_label(text: &str, x: f32, y: f32) {
    // Shadow then main
    draw_text(text, x + 1.5, y + 1.5, 16.0, Color::new(0.0, 0.0, 0.0, 0.55));
    draw_text(text, x, y, 16.0, Color::new(0.85, 0.85, 0.80, 1.0));
}

fn mc_field(value: &str, x: f32, y: f32, w: f32, h: f32, focused: bool) {
    // Outer dark border, inner black bg
    draw_rectangle(x, y, w, h, Color::new(0.0, 0.0, 0.0, 0.85));
    draw_rectangle(x + 2.0, y + 2.0, w - 4.0, h - 4.0, Color::new(0.08, 0.08, 0.10, 1.0));
    draw_rectangle_lines(
        x, y, w, h,
        if focused { 2.5 } else { 1.5 },
        if focused { Color::new(1.0, 0.88, 0.0, 1.0) } else { Color::new(0.38, 0.38, 0.45, 1.0) },
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

    draw_rectangle(px, py, pw, ph, Color::new(0.20, 0.15, 0.10, 0.97));
    draw_rectangle_lines(px, py, pw, ph, 2.0, Color::new(0.42, 0.32, 0.24, 1.0));

    // Title
    let title = "MINECRAFT SETTINGS";
    let tm = measure_text(title, None, 34, 1.0);
    draw_text(title, px + (pw - tm.width) * 0.5, py + 46.0, 34.0, WHITE);
    draw_line(px + 20.0, py + 58.0, px + pw - 20.0, py + 58.0, 1.0,
        Color::new(0.35, 0.28, 0.20, 1.0));

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

    // Mouse drag on slider
    let (mx, my) = mouse_position();
    if is_mouse_button_down(MouseButton::Left)
        && mx >= slx && mx <= slx + slw
        && my >= sly - 6.0 && my <= sly + slh + 6.0
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
        {
            let sw = screen_width();
            let sh = screen_height();
            let ccx = sw * 0.5;
            let ccy = sh * 0.5;
            let bp2 = 30.0_f32;
            // Sky
            draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.42, 0.72, 1.0, 1.0));
            // Grass
            draw_rectangle(0.0, sh * 0.66, sw, bp2, Color::new(0.37, 0.68, 0.27, 1.0));
            // Dirt
            draw_rectangle(0.0, sh * 0.66 + bp2, sw, sh, Color::new(0.48, 0.32, 0.18, 1.0));
            // Title
            let conn_title = "MINE  MAZE";
            let cts = (sw * 0.10).clamp(44.0, 80.0);
            let ctw = measure_text(conn_title, None, cts as u16, 1.0).width;
            draw_text(conn_title, ccx - ctw*0.5 + 4.0, ccy - 60.0 + 4.0, cts, Color::new(0.38, 0.28, 0.0, 0.90));
            draw_text(conn_title, ccx - ctw*0.5,        ccy - 60.0,       cts, Color::new(1.0, 0.88, 0.0, 1.0));
            // Connecting text box
            draw_rectangle(ccx - 200.0, ccy - 8.0, 400.0, 80.0, Color::new(0.12, 0.12, 0.15, 0.88));
            draw_rectangle_lines(ccx - 202.0, ccy - 10.0, 404.0, 84.0, 2.0, Color::new(0.50, 0.50, 0.55, 1.0));
            let conn_str = format!("Connecting to {}...", server_addr_str);
            let csw = measure_text(&conn_str, None, 20, 1.0).width;
            draw_text(&conn_str, ccx - csw*0.5, ccy + 16.0, 20.0, Color::new(0.85, 0.85, 0.85, 1.0));
            let pstr = format!("Player: {}", player_name);
            let psw = measure_text(&pstr, None, 18, 1.0).width;
            draw_text(&pstr, ccx - psw*0.5, ccy + 44.0, 18.0, Color::new(0.68, 0.68, 0.72, 1.0));
        }
        next_frame().await;
    };

    let map = Map::generate(seed, difficulty);
    let mut prediction   = PredictionLocal::new();
    // (0,0) is always a border wall — initialize at a real floor tile so
    // collision checks don't block movement before the first server snapshot.
    let (sx, sy) = map.spawn_points.first().copied().unwrap_or((1.5, 1.5));
    prediction.local_state.x = sx;
    prediction.local_state.y = sy;
    prediction.map       = Some(map);
    let mut interpolation = InterpolationRemote::new(100);
    let mut renderer     = Renderer::new();

    let mut messages: Vec<String> = vec![format!(
        "Welcome {}!  WASD=move  Mouse=aim  Click=shoot  Tab=settings  Q=quit",
        player_name
    )];

    let mut sensitivity: f32  = 0.35;
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
            // mouse_delta_position returns a 0..1 normalized fraction of screen width.
            // Multiply by 2π so a full screen-width swipe = 360° at sensitivity 1.0.
            input.turn_delta = -mdx * sensitivity * std::f32::consts::TAU;

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
