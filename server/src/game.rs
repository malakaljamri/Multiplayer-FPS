use std::collections::HashMap;
use std::net::SocketAddr;

use shared::maze::{Difficulty, Map};
use shared::physics::{apply_input, cast_hitscan_ray, ray_intersects_circle};
use shared::protocol::{
    InputData, PlayerState, MAX_HEALTH, TICK_DURATION, WEAPON_COOLDOWN_TICKS, WEAPON_DAMAGE,
};

/// Server-side representation of a connected player.
#[allow(dead_code)]
pub struct Player {
    pub id: u32,
    pub name: String,
    pub addr: SocketAddr,
    pub x: f32,
    pub y: f32,
    pub angle: f32,
    pub health: u8,
    /// Last input sequence processed for this player.
    pub last_input_seq: u32,
    /// The server tick when the player last shot.
    pub last_shot_tick: u32,
}

/// Authoritative game state managed by the server.
pub struct GameState {
    pub players: HashMap<u32, Player>,
    pub tick: u32,
    pub seed: u64,
    pub difficulty: Difficulty,
    pub map: Map,
    pub message_queue: Vec<String>,
}

impl GameState {
    pub fn new(seed: u64, difficulty: Difficulty) -> Self {
        let map = Map::generate(seed, difficulty);
        Self {
            players: HashMap::new(),
            tick: 0,
            seed,
            difficulty,
            map,
            message_queue: Vec::new(),
        }
    }

    /// Add a player at a spawn position. Returns the assigned player id.
    pub fn add_player(&mut self, id: u32, name: String, addr: SocketAddr) {
        // Find a valid spawn position (empty floor) near the center
        let mut spawn_x = 1.5;
        let mut spawn_y = 1.5;

        // Simple search for an empty tile
        'search: for y in 1..self.map.height.saturating_sub(1) {
            for x in 1..self.map.width.saturating_sub(1) {
                if !self.map.is_wall(x as f32 + 0.5, y as f32 + 0.5) {
                    spawn_x = x as f32 + 0.5;
                    spawn_y = y as f32 + 0.5;
                    break 'search;
                }
            }
        }

