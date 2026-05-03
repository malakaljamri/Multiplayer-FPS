use macroquad::prelude::*;
use macroquad::models::Vertex;
use shared::maze::Map;
use shared::protocol::PlayerState;

/// Minimap display size in pixels (one side).
const MINIMAP_PX: f32 = 160.0;

/// Wall height = corridor width → perfect 1×1×1 cubes.
const WALL_H: f32 = 1.0;

/// Eye height (camera Y), just above mid-wall.
const EYE_H: f32 = 0.55;

pub struct BulletMark {
    pub cell_x: i32,
    pub cell_y: i32,
    pub hit_y_side: bool,
    pub wall_frac: f32,
    pub step_x: i32,
    pub step_y: i32,
}

pub struct Renderer {
    fps_display: f32,
    fps_accum: f32,
    fps_frames: u32,
    flash_timer: f32,
    recoil_angle: f32,
    recoil_velocity: f32,
}

#[derive(Copy, Clone)]
enum WallBlockType {
    Brick,
    Cobblestone,
    Mossy,
    Sandstone,
    WoodPlanks,
    NetherBrick,
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

        self.fps_accum += dt;
        self.fps_frames += 1;
        if self.fps_accum >= 0.4 {
            self.fps_display = self.fps_frames as f32 / self.fps_accum;
            self.fps_accum = 0.0;
            self.fps_frames = 0;
        }

        if is_shooting {
            self.flash_timer = 0.12;
            self.recoil_velocity += 0.30;
        }
        self.flash_timer = (self.flash_timer - dt).max(0.0);

        let spring = 38.0_f32;
        let damping = 9.0_f32;
        self.recoil_velocity +=
            (-self.recoil_angle * spring - self.recoil_velocity * damping) * dt;
        self.recoil_angle =
            (self.recoil_angle + self.recoil_velocity * dt).clamp(-0.05, 0.35);

        let w = screen_width();
        let h = screen_height();

