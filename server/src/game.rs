use std::collections::HashMap;
use std::net::SocketAddr;

use shared::maze::{Difficulty, Map};
use shared::physics::{apply_input, cast_hitscan_ray, ray_intersects_circle};
use shared::protocol::{
    InputData, PlayerState, MAX_HEALTH, TICK_DURATION, WEAPON_COOLDOWN_TICKS, WEAPON_DAMAGE,
};

/// Frags needed to advance from each level.
const FRAGS_TO_ADVANCE: [u32; 3] = [5, 10, 15];

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
    pub frags: u32,
    /// Last input sequence processed for this player.
    pub last_input_seq: u32,
    /// The server tick when the player last shot.
    pub last_shot_tick: u32,
    /// Whether the player is in game over state (dead and needs to respawn).
    pub is_game_over: bool,
}

/// Pending level change information.
pub struct LevelChangePending {
    pub seed: u64,
    pub difficulty: Difficulty,
    pub level: u8,
}

/// Authoritative game state managed by the server.
pub struct GameState {
    pub players: HashMap<u32, Player>,
    pub tick: u32,
    pub seed: u64,
    pub difficulty: Difficulty,
    pub map: Map,
    pub message_queue: Vec<String>,
    /// Current level (1-based: 1, 2, 3).
    pub level: u8,
    /// Total frags across all players since last level change.
    pub total_frags: u32,
    /// Set when a level advance is triggered; cleared by the server loop.
    pub pending_level_change: Option<LevelChangePending>,
}

impl GameState {
    fn collides_with_other_player(
        others: &[(f32, f32)],
        x: f32,
        y: f32,
        player_radius: f32,
    ) -> bool {
        let min_dist_sq = (player_radius * 2.0).powi(2);
        others.iter().any(|(ox, oy)| {
            let dx = x - *ox;
            let dy = y - *oy;
            (dx * dx + dy * dy) < min_dist_sq
        })
    }

    pub fn new(seed: u64, difficulty: Difficulty) -> Self {
        let map = Map::generate(seed, difficulty);
        Self {
            players: HashMap::new(),
            tick: 0,
            seed,
            difficulty,
            map,
            message_queue: Vec::new(),
            level: 1,
            total_frags: 0,
            pending_level_change: None,
        }
    }

    /// Add a player at a spawn position.
    pub fn add_player(&mut self, id: u32, name: String, addr: SocketAddr) {
        let spawn_offset = self.players.len();
        let (spawn_x, spawn_y) = self.find_spawn_nth(spawn_offset);
        let player = Player {
            id,
            name,
            addr,
            x: spawn_x,
            y: spawn_y,
            angle: 0.0,
            health: MAX_HEALTH,
            frags: 0,
            last_input_seq: 0,
            last_shot_tick: 0,
            is_game_over: false,
        };
        self.players.insert(id, player);
    }

    /// Remove a player by id.
    pub fn remove_player(&mut self, id: u32) -> bool {
        self.players.remove(&id).is_some()
    }

