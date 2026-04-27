use macroquad::prelude::*;
use shared::maze::Map;
use shared::protocol::PlayerState;

/// Horizontal field of view in radians (66°).
const FOV: f32 = 1.1519173;
const HALF_FOV: f32 = FOV / 2.0;

/// Minimap display size in pixels (one side).
const MINIMAP_PX: f32 = 160.0;

/// A bullet impact mark on a wall surface.
pub struct BulletMark {
    pub cell_x: i32,
    pub cell_y: i32,
    /// True when the hit wall was a Y-axis face (horizontal wall boundary).
    pub hit_y_side: bool,
    /// Fractional position 0..1 along the struck wall face.
    pub wall_frac: f32,
}

pub struct Renderer {
    fps_display: f32,
    fps_accum: f32,
    fps_frames: u32,
    flash_timer: f32,
    recoil_angle: f32,
    recoil_velocity: f32,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            fps_display: 0.0,
            fps_accum: 0.0,
            fps_frames: 0,
            flash_timer: 0.0,
            recoil_angle: 0.0,
            recoil_velocity: 0.0,
        }
    }

    pub fn render(
        &mut self,
        local: &PlayerState,
        remote_players: &[PlayerState],
        map: &Map,
        messages: &[String],
        is_shooting: bool,
        bullet_marks: &[BulletMark],
    ) {
        let dt = get_frame_time();

        // FPS averaging over 0.4s windows
        self.fps_accum += dt;
        self.fps_frames += 1;
        if self.fps_accum >= 0.4 {
            self.fps_display = self.fps_frames as f32 / self.fps_accum;
            self.fps_accum = 0.0;
            self.fps_frames = 0;
        }

        if is_shooting {
            self.flash_timer = 0.12;
            self.recoil_velocity += 0.30;   // impulse upward
        }
        self.flash_timer = (self.flash_timer - dt).max(0.0);

        // Spring-damper recoil: pulls back to 0
        let spring = 38.0_f32;
        let damping = 9.0_f32;
        self.recoil_velocity += (-self.recoil_angle * spring - self.recoil_velocity * damping) * dt;
        self.recoil_angle = (self.recoil_angle + self.recoil_velocity * dt).clamp(-0.05, 0.35);

        let w = screen_width();
        let h = screen_height();

        self.draw_3d_view(local, remote_players, map, w, h, bullet_marks);
        self.draw_weapon_viewmodel(w, h);
        self.draw_hud(local, remote_players, map, messages, w, h);
    }

    // -----------------------------------------------------------------------
    // 3D raycasting view
    // -----------------------------------------------------------------------

    fn draw_3d_view(
        &self,
        player: &PlayerState,
        remote_players: &[PlayerState],
        map: &Map,
        w: f32,
        h: f32,
        bullet_marks: &[BulletMark],
    ) {
        // Ceiling (very dark blue-black) and floor (slightly lighter)
        draw_rectangle(0.0, 0.0, w, h * 0.5, Color::new(0.04, 0.04, 0.07, 1.0));
        draw_rectangle(
            0.0,
            h * 0.5,
            w,
            h * 0.5,
            Color::new(0.14, 0.14, 0.16, 1.0),
        );

        let cols = w as usize;
        let mut z_buf = vec![f32::MAX; cols];

        // Cast one ray per screen column
        for col in 0..cols {
            // Map column to camera plane position (-1..1)
            let cam_x = 2.0 * col as f32 / w - 1.0;
            let ray_angle = player.angle + cam_x * HALF_FOV;
            let rdx = ray_angle.cos();
            let rdy = ray_angle.sin();

            let (perp, hit_y_side, mx, my) = Self::cast_ray(player.x, player.y, rdx, rdy, map);
            z_buf[col] = perp;

            if perp <= 0.0 {
                continue;
            }

            let wall_h = (h / perp).min(h * 3.0);
            let top = (h - wall_h) * 0.5;

            // Distance fog + side shading (EW side = darker, NS = lighter)
            let fog = (1.0 - perp / 16.0).clamp(0.0, 1.0);
            let side_dim = if hit_y_side { 0.65 } else { 1.0 };
            let b = (fog * side_dim * 0.85 + 0.12).min(1.0);

            // Classic Maze Wars monochrome walls — slight blue tint
            draw_line(
                col as f32,
                top,
                col as f32,
                top + wall_h,
                1.0,
                Color::new(b * 0.88, b * 0.90, b, 1.0),
            );

            // Bullet mark overlay — dark burn mark on matching columns
            if !bullet_marks.is_empty() {
                let wall_frac = if !hit_y_side {
                    let hit = player.y + perp * rdy;
                    hit - hit.floor()
                } else {
                    let hit = player.x + perp * rdx;
                    hit - hit.floor()
                };
                for mark in bullet_marks {
                    if mark.cell_x == mx && mark.cell_y == my && mark.hit_y_side == hit_y_side {
                        let d = (wall_frac - mark.wall_frac).abs();
                        if d < 0.05 {
                            let fade = 1.0 - d / 0.05;
                            let hole_h = (wall_h * 0.14).max(4.0);
                            let hole_y = (h - hole_h) * 0.5;
                            draw_line(
                                col as f32,
                                hole_y,
                                col as f32,
                                hole_y + hole_h,
                                2.5,
                                Color::new(0.05, 0.02, 0.02, fade * 0.95),
                            );
                        }
                    }
                }
            }
        }

        // Draw "eye" sprites for remote players
        self.draw_sprites(player, remote_players, &z_buf, w, h);

        // Muzzle flash overlay
        if self.flash_timer > 0.0 {
            let alpha = self.flash_timer / 0.12 * 0.35;
            draw_rectangle(0.0, 0.0, w, h, Color::new(1.0, 0.85, 0.2, alpha));
        }
    }

    // -----------------------------------------------------------------------
    // DDA ray cast — returns (perpendicular_distance, hit_y_side)
    // -----------------------------------------------------------------------

    // Returns (perp_dist, hit_y_side, cell_x, cell_y)
    fn cast_ray(px: f32, py: f32, rdx: f32, rdy: f32, map: &Map) -> (f32, bool, i32, i32) {
        let mut mx = px as i32;
        let mut my = py as i32;

        let ddx = if rdx.abs() < 1e-20 { 1e20 } else { (1.0 / rdx).abs() };
        let ddy = if rdy.abs() < 1e-20 { 1e20 } else { (1.0 / rdy).abs() };

        let (step_x, mut sx): (i32, f32) = if rdx < 0.0 {
            (-1, (px - mx as f32) * ddx)
        } else {
            (1, (mx as f32 + 1.0 - px) * ddx)
        };
        let (step_y, mut sy): (i32, f32) = if rdy < 0.0 {
            (-1, (py - my as f32) * ddy)
        } else {
            (1, (my as f32 + 1.0 - py) * ddy)
        };

        let mut hit_y = false;

        for _ in 0..128 {
            if sx < sy {
                sx += ddx;
                mx += step_x;
                hit_y = false;
            } else {
                sy += ddy;
                my += step_y;
                hit_y = true;
            }
            if map.is_wall(mx as f32, my as f32) {
                break;
            }
        }

        let perp = if !hit_y {
            (mx as f32 - px + (1 - step_x) as f32 * 0.5) / rdx
        } else {
            (my as f32 - py + (1 - step_y) as f32 * 0.5) / rdy
        };

        (perp.max(0.01), hit_y, mx, my)
    }

    // -----------------------------------------------------------------------
    // Sprite / enemy rendering (Maze Wars "eye" style)
    // -----------------------------------------------------------------------

    fn draw_sprites(
        &self,
        player: &PlayerState,
        remote_players: &[PlayerState],
        z_buf: &[f32],
        w: f32,
        h: f32,
    ) {
        // Sort farthest-first so nearer sprites overdraw
        let mut sprites: Vec<(&PlayerState, f32)> = remote_players
            .iter()
            .map(|p| {
                let dx = p.x - player.x;
                let dy = p.y - player.y;
                (p, (dx * dx + dy * dy).sqrt())
            })
            .collect();
        sprites.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for (remote, dist) in sprites {
            if dist < 0.3 {
                continue;
            }

            let dx = remote.x - player.x;
            let dy = remote.y - player.y;

            // Angle from player to sprite, relative to player's facing direction
            let mut rel_angle = dy.atan2(dx) - player.angle;
            while rel_angle < -std::f32::consts::PI {
                rel_angle += std::f32::consts::TAU;
            }
            while rel_angle > std::f32::consts::PI {
                rel_angle -= std::f32::consts::TAU;
            }

            // Skip if outside FOV (+10% margin)
            if rel_angle.abs() > HALF_FOV * 1.1 {
                continue;
            }

            // Project onto screen column
            let screen_x = ((rel_angle / FOV + 0.5) * w) as i32;

            // Check z-buffer occlusion at center column
            let col_idx = screen_x.clamp(0, w as i32 - 1) as usize;
            if dist >= z_buf[col_idx] {
                continue;
            }

            // Size based on distance
            let sprite_h = (h / dist * 0.75).clamp(8.0, h * 0.85);
            let sprite_w = sprite_h * 0.65;
            let top = (h - sprite_h) * 0.5;
            let left = screen_x as f32 - sprite_w * 0.5;

            // Face background (skin tone)
            draw_rectangle(left, top, sprite_w, sprite_h, Color::new(0.85, 0.78, 0.55, 1.0));

            // Two eyes — the signature Maze Wars look
            let eye_w = sprite_w * 0.22;
            let eye_h = sprite_h * 0.28;
            let eye_y = top + sprite_h * 0.28;
            let left_eye_x = left + sprite_w * 0.14;
            let right_eye_x = left + sprite_w * 0.55;

            // Eye whites
            draw_rectangle(left_eye_x, eye_y, eye_w, eye_h, WHITE);
            draw_rectangle(right_eye_x, eye_y, eye_w, eye_h, WHITE);

            // Pupils (slightly off-center, looking at you)
            let pupil_w = eye_w * 0.55;
            let pupil_h = eye_h * 0.65;
            let pupil_ox = (eye_w - pupil_w) * 0.5;
            let pupil_oy = (eye_h - pupil_h) * 0.5;
            draw_rectangle(left_eye_x + pupil_ox, eye_y + pupil_oy, pupil_w, pupil_h, BLACK);
            draw_rectangle(right_eye_x + pupil_ox, eye_y + pupil_oy, pupil_w, pupil_h, BLACK);

            // Mouth (a grim line)
            let mouth_y = top + sprite_h * 0.72;
            let mouth_w = sprite_w * 0.5;
            let mouth_x = left + (sprite_w - mouth_w) * 0.5;
            draw_line(mouth_x, mouth_y, mouth_x + mouth_w, mouth_y, 2.0_f32.max(sprite_h * 0.025), DARKBROWN);

            // Health-colored outline
            let hp_color = if remote.health > 60 {
                GREEN
            } else if remote.health > 30 {
                YELLOW
            } else {
                RED
            };
            draw_rectangle_lines(left, top, sprite_w, sprite_h, 2.0, hp_color);
        }
    }

    // -----------------------------------------------------------------------
    // Minecraft-style block gun viewmodel (drawn with rectangles, no texture)
    // -----------------------------------------------------------------------

    fn draw_weapon_viewmodel(&self, w: f32, h: f32) {
        let rd = self.recoil_angle * h * 0.18; // recoil drop

        // One "block" in pixels, scaled to screen height
        let s = (h * 0.026).max(8.0);

        // Receiver anchor — bottom-right area
        let rx = w * 0.70;
        let ry = h * 0.64 + rd;

        // ── palette ───────────────────────────────────────────────────────
        let m  = Color::new(0.52, 0.54, 0.58, 1.0); // steel mid
        let mh = Color::new(0.70, 0.72, 0.76, 1.0); // steel highlight (top)
        let ms = Color::new(0.28, 0.29, 0.32, 1.0); // steel shadow
        let wc = Color::new(0.54, 0.32, 0.12, 1.0); // wood
        let wh = Color::new(0.68, 0.45, 0.22, 1.0); // wood highlight
        let wd = Color::new(0.34, 0.19, 0.07, 1.0); // wood dark
        let bk = Color::new(0.07, 0.07, 0.08, 1.0); // black

        // ── BARREL (points left toward screen centre) ─────────────────────
        draw_rectangle(rx - s*9.0, ry - s*1.5, s*9.0, s*1.5, m);   // face
        draw_rectangle(rx - s*9.0, ry - s*2.0, s*9.0, s*0.5, mh);  // top face
        draw_rectangle(rx - s*9.0, ry,          s*9.0, s*0.5, ms);  // bottom shadow
        draw_rectangle(rx - s*9.5, ry - s*2.0, s*0.5, s*2.5, bk);  // muzzle cap

        // ── IRON SIGHTS ───────────────────────────────────────────────────
        draw_rectangle(rx - s*7.5, ry - s*2.5, s*0.6, s*1.0, bk);  // front post
        draw_rectangle(rx - s*2.2, ry - s*3.6, s*2.2, s*0.45, bk); // rear bar
        draw_rectangle(rx - s*2.2, ry - s*4.1, s*0.6, s*0.55, bk); // rear left notch
        draw_rectangle(rx - s*0.7, ry - s*4.1, s*0.6, s*0.55, bk); // rear right notch

        // ── RECEIVER / BODY ───────────────────────────────────────────────
        draw_rectangle(rx - s*1.5, ry - s*2.5, s*5.0, s*5.0, ms);  // front face
        draw_rectangle(rx - s*1.5, ry - s*3.0, s*5.0, s*0.5, m);   // top face
        draw_rectangle(rx + s*3.5, ry - s*3.0, s*0.5, s*6.0, bk);  // right edge
        // ejection port cutout
        draw_rectangle(rx - s*1.0, ry - s*2.0, s*3.0, s*1.5,
            Color::new(0.13, 0.13, 0.16, 1.0));

        // ── STOCK (wood, going right) ─────────────────────────────────────
        draw_rectangle(rx + s*3.5, ry - s*2.0, s*8.0, s*3.5, wc);  // face
        draw_rectangle(rx + s*3.5, ry - s*2.5, s*8.0, s*0.5, wh);  // top
        draw_rectangle(rx + s*3.5, ry + s*1.5, s*8.0, s*0.5, wd);  // bottom
        draw_rectangle(rx + s*11.5, ry - s*2.5, s*0.5, s*4.5, wd); // butt plate
        // wood-grain lines
        for i in 0i32..3 {
            draw_rectangle(rx + s*4.0, ry - s*(1.4 - i as f32 * 0.75),
                s*6.5, s*0.22, wd);
        }

        // ── GRIP / HANDLE (going down) ────────────────────────────────────
        draw_rectangle(rx + s*0.5, ry + s*2.5, s*2.0, s*5.5, wc);  // face
        draw_rectangle(rx + s*0.5, ry + s*2.5, s*2.0, s*0.4, wd);  // top shadow
        draw_rectangle(rx + s*2.5, ry + s*2.5, s*0.4, s*5.5, bk);  // right edge
        // checkering
        for i in 0i32..5 {
            draw_rectangle(rx + s*0.75, ry + s*(3.0 + i as f32 * 0.85),
                s*1.5, s*0.32, wd);
        }
        draw_rectangle(rx + s*0.5, ry + s*8.0, s*2.0, s*0.5, wd);  // grip base

        // ── TRIGGER GUARD ─────────────────────────────────────────────────
        draw_rectangle(rx - s*0.5, ry + s*2.0, s*2.5, s*0.35, bk); // top
        draw_rectangle(rx - s*0.5, ry + s*2.0, s*0.35, s*2.0, bk); // left side
        draw_rectangle(rx + s*1.65, ry + s*2.0, s*0.35, s*2.0, bk);// right side
        draw_rectangle(rx - s*0.5, ry + s*4.0, s*2.5, s*0.35, bk); // bottom
        // trigger
        draw_rectangle(rx + s*0.5, ry + s*2.4, s*0.4, s*1.5, bk);

        // ── MAGAZINE ──────────────────────────────────────────────────────
        draw_rectangle(rx + s*0.3, ry + s*0.5, s*1.8, s*2.2, ms);
        draw_rectangle(rx + s*0.3, ry + s*0.5, s*1.8, s*0.4, m);   // mag top
    }

    // -----------------------------------------------------------------------
    // HUD overlay (minimap, health, FPS, crosshair, messages)
    // -----------------------------------------------------------------------

    fn draw_hud(
        &self,
        local: &PlayerState,
        remote_players: &[PlayerState],
        map: &Map,
        messages: &[String],
        w: f32,
        h: f32,
    ) {
        self.draw_minimap(local, remote_players, map, w, h);
        self.draw_health_bar(local, h);
        self.draw_fps(w);
        self.draw_crosshair(w, h);
        self.draw_messages(messages);
    }

    fn draw_minimap(
        &self,
        local: &PlayerState,
        remote_players: &[PlayerState],
        map: &Map,
        w: f32,
        h: f32,
    ) {
        let cell = (MINIMAP_PX / map.width.max(map.height) as f32).max(1.0);
        let mm_w = map.width as f32 * cell;
        let mm_h = map.height as f32 * cell;
        let ox = w - mm_w - 14.0;
        let oy = h - mm_h - 14.0;

        // Background
        draw_rectangle(ox - 2.0, oy - 2.0, mm_w + 4.0, mm_h + 4.0, Color::new(0.0, 0.0, 0.0, 0.65));

        // Walls
        for row in 0..map.height {
            for col in 0..map.width {
                if map.is_wall(col as f32, row as f32) {
                    draw_rectangle(
                        ox + col as f32 * cell,
                        oy + row as f32 * cell,
                        cell,
                        cell,
                        Color::new(0.55, 0.55, 0.65, 1.0),
                    );
                }
            }
        }

        // Remote players (red dots)
        for rp in remote_players {
            let rx = ox + rp.x * cell;
            let ry = oy + rp.y * cell;
            draw_circle(rx, ry, (cell * 1.2).max(2.0), RED);
        }

        // Local player (bright green) with direction arrow
        let px = ox + local.x * cell;
        let py = oy + local.y * cell;
        draw_circle(px, py, (cell * 1.4).max(2.5), GREEN);

        // Direction indicator
        let arrow_len = cell * 2.5;
        draw_line(
            px,
            py,
            px + local.angle.cos() * arrow_len,
            py + local.angle.sin() * arrow_len,
            1.5,
            LIME,
        );

        // Border
        draw_rectangle_lines(ox - 2.0, oy - 2.0, mm_w + 4.0, mm_h + 4.0, 1.5, DARKGRAY);
    }

    fn draw_health_bar(&self, local: &PlayerState, h: f32) {
        let bar_w = 200.0;
        let bar_h = 18.0;
        let x = 14.0;
        let y = h - bar_h - 14.0;

        draw_rectangle(x, y, bar_w, bar_h, Color::new(0.15, 0.15, 0.15, 0.8));
        let pct = local.health as f32 / 100.0;
        let color = if local.health > 60 {
            GREEN
        } else if local.health > 30 {
            YELLOW
        } else {
            RED
        };
        draw_rectangle(x, y, bar_w * pct, bar_h, color);
        draw_rectangle_lines(x, y, bar_w, bar_h, 1.5, DARKGRAY);
        draw_text(
            &format!("HP: {}", local.health),
            x + 6.0,
            y + 13.0,
            16.0,
            WHITE,
        );
    }

    fn draw_fps(&self, w: f32) {
        let fps_text = format!("FPS: {:.0}", self.fps_display);
        let color = if self.fps_display >= 50.0 {
            GREEN
        } else if self.fps_display >= 30.0 {
            YELLOW
        } else {
            RED
        };
        draw_text(&fps_text, w - 90.0, 22.0, 20.0, color);
    }

    fn draw_crosshair(&self, w: f32, h: f32) {
        let cx = w * 0.5;
        let cy = h * 0.5;
        let gap = 5.0;
        let len = 9.0;
        let t = 1.5;
        let c = Color::new(0.0, 1.0, 0.25, 0.85);

        draw_line(cx - gap - len, cy, cx - gap, cy, t, c);
        draw_line(cx + gap, cy, cx + gap + len, cy, t, c);
        draw_line(cx, cy - gap - len, cx, cy - gap, t, c);
        draw_line(cx, cy + gap, cx, cy + gap + len, t, c);
        draw_circle(cx, cy, 1.5, c);
    }

    fn draw_messages(&self, messages: &[String]) {
        for (i, msg) in messages.iter().enumerate() {
            draw_text(msg, 14.0, 22.0 + i as f32 * 20.0, 17.0, MAGENTA);
        }
    }
}