        self.draw_3d_view(local, remote_players, map, bullet_marks);
        self.draw_hud(local, remote_players, map, messages, w, h);
        
    }

    // -----------------------------------------------------------------------
    // 3-D world
    // -----------------------------------------------------------------------

    fn draw_3d_view(
        &self,
        player: &PlayerState,
        remote_players: &[PlayerState],
        map: &Map,
        bullet_marks: &[BulletMark],
    ) {
        clear_background(Color::new(0.40, 0.70, 0.98, 1.0));

        // Camera
        let eye = Vec3::new(player.x, EYE_H, player.y);
        let look = Vec3::new(player.angle.cos(), 0.0, player.angle.sin());
        set_camera(&Camera3D {
            position: eye,
            target: eye + look,
            up: Vec3::Y,
            fovy: 66.0_f32.to_radians(),
            ..Default::default()
        });

        // Ground
        draw_cube(
            Vec3::new(0.0, -0.01, 0.0),
            Vec3::new(256.0, 0.02, 256.0),
            None,
            Color::new(0.35, 0.67, 0.27, 1.0),
        );

        // Walls — perfect 1×1×1 cubes
        for row in 0..map.height {
            for col in 0..map.width {
                if map.is_wall(col as f32, row as f32) {
                    let color = Self::wall_color(col as i32, row as i32);
                    draw_cube(
                        Vec3::new(col as f32 + 0.5, WALL_H * 0.5, row as f32 + 0.5),
                        Vec3::new(1.0, WALL_H, 1.0),
                        None,
                        color,
                    );
                }
            }
        }
        
        if !bullet_marks.is_empty() {
            for mark in bullet_marks {

                let y = 0.5;

                let (pos, size) = if mark.hit_y_side {
                    // Hit horizontal wall (constant Z)

                    let wall_z = if mark.step_y > 0 {
                        mark.cell_y as f32
                    } else {
                        mark.cell_y as f32 + 1.0
                    };

                    let x = mark.cell_x as f32 + mark.wall_frac;

                    (
                        vec3(x, y, wall_z),
                        vec3(0.08, 0.08, 0.005),
                    )
                } else {
                    // Hit vertical wall (constant X)

                    let wall_x = if mark.step_x > 0 {
                        mark.cell_x as f32
                    } else {
                        mark.cell_x as f32 + 1.0
                    };

                    let z = mark.cell_y as f32 + mark.wall_frac;

                    (
                        vec3(wall_x, y, z),
                        vec3(0.005, 0.08, 0.08),
                    )
                };

                let normal = if mark.hit_y_side {
                    vec3(0.0, 0.0, -(mark.step_y as f32))
                } else {
                    vec3(-(mark.step_x as f32), 0.0, 0.0)
                };

                let final_pos = pos + normal * 0.01;

                draw_cube(
                    final_pos,
                    size,
                    None,
                    Color::new(0.1, 0.05, 0.05, 1.0),
                );
            }
        }



        // Remote players — drawn as a single oriented model each
        for remote in remote_players {
            if remote.health <= 0 {
                continue;
            }
            self.draw_player_model(remote);
        }

        self.draw_weapon_viewmodelv2(player);
        set_default_camera();

        // Muzzle flash (2-D)
        if self.flash_timer > 0.0 {
            let alpha = self.flash_timer / 0.12 * 0.35;
            draw_rectangle(
                0.0, 0.0,
                screen_width(), screen_height(),
                Color::new(1.0, 0.85, 0.2, alpha),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Player model — all parts in LOCAL space, single Y-axis rotation applied
    // via macroquad's internal GL matrix stack.
    //
    // Local-space convention:
    //   • Model stands at the world origin facing +Z  (i.e. angle = 0)
    //   • Feet rest on Y = 0, top of head at Y ≈ 1.0
    //   • Left/right are ±X  (player's OWN left/right when facing +Z)
    //
    // A Mat4 rotation around Y by `angle` then translates to world position,
    // so the whole figure turns as one rigid body.
    // -----------------------------------------------------------------------
    fn draw_player_model(&self, remote: &PlayerState) {
        // Build: T(world_pos) * R_y(angle)
        // macroquad angles: server angle=0 → facing +X in its 2-D space,
        // which maps to +X in 3-D (cos0=1, sin0=0).  Our local model faces +Z,
        // so we need an extra −90° (−π/2) offset so local +Z aligns with world +X
        // when angle=0.
        let model_mat = Mat4::from_translation(Vec3::new(remote.x, 0.0, remote.y))
            * Mat4::from_rotation_y(-(remote.angle - std::f32::consts::FRAC_PI_2));

        // Push the model matrix onto macroquad's internal GL stack.
        // Everything drawn until pop_matrix() is in model space.
        let gl = unsafe { get_internal_gl() };
        gl.quad_gl.push_model_matrix(model_mat);

        let skin  = Color::new(0.90, 0.74, 0.56, 1.0);
        let shirt = Color::new(0.20, 0.50, 0.85, 1.0);
        let pants = Color::new(0.20, 0.24, 0.58, 1.0);
        let hp_color = if remote.health > 60 { GREEN }
            else if remote.health > 30 { YELLOW } else { RED };

        // All positions below are in LOCAL model space.
        // The model is 1.0 unit tall total:
        //   Legs  : Y  0.00 – 0.30  (centre 0.15)
        //   Body  : Y  0.30 – 0.65  (centre 0.475)
        //   Head  : Y  0.70 – 0.95  (centre 0.825)
        // Gaps between segments are intentional for a blocky Minecraft look.

        // ── Head ──────────────────────────────────────────────────────────
        draw_cube(Vec3::new(0.0,  0.825, 0.0), Vec3::new(0.22, 0.25, 0.22), None, skin);

        // Face (FRONT only = +Z direction in local space)
        // Eyes
        draw_cube(
            Vec3::new(-0.05, 0.85, 0.11),
            Vec3::new(0.04, 0.04, 0.02),
            None,
            WHITE,
        );

        draw_cube(
            Vec3::new(0.05, 0.85, 0.11),
            Vec3::new(0.04, 0.04, 0.02),
            None,
            WHITE,
        );

        // Pupils
        draw_cube(
            Vec3::new(-0.05, 0.85, 0.12),
            Vec3::new(0.02, 0.02, 0.02),
            None,
            BLUE,
        );

        draw_cube(
            Vec3::new(0.05, 0.85, 0.12),
            Vec3::new(0.02, 0.02, 0.02),
            None,
            BLUE,
        );

        // Hair cap
        draw_cube(
            Vec3::new(0.0, 0.92, 0.0),
            Vec3::new(0.24, 0.08, 0.24),
            None,
            Color::new(0.33, 0.20, 0.08, 1.0),
        );
        draw_cube(
            Vec3::new(0.0, 0.88, 0.115),
            Vec3::new(0.24, 0.06, 0.03),
            None,
            Color::new(0.33, 0.20, 0.08, 1.0),
        );

        // Main gun body — pushed farther forward
        draw_cube(
            Vec3::new(0.0, 0.58, 0.26),
            Vec3::new(0.06, 0.06, 0.3),
            None,
            DARKGRAY,
        );

        // Barrel
        draw_cube(
            Vec3::new(0.0, 0.58, 0.41),
            Vec3::new(0.03, 0.03, 0.01),
            None,
            BLACK,
        );

        // Handle / grip
        draw_cube(
            Vec3::new(0.0, 0.5, 0.18),
            Vec3::new(0.045, 0.14, 0.05),
            None,
            Color::new(0.20, 0.20, 0.20, 1.0),
        );

        // ── Body ──────────────────────────────────────────────────────────
        draw_cube(Vec3::new(0.0,  0.475, 0.0), Vec3::new(0.22, 0.35, 0.12), None, shirt);

        // ── Arms (±X from body, same height band as body) ─────────────────
        draw_cube(Vec3::new(-0.17, 0.475, 0.0), Vec3::new(0.10, 0.35, 0.10), None, skin);
        draw_cube(Vec3::new( 0.17, 0.475, 0.0), Vec3::new(0.10, 0.35, 0.10), None, skin);

        // ── Legs (±X, lower half) ─────────────────────────────────────────
        draw_cube(Vec3::new(-0.06, 0.15,  0.0), Vec3::new(0.10, 0.30, 0.10), None, pants);
        draw_cube(Vec3::new( 0.06, 0.15,  0.0), Vec3::new(0.10, 0.30, 0.10), None, pants);

        // ── Health wireframe ──────────────────────────────────────────────
        draw_cube_wires(Vec3::new(0.0, 0.475, 0.0), Vec3::new(0.40, 0.95, 0.25), hp_color);

        gl.quad_gl.pop_model_matrix();
    }

    // -----------------------------------------------------------------------
    // Weapon viewmodel (2-D rectangles, unchanged)
    // -----------------------------------------------------------------------

    fn draw_weapon_viewmodel(&self, w: f32, h: f32) {
        let rd = self.recoil_angle * h * 0.18;
        let s = (h * 0.026).max(8.0);
        let rx = w * 0.70;
        let ry = h * 0.64 + rd;

        let m  = Color::new(0.52, 0.54, 0.58, 1.0);
        let mh = Color::new(0.70, 0.72, 0.76, 1.0);
        let ms = Color::new(0.28, 0.29, 0.32, 1.0);
        let wc = Color::new(0.54, 0.32, 0.12, 1.0);
        let wh = Color::new(0.68, 0.45, 0.22, 1.0);
        let wd = Color::new(0.34, 0.19, 0.07, 1.0);
        let bk = Color::new(0.07, 0.07, 0.08, 1.0);

        draw_rectangle(rx - s*9.0, ry - s*1.5, s*9.0, s*1.5, m);
        draw_rectangle(rx - s*9.0, ry - s*2.0, s*9.0, s*0.5, mh);
        draw_rectangle(rx - s*9.0, ry,          s*9.0, s*0.5, ms);
        draw_rectangle(rx - s*9.5, ry - s*2.0, s*0.5, s*2.5, bk);

        draw_rectangle(rx - s*7.5, ry - s*2.5, s*0.6, s*1.0, bk);
        draw_rectangle(rx - s*2.2, ry - s*3.6, s*2.2, s*0.45, bk);
        draw_rectangle(rx - s*2.2, ry - s*4.1, s*0.6, s*0.55, bk);
        draw_rectangle(rx - s*0.7, ry - s*4.1, s*0.6, s*0.55, bk);

        draw_rectangle(rx - s*1.5, ry - s*2.5, s*5.0, s*5.0, ms);
        draw_rectangle(rx - s*1.5, ry - s*3.0, s*5.0, s*0.5, m);
        draw_rectangle(rx + s*3.5, ry - s*3.0, s*0.5, s*6.0, bk);
        draw_rectangle(rx - s*1.0, ry - s*2.0, s*3.0, s*1.5,
            Color::new(0.13, 0.13, 0.16, 1.0));

        draw_rectangle(rx + s*3.5, ry - s*2.0, s*8.0, s*3.5, wc);
        draw_rectangle(rx + s*3.5, ry - s*2.5, s*8.0, s*0.5, wh);
        draw_rectangle(rx + s*3.5, ry + s*1.5, s*8.0, s*0.5, wd);
        draw_rectangle(rx + s*11.5, ry - s*2.5, s*0.5, s*4.5, wd);
        for i in 0i32..3 {
            draw_rectangle(rx + s*4.0, ry - s*(1.4 - i as f32 * 0.75),
                s*6.5, s*0.22, wd);
        }

        draw_rectangle(rx + s*0.5, ry + s*2.5, s*2.0, s*5.5, wc);
        draw_rectangle(rx + s*0.5, ry + s*2.5, s*2.0, s*0.4, wd);
        draw_rectangle(rx + s*2.5, ry + s*2.5, s*0.4, s*5.5, bk);
        for i in 0i32..5 {
            draw_rectangle(rx + s*0.75, ry + s*(3.0 + i as f32 * 0.85),
                s*1.5, s*0.32, wd);
        }
        draw_rectangle(rx + s*0.5, ry + s*8.0, s*2.0, s*0.5, wd);

        draw_rectangle(rx - s*0.5, ry + s*2.0, s*2.5, s*0.35, bk);
        draw_rectangle(rx - s*0.5, ry + s*2.0, s*0.35, s*2.0, bk);
        draw_rectangle(rx + s*1.65, ry + s*2.0, s*0.35, s*2.0, bk);
        draw_rectangle(rx - s*0.5, ry + s*4.0, s*2.5, s*0.35, bk);
        draw_rectangle(rx + s*0.5, ry + s*2.4, s*0.4, s*1.5, bk);

        draw_rectangle(rx + s*0.3, ry + s*0.5, s*1.8, s*2.2, ms);
        draw_rectangle(rx + s*0.3, ry + s*0.5, s*1.8, s*0.4, m);
    }

    fn draw_weapon_viewmodelv2(&self, player: &PlayerState) {
        let eye = vec3(player.x, EYE_H, player.y);

        let forward = vec3(player.angle.cos(), 0.0, player.angle.sin());
        let right = vec3(-forward.z, 0.0, forward.x);
        let up = vec3(0.0, 0.5, 0.0);

        let recoil = self.recoil_angle * 0.04;

        // FPS placement
        let gun_pos =
            eye
            + forward * 0.30
            + right * 0.18
            + up * (-0.18 + recoil);

        let yaw = player.angle - std::f32::consts::FRAC_PI_2;
        let model =
            Mat4::from_translation(gun_pos)
            * Mat4::from_rotation_y(-yaw)
            * Mat4::from_rotation_x(-0.20);

        let gl = unsafe { get_internal_gl() };
        gl.quad_gl.push_model_matrix(model);

        let gun_color = DARKGRAY;
        let grip_color = Color::new(0.20, 0.20, 0.20, 1.0);

        // Main body (same proportions as player gun)
        draw_cube(
            vec3(0.0, 0.0, 0.0),
            vec3(0.06, 0.06, 0.30),
            None,
            gun_color,
        );

        // Barrel
        draw_cube(
            vec3(0.0, 0.0, 0.18),
            vec3(0.03, 0.03, 0.05),
            None,
            BLACK,
        );

        // Grip / handle
        draw_cube(
            vec3(0.0, -0.08, -0.05),
            vec3(0.045, 0.14, 0.05),
            None,
            grip_color,
        );

        gl.quad_gl.pop_model_matrix();
    }

    // -----------------------------------------------------------------------
    // HUD
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

        draw_rectangle(ox-2.0, oy-2.0, mm_w+4.0, mm_h+4.0,
            Color::new(0.0,0.0,0.0,0.65));

        for row in 0..map.height {
            for col in 0..map.width {
                if map.is_wall(col as f32, row as f32) {
                    draw_rectangle(
                        ox + col as f32 * cell, oy + row as f32 * cell,
                        cell, cell,
                        Self::wall_color(col as i32, row as i32),
                    );
                }
            }
        }

        for rp in remote_players {
            draw_circle(ox + rp.x*cell, oy + rp.y*cell, (cell*0.8), RED);
        }

        let px = ox + local.x * cell;
        let py = oy + local.y * cell;
        draw_circle(px, py, (cell*1.0).max(2.5), GREEN);
        draw_line(px, py,
            px + local.angle.cos() * cell * 2.0,
            py + local.angle.sin() * cell * 2.0,
            1.5, BLACK);

        draw_rectangle_lines(ox-2.0, oy-2.0, mm_w+4.0, mm_h+4.0, 1.5, DARKGRAY);
    }

    fn draw_health_bar(&self, local: &PlayerState, h: f32) {
        let bar_w = 200.0;
        let bar_h = 18.0;
        let x = 14.0;
        let y = h - bar_h - 14.0;
        draw_rectangle(x, y, bar_w, bar_h, Color::new(0.15, 0.15, 0.15, 0.8));
        let pct = local.health as f32 / 100.0;
        let color = if local.health > 60 { GREEN }
            else if local.health > 30 { YELLOW } else { RED };
        draw_rectangle(x, y, bar_w*pct, bar_h, color);
        draw_rectangle_lines(x, y, bar_w, bar_h, 1.5, DARKGRAY);
        draw_text(&format!("HP: {}", local.health), x+6.0, y+13.0, 16.0, WHITE);
    }

    fn draw_fps(&self, w: f32) {
        let color = if self.fps_display >= 50.0 { GREEN }
            else if self.fps_display >= 30.0 { YELLOW } else { RED };
        draw_text(&format!("FPS: {:.0}", self.fps_display), w-90.0, 22.0, 20.0, color);
    }

    fn draw_crosshair(&self, w: f32, h: f32) {
        let cx = w * 0.5;
        let cy = h * 0.5;
        let (gap, len, t) = (5.0, 9.0, 1.5);
        let c = Color::new(0.0, 1.0, 0.25, 0.85);
        draw_line(cx-gap-len, cy, cx-gap, cy, t, c);
        draw_line(cx+gap, cy, cx+gap+len, cy, t, c);
        draw_line(cx, cy-gap-len, cx, cy-gap, t, c);
        draw_line(cx, cy+gap, cx, cy+gap+len, t, c);
        draw_circle(cx, cy, 1.5, c);
    }

    fn draw_messages(&self, messages: &[String]) {
        for (i, msg) in messages.iter().enumerate() {
            draw_text(msg, 14.0, 22.0 + i as f32 * 20.0, 17.0, MAGENTA);
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn wall_block_type(x: i32, y: i32) -> WallBlockType {
        match ((x * 31 + y * 17).abs() % 6) as i32 {
            0 => WallBlockType::Brick,
            1 => WallBlockType::Cobblestone,
            2 => WallBlockType::Mossy,
            3 => WallBlockType::Sandstone,
            4 => WallBlockType::WoodPlanks,
            _ => WallBlockType::NetherBrick,
        }
    }

    fn wall_color(x: i32, y: i32) -> Color {
        match Self::wall_block_type(x, y) {
            WallBlockType::Brick       => Color::new(0.68, 0.30, 0.26, 1.0),
            WallBlockType::Cobblestone => Color::new(0.50, 0.50, 0.54, 1.0),
            WallBlockType::Mossy       => Color::new(0.36, 0.48, 0.30, 1.0),
            WallBlockType::Sandstone   => Color::new(0.75, 0.68, 0.45, 1.0),
            WallBlockType::WoodPlanks  => Color::new(0.72, 0.56, 0.32, 1.0),
            WallBlockType::NetherBrick => Color::new(0.34, 0.10, 0.10, 1.0),
        }
    }
}