    /// Respawn a player who was in game over state.
    pub fn respawn_player(&mut self, player_id: u32) -> bool {
        // Check if player is in game over state first
        let is_game_over = self.players.get(&player_id).map(|p| p.is_game_over).unwrap_or(false);
        if !is_game_over {
            return false;
        }

        let (spawn_x, spawn_y) = self.find_spawn(None);

        if let Some(player) = self.players.get_mut(&player_id) {
            player.is_game_over = false;
            player.health = MAX_HEALTH;
            player.x = spawn_x;
            player.y = spawn_y;
            player.angle = 0.0;
            true
        } else {
            false
        }
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
        const PLAYER_RADIUS: f32 = 0.2;

        let other_positions: Vec<(f32, f32)> = self
            .players
            .iter()
            .filter(|(id, other)| **id != player_id && other.health > 0)
            .map(|(_, other)| (other.x, other.y))
            .collect();

        if let Some(player) = self.players.get_mut(&player_id) {
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

            // Prevent players from pushing each other by blocking overlap.
            let mut final_x = player.x;
            let mut final_y = player.y;

            if !Self::collides_with_other_player(&other_positions, new_x, player.y, PLAYER_RADIUS) {
                final_x = new_x;
            }
            if !Self::collides_with_other_player(&other_positions, final_x, new_y, PLAYER_RADIUS) {
                final_y = new_y;
            }

            player.x = final_x;
            player.y = final_y;
            player.angle = new_angle;

            if input.shoot && self.tick >= player.last_shot_tick + WEAPON_COOLDOWN_TICKS {
                player.last_shot_tick = self.tick;

                let shooter_x = player.x;
                let shooter_y = player.y;
                let shooter_angle = player.angle;
                let shooter_name = player.name.clone();
                let shooter_id = player.id;
                let shooter_dx = shooter_angle.cos();
                let shooter_dy = shooter_angle.sin();

                let max_dist = cast_hitscan_ray(shooter_x, shooter_y, shooter_angle, &self.map);

                let mut hit_target_id = None;
                let mut min_hit_dist = max_dist;

                for (&other_id, other) in &self.players {
                    if other_id == player_id || other.health == 0 {
                        continue;
                    }
                    if ray_intersects_circle(
                        shooter_x, shooter_y, shooter_dx, shooter_dy, other.x, other.y,
                        0.3,
                    ) {
                        let dist =
                            ((other.x - shooter_x).powi(2) + (other.y - shooter_y).powi(2))
                                .sqrt();
                        if dist < min_hit_dist {
                            min_hit_dist = dist;
                            hit_target_id = Some(other_id);
                        }
                    }
                }

                if let Some(target_id) = hit_target_id {
                    let mut is_dead = false;
                    let mut target_name = String::new();

                    if let Some(target) = self.players.get_mut(&target_id) {
                        target_name = target.name.clone();
                        target.health = target.health.saturating_sub(WEAPON_DAMAGE);

                        if target.health == 0 {
                            is_dead = true;
                            target.is_game_over = true;
                        }
                    }

                    if is_dead {
                        // Award frag to shooter
                        if let Some(shooter) = self.players.get_mut(&shooter_id) {
                            shooter.frags += 1;
                        }
                        self.total_frags += 1;

                        let frag_count = self
                            .players
                            .get(&shooter_id)
                            .map(|p| p.frags)
                            .unwrap_or(0);
                        self.message_queue.push(format!(
                            "{} fragged {}! ({} kills)",
                            shooter_name, target_name, frag_count
                        ));

                        // Check if only one player is alive (last man standing)
                        let alive_players: Vec<&Player> = self.players
                            .values()
                            .filter(|p| !p.is_game_over)
                            .collect();
                        
                        if alive_players.len() == 1 {
                            let winner = &alive_players[0];
                            self.message_queue.push(format!(
                                "🎉 CONGRATULATIONS! {} is the last one standing! 🎉",
                                winner.name
                            ));
                        }

                        // Check level advancement
                        let threshold = FRAGS_TO_ADVANCE
                            .get(self.level as usize - 1)
                            .copied()
                            .unwrap_or(u32::MAX);
                        if self.total_frags >= threshold && self.pending_level_change.is_none() {
                            self.trigger_level_advance();
                        }
                    }
                }
            }
        }
    }

