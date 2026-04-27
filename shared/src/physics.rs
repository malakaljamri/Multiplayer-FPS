use crate::maze::Map;
use crate::protocol::{InputData, MOVE_SPEED};

/// Applies movement and turning input to an entity's position and rotation over a time delta.
/// Returns the new `(x, y, angle)`.
/// Both server and client use this to ensure prediction is identical to server authoritative logic.
pub fn apply_input(
    curr_x: f32,
    curr_y: f32,
    curr_angle: f32,
    input: &InputData,
    dt: f32,
    map: &Map,
) -> (f32, f32, f32) {
    // Mouse-driven rotation (turn_delta is a raw radian delta, not scaled by dt)
    let new_angle = curr_angle + input.turn_delta;

    // Movement along facing direction
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;
    if input.forward {
        dx += new_angle.cos() * MOVE_SPEED * dt;
        dy += new_angle.sin() * MOVE_SPEED * dt;
    }
    if input.backward {
        dx -= new_angle.cos() * MOVE_SPEED * dt;
        dy -= new_angle.sin() * MOVE_SPEED * dt;
    }
    // Strafe: perpendicular to facing direction.
    // Right perp = (-sin θ, cos θ), left = (sin θ, -cos θ)
    if input.strafe_right {
        dx += -new_angle.sin() * MOVE_SPEED * dt;
        dy += new_angle.cos() * MOVE_SPEED * dt;
    }
    if input.strafe_left {
        dx += new_angle.sin() * MOVE_SPEED * dt;
        dy += -new_angle.cos() * MOVE_SPEED * dt;
    }

    // A player's collision radius
    const RADIUS: f32 = 0.2;

    let mut new_x = curr_x;
    let mut new_y = curr_y;

    // Slide collision on X axis
    if dx.abs() > 1e-6 && !map.is_wall(curr_x + dx + RADIUS.copysign(dx), curr_y) {
        new_x += dx;
    }

    // Slide collision on Y axis
    if dy.abs() > 1e-6 && !map.is_wall(curr_x, curr_y + dy + RADIUS.copysign(dy)) {
        new_y += dy;
    }

    (new_x, new_y, new_angle)
}

/// Casts a ray from (px, py) at `angle` until it hits a wall on the `map`.
/// Returns the distance to the wall. Uses the DDA algorithm.
pub fn cast_hitscan_ray(px: f32, py: f32, angle: f32, map: &Map) -> f32 {
    let rdx = angle.cos();
    let rdy = angle.sin();

    let mut map_x = px as i32;
    let mut map_y = py as i32;

    let delta_dist_x = if rdx == 0.0 { 1e30 } else { (1.0 / rdx).abs() };
    let delta_dist_y = if rdy == 0.0 { 1e30 } else { (1.0 / rdy).abs() };

    let step_x: i32;
    let step_y: i32;
    let mut side_dist_x: f32;
    let mut side_dist_y: f32;

    if rdx < 0.0 {
        step_x = -1;
        side_dist_x = (px - map_x as f32) * delta_dist_x;
    } else {
        step_x = 1;
        side_dist_x = (map_x as f32 + 1.0 - px) * delta_dist_x;
    }

    if rdy < 0.0 {
        step_y = -1;
        side_dist_y = (py - map_y as f32) * delta_dist_y;
    } else {
        step_y = 1;
        side_dist_y = (map_y as f32 + 1.0 - py) * delta_dist_y;
    }

    let mut side = 0;

    for _ in 0..100 {
        // Max distance 100 tiles
        if side_dist_x < side_dist_y {
            side_dist_x += delta_dist_x;
            map_x += step_x;
            side = 0;
        } else {
            side_dist_y += delta_dist_y;
            map_y += step_y;
            side = 1;
        }

        if map.is_wall(map_x as f32, map_y as f32) {
            break;
        }
    }

    if side == 0 {
        (map_x as f32 - px + (1 - step_x) as f32 / 2.0) / rdx
    } else {
        (map_y as f32 - py + (1 - step_y) as f32 / 2.0) / rdy
    }
}

/// Casts a ray and returns info about the wall hit:
/// `(cell_x, cell_y, hit_y_side, wall_frac)` where `wall_frac` is 0..1
/// along the face of the struck wall tile.  Returns `None` if no wall is hit.
pub fn cast_ray_hit(px: f32, py: f32, angle: f32, map: &Map) -> Option<(i32, i32, bool, f32)> {
    let rdx = angle.cos();
    let rdy = angle.sin();

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

    let mut hit_y;

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
            // Perpendicular distance to the hit wall face
            let perp = if !hit_y {
                (mx as f32 - px + (1 - step_x) as f32 * 0.5) / rdx
            } else {
                (my as f32 - py + (1 - step_y) as f32 * 0.5) / rdy
            };
            // Fractional position (0..1) along the struck wall face
            let wall_frac = if !hit_y {
                let hit = py + perp * rdy;
                hit - hit.floor()
            } else {
                let hit = px + perp * rdx;
                hit - hit.floor()
            };
            return Some((mx, my, hit_y, wall_frac));
        }
    }
    None
}

/// Checks if a ray starting at `(px, py)` extending to `(px + dx, py + dy)`
/// intersects a circle of `radius` at `(cx, cy)`.
pub fn ray_intersects_circle(
    px: f32,
    py: f32,
    dx: f32,
    dy: f32,
    cx: f32,
    cy: f32,
    radius: f32,
) -> bool {
    let t_closest = ((cx - px) * dx + (cy - py) * dy) / (dx * dx + dy * dy);

    if t_closest < 0.0 {
        return false;
    }

    let proj_x = px + t_closest * dx;
    let proj_y = py + t_closest * dy;
    let dist_sq = (cx - proj_x).powi(2) + (cy - proj_y).powi(2);

    dist_sq <= radius * radius
}