        let player = Player {
            id,
            name,
            addr,
            x: spawn_x,
            y: spawn_y,
            angle: 0.0,
            health: MAX_HEALTH,
            last_input_seq: 0,
            last_shot_tick: 0,
        };
        self.players.insert(id, player);
    }

    /// Remove a player by id. Returns `true` if the player existed.
    pub fn remove_player(&mut self, id: u32) -> bool {
        self.players.remove(&id).is_some()
    }

    /// Remove a player by their socket address. Returns the id if found.
    #[allow(dead_code)]
    pub fn remove_player_by_addr(&mut self, addr: &SocketAddr) -> Option<u32> {
        let id = self
            .players
            .values()
            .find(|p| p.addr == *addr)
            .map(|p| p.id);
        if let Some(id) = id {
            self.players.remove(&id);
        }
        id
    }

    /// Find a player id by socket address.
    #[allow(dead_code)]
    pub fn player_id_by_addr(&self, addr: &SocketAddr) -> Option<u32> {
        self.players
            .values()
            .find(|p| p.addr == *addr)
            .map(|p| p.id)
    }

    /// Process a movement input for a player.
    pub fn process_input(&mut self, player_id: u32, input: &InputData, input_seq: u32) {
        if let Some(player) = self.players.get_mut(&player_id) {
            // Only process inputs newer than what we've already seen.
            if input_seq <= player.last_input_seq && player.last_input_seq > 0 {
                return;
            }
            player.last_input_seq = input_seq;

            let (new_x, new_y, new_angle) = apply_input(
                player.x,
                player.y,
                player.angle,
                input,
                TICK_DURATION as f32,
                &self.map,
            );
            player.x = new_x;
            player.y = new_y;
            player.angle = new_angle;

            // Check if shooting is allowed
            if input.shoot && self.tick >= player.last_shot_tick + WEAPON_COOLDOWN_TICKS {
                player.last_shot_tick = self.tick;

                // Hitscan parameters
                let shooter_x = player.x;
                let shooter_y = player.y;
                let shooter_angle = player.angle;
                let shooter_name = player.name.clone();
                let shooter_dx = shooter_angle.cos();
                let shooter_dy = shooter_angle.sin();

                // Drop mutable borrow to iterate over other players
                let max_dist = cast_hitscan_ray(shooter_x, shooter_y, shooter_angle, &self.map);

                let mut hit_target_id = None;
                let mut min_hit_dist = max_dist;

                // Find closest hit player
                for (&other_id, other) in &self.players {
                    if other_id == player_id || other.health == 0 {
                        continue;
                    }
                    if ray_intersects_circle(
                        shooter_x, shooter_y, shooter_dx, shooter_dy, other.x, other.y,
                        0.3, // Simple player radius
                    ) {
                        let dist =
                            ((other.x - shooter_x).powi(2) + (other.y - shooter_y).powi(2)).sqrt();
                        if dist < min_hit_dist {
                            min_hit_dist = dist;
                            hit_target_id = Some(other_id);
                        }
                    }
                }

                // Apply damage if a player was hit
                if let Some(target_id) = hit_target_id {
                    let mut is_dead = false;
                    let mut target_name = String::new();

                    if let Some(target) = self.players.get_mut(&target_id) {
                        target_name = target.name.clone();
                        target.health = target.health.saturating_sub(WEAPON_DAMAGE);

                        if target.health == 0 {
                            is_dead = true;
                            target.health = MAX_HEALTH; // Respawn

                            // Randomize coordinates safely
                            let mut spawn_x = 1.5;
                            let mut spawn_y = 1.5;
                            'find_spawn: for y in 1..self.map.height.saturating_sub(1) {
                                for x in 1..self.map.width.saturating_sub(1) {
                                    if !self.map.is_wall(x as f32 + 0.5, y as f32 + 0.5) {
                                        let dist_from_shooter = ((x as f32 + 0.5 - shooter_x)
                                            .powi(2)
                                            + (y as f32 + 0.5 - shooter_y).powi(2))
                                        .sqrt();
                                        if dist_from_shooter > 5.0 {
                                            spawn_x = x as f32 + 0.5;
                                            spawn_y = y as f32 + 0.5;
                                            break 'find_spawn;
                                        }
                                    }
                                }
                            }
                            target.x = spawn_x;
                            target.y = spawn_y;
                        }
                    }

                    if is_dead {
                        self.message_queue
                            .push(format!("{} fragged {}!", shooter_name, target_name));
                    }
                }
            }
        }
    }

    /// Advance the tick counter.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Build a snapshot of all players for broadcast.
    pub fn snapshot(&self) -> Vec<PlayerState> {
        self.players
            .values()
            .map(|p| PlayerState {
                id: p.id,
                x: p.x,
                y: p.y,
                angle: p.angle,
                health: p.health,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_addr() -> SocketAddr {
        "127.0.0.1:9999".parse().unwrap()
    }

    #[test]
    fn add_and_remove_player() {
        let mut gs = GameState::new(1, Difficulty::Easy);
        gs.add_player(1, "Alice".into(), dummy_addr());
        assert_eq!(gs.players.len(), 1);
        assert!(gs.remove_player(1));
        assert_eq!(gs.players.len(), 0);
    }

    #[test]
    fn process_forward_input() {
        let mut gs = GameState::new(1, Difficulty::Easy);
        let addr = dummy_addr();
        gs.add_player(1, "Bob".into(), addr);

        let start_x = gs.players[&1].x;
        let start_y = gs.players[&1].y;

        // Player faces angle 0 → forward = +x direction.
        let input = InputData {
            forward: true,
            ..Default::default()
        };
        gs.process_input(1, &input, 1);

        let p = &gs.players[&1];
        assert!(p.x > start_x, "player should move forward in x");
        // y should stay roughly the same (angle=0 → sin(0)=0)
        assert!((p.y - start_y).abs() < 0.001);
    }

    #[test]
    fn process_turn_input() {
        let mut gs = GameState::new(1, Difficulty::Easy);
        gs.add_player(1, "Carol".into(), dummy_addr());

        let start_angle = gs.players[&1].angle;
        let input = InputData {
            turn_right: true,
            ..Default::default()
        };
        gs.process_input(1, &input, 1);

        assert!(gs.players[&1].angle > start_angle);
    }

    #[test]
    fn snapshot_contains_all_players() {
        let mut gs = GameState::new(1, Difficulty::Easy);
        gs.add_player(1, "A".into(), "127.0.0.1:1001".parse().unwrap());
        gs.add_player(2, "B".into(), "127.0.0.1:1002".parse().unwrap());
        let snap = gs.snapshot();
        assert_eq!(snap.len(), 2);
    }

    #[test]
    fn duplicate_input_seq_ignored() {
        let mut gs = GameState::new(1, Difficulty::Easy);
        gs.add_player(1, "Dup".into(), dummy_addr());

        let input = InputData {
            forward: true,
            ..Default::default()
        };
        gs.process_input(1, &input, 1);
        let x_after_first = gs.players[&1].x;

        // Same sequence again — should be ignored.
        gs.process_input(1, &input, 1);
        assert_eq!(gs.players[&1].x, x_after_first);
    }
}
