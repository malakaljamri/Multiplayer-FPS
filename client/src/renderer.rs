use macroquad::prelude::*;
use shared::maze::Map;
use shared::protocol::PlayerState;

pub struct Renderer {
    width: f32,
    height: f32,
    time: f32,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            width: screen_width(),
            height: screen_height(),
            time: 0.0,
        }
    }

    /// Renders a full frame using macroquad 3D and 2D.
    pub fn render(
        &mut self,
        local_state: &PlayerState,
        remote_players: &[PlayerState],
        map: &Map,
        messages: &[String],
        is_shooting: bool,
    ) {
        self.width = screen_width();
        self.height = screen_height();
        self.time += get_frame_time();

        // Clear background to a deep space/twilight color
        clear_background(Color::new(0.05, 0.05, 0.1, 1.0));

        // Camera Bobbing
        // Only bob if player is moving (we could pass a 'is_moving' flag, but for now we'll just check time)
        // Note: Real games use speed, but we'll use a subtle continuous float for "breathing" if stationary
        let bob_offset = (self.time * 10.0).sin() * 0.02;

        let cam_x = local_state.x;
        let cam_y = 0.5 + bob_offset; // Eye height + bob
        let cam_z = local_state.y;

        let look_x = cam_x + local_state.angle.cos();
        let look_y = cam_y - 0.05; // Slightly look down
        let look_z = cam_z + local_state.angle.sin();

        set_camera(&Camera3D {
            position: vec3(cam_x, cam_y, cam_z),
            target: vec3(look_x, look_y, look_z),
            up: vec3(0., 1., 0.),
            fovy: std::f32::consts::PI / 3.0, // 60 degrees
            aspect: Some(self.width / self.height),
            ..Default::default()
        });

        // Draw Floor (a large plane)
        draw_cube(
            vec3(map.width as f32 / 2.0, 0.0, map.height as f32 / 2.0),
            vec3(map.width as f32, 0.01, map.height as f32),
            None,
            Color::new(0.3, 0.3, 0.3, 1.0),
        );

        // Draw Walls with Distance-based "Fog" (manual color dimming)
        for y in 0..map.height {
            for x in 0..map.width {
                if map.is_wall(x as f32, y as f32) {
                    let wx = x as f32 + 0.5;
                    let wz = y as f32 + 0.5;

                    // distance to camera
                    let dx = wx - cam_x;
                    let dz = wz - cam_z;
                    let dist = (dx * dx + dz * dz).sqrt();

                    let fog_factor = (1.0 - (dist / 15.0)).clamp(0.0, 1.0);
                    let wall_color = Color::new(
                        0.4 * fog_factor + 0.1,
                        0.4 * fog_factor + 0.1,
                        0.7 * fog_factor + 0.2,
                        1.0,
                    );
                    let wire_color =
                        Color::new(0.2 * fog_factor, 0.2 * fog_factor, 0.5 * fog_factor, 1.0);

                    draw_cube(vec3(wx, 0.5, wz), vec3(1.0, 1.0, 1.0), None, wall_color);

                    draw_cube_wires(vec3(wx, 0.5, wz), vec3(1.0, 1.0, 1.0), wire_color);
                }
            }
        }

        // Draw Remote Players (as simple capsules or stacked cubes)
        for remote in remote_players {
            let rx = remote.x;
            let rz = remote.y;

            draw_cylinder(vec3(rx, 0.3, rz), 0.2, 0.2, 0.6, None, RED);

            // Draw head
            draw_sphere(
                vec3(rx, 0.7, rz),
                0.15, // radius
                None,
                YELLOW,
            );
        }

        // Back to 2D for UI
        set_default_camera();

        self.draw_hud(local_state, map, messages, is_shooting);
    }

    fn draw_hud(&self, local: &PlayerState, map: &Map, messages: &[String], is_shooting: bool) {
        // --- 1. Crosshair ---
        let center_x = self.width / 2.0;
        let center_y = self.height / 2.0;

        if is_shooting {
            // Animated Muzzle flash
            let scale = (self.time * 50.0).sin().abs() * 5.0 + 10.0;
            draw_circle(center_x, center_y, scale, Color::new(1.0, 0.8, 0.2, 0.8));
            draw_circle(center_x, center_y, scale * 0.5, WHITE);

            // Recoil kick for crosshair
            let kick = 5.0;
            draw_line(
                center_x - 15.0,
                center_y,
                center_x - 5.0,
                center_y,
                2.0,
                RED,
            );
            draw_line(
                center_x + 5.0,
                center_y,
                center_x + 15.0,
                center_y,
                2.0,
                RED,
            );
            draw_line(
                center_x,
                center_y - 15.0 - kick,
                center_x,
                center_y - 5.0 - kick,
                2.0,
                RED,
            );
            draw_line(
                center_x,
                center_y + 5.0,
                center_x,
                center_y + 15.0,
                2.0,
                RED,
            );
        } else {
            // Sleek modern crosshair
            let gap = 4.0;
            let len = 8.0;
            let color = Color::new(0.0, 1.0, 0.2, 0.8);
            draw_line(
                center_x - gap - len,
                center_y,
                center_x - gap,
                center_y,
                2.0,
                color,
            );
            draw_line(
                center_x + gap,
                center_y,
                center_x + gap + len,
                center_y,
                2.0,
                color,
            );
            draw_line(
                center_x,
                center_y - gap - len,
                center_x,
                center_y - gap,
                2.0,
                color,
            );
            draw_line(
                center_x,
                center_y + gap,
                center_x,
                center_y + gap + len,
                2.0,
                color,
            );
            draw_circle(center_x, center_y, 1.5, WHITE);
        }

        // --- 2. Health Bar ---
        let hp_width = 200.0;
        let hp_height = 20.0;
        let hp_x = 20.0;
        let hp_y = self.height - 40.0;

        draw_rectangle(hp_x, hp_y, hp_width, hp_height, DARKGRAY);
        let hp_pct = local.health as f32 / 100.0;
        let color = match local.health {
            h if h > 60 => GREEN,
            h if h > 30 => YELLOW,
            _ => RED,
        };
        draw_rectangle(hp_x, hp_y, hp_width * hp_pct, hp_height, color);
        draw_rectangle_lines(hp_x, hp_y, hp_width, hp_height, 2.0, WHITE);

        draw_text(
            &format!("HP: {}", local.health),
            hp_x + 10.0,
            hp_y + 15.0,
            20.0,
            WHITE,
        );

        // --- 3. Minimap ---
        let mm_size = 4.0; // pixels per cell
        let mm_w = map.width as f32 * mm_size;
        // let mm_h = map.height as f32 * mm_size;
        let mm_offset_x = self.width - mm_w - 20.0;
        let mm_offset_y = 20.0;

        for y in 0..map.height {
            for x in 0..map.width {
                if map.is_wall(x as f32, y as f32) {
                    draw_rectangle(
                        mm_offset_x + x as f32 * mm_size,
                        mm_offset_y + y as f32 * mm_size,
                        mm_size,
                        mm_size,
                        DARKGRAY,
                    );
                }
            }
        }
        // Local player on minimap
        draw_circle(
            mm_offset_x + local.x * mm_size,
            mm_offset_y + local.y * mm_size,
            mm_size * 0.8,
            GREEN,
        );

        // --- 4. Kill Feed / Messages ---
        let msg_start_y = 20.0;
        for (i, msg) in messages.iter().enumerate() {
            draw_text(msg, 20.0, msg_start_y + (i as f32 * 20.0), 20.0, MAGENTA);
        }
    }
}