    /// Triggers a level advance: picks the next difficulty and generates new map info.
    fn trigger_level_advance(&mut self) {
        let next_level = (self.level + 1).min(3);
        let next_difficulty = match next_level {
            1 => Difficulty::Easy,
            2 => Difficulty::Medium,
            _ => Difficulty::Hard,
        };

        // Generate a new random-ish seed from current tick
        let new_seed = self.seed.wrapping_add(self.tick as u64 * 6364136223846793005 + 1442695040888963407);

        self.message_queue.push(format!(
            "=== LEVEL {} START! Maze grows harder... ===",
            next_level
        ));

        self.pending_level_change = Some(LevelChangePending {
            seed: new_seed,
            difficulty: next_difficulty,
            level: next_level,
        });

        // Apply the level change immediately to the server's map
        self.seed = new_seed;
        self.difficulty = next_difficulty;
        self.level = next_level;
        self.total_frags = 0;
        self.map = Map::generate(new_seed, next_difficulty);

        // Respawn everyone in the new map
        let player_ids: Vec<u32> = self.players.keys().copied().collect();
        let mut spawn_offset = 0;
        for id in player_ids {
            let (sx, sy) = self.find_spawn_nth(spawn_offset);
            spawn_offset += 1;
            if let Some(p) = self.players.get_mut(&id) {
                p.x = sx;
                p.y = sy;
                p.health = MAX_HEALTH;
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
                is_game_over: p.is_game_over,
            })
            .collect()
    }

    /// Find an empty spawn tile far from `avoid` if provided.
    fn find_spawn(&self, avoid: Option<(f32, f32)>) -> (f32, f32) {
        if !self.map.spawn_points.is_empty() {
            if let Some((ax, ay)) = avoid {
                if let Some(&(sx, sy)) = self
                    .map
                    .spawn_points
                    .iter()
                    .find(|(sx, sy)| ((*sx - ax).powi(2) + (*sy - ay).powi(2)).sqrt() > 5.0)
                {
                    return (sx, sy);
                }
            }
            return self.map.spawn_points[0];
        }

        let mut best = (1.5f32, 1.5f32);
        for y in 1..self.map.height.saturating_sub(1) {
            for x in 1..self.map.width.saturating_sub(1) {
                let cx = x as f32 + 0.5;
                let cy = y as f32 + 0.5;
                if !self.map.is_wall(cx, cy) {
                    if let Some((ax, ay)) = avoid {
                        let d = ((cx - ax).powi(2) + (cy - ay).powi(2)).sqrt();
                        if d > 5.0 {
                            return (cx, cy);
                        }
                    } else {
                        return (cx, cy);
                    }
                    best = (cx, cy);
                }
            }
        }
        best
    }

    /// Find the Nth empty floor tile (wraps around for multiple players).
    fn find_spawn_nth(&self, n: usize) -> (f32, f32) {
        if !self.map.spawn_points.is_empty() {
            return self.map.spawn_points[n % self.map.spawn_points.len()];
        }

        let mut count = 0;
        for y in 1..self.map.height.saturating_sub(1) {
            for x in 1..self.map.width.saturating_sub(1) {
                let cx = x as f32 + 0.5;
                let cy = y as f32 + 0.5;
                if !self.map.is_wall(cx, cy) {
                    if count == n % (self.map.width * self.map.height) {
                        return (cx, cy);
                    }
                    count += 1;
                }
            }
        }
        (1.5, 1.5)
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

        let input = InputData {
            forward: true,
            ..Default::default()
        };
        gs.process_input(1, &input, 1);

        let p = &gs.players[&1];
        assert!(p.x > start_x, "player should move forward in x");
        assert!((p.y - start_y).abs() < 0.001);
    }

    #[test]
    fn process_turn_input() {
        let mut gs = GameState::new(1, Difficulty::Easy);
        gs.add_player(1, "Carol".into(), dummy_addr());

        let start_angle = gs.players[&1].angle;
        // Positive turn_delta rotates the player clockwise (increasing angle)
        let input = InputData {
            turn_delta: 0.2,
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

        gs.process_input(1, &input, 1);
        assert_eq!(gs.players[&1].x, x_after_first);
    }

    #[test]
    fn level_starts_at_one() {
        let gs = GameState::new(42, Difficulty::Easy);
        assert_eq!(gs.level, 1);
        assert_eq!(gs.total_frags, 0);
    }
}
