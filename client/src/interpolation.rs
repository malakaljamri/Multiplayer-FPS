use std::collections::HashMap;
use std::time::{Duration, Instant};

use shared::protocol::PlayerState;

/// A snapshot of all remote players at a specific server tick.
#[derive(Debug, Clone)]
pub struct SnapshotNode {
    pub tick: u32,
    pub timestamp: Instant,
    pub players: HashMap<u32, PlayerState>,
}

/// Manages interpolation of remote players over time to smooth out network stutters.
pub struct InterpolationRemote {
    pub interpolation_delay: Duration,
    /// Ordered list of snapshots, newest at the back.
    history: Vec<SnapshotNode>,
}

impl InterpolationRemote {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            interpolation_delay: Duration::from_millis(delay_ms),
            history: Vec::new(),
        }
    }

    /// Push a new server snapshot into the history buffer.
    pub fn push_snapshot(&mut self, tick: u32, players: Vec<PlayerState>, local_id: u32) {
        // Discard older snapshots
        if let Some(last) = self.history.last() {
            if tick <= last.tick {
                return;
            }
        }

        let mut map = HashMap::new();
        for p in players {
            if p.id != local_id {
                map.insert(p.id, p);
            }
        }

        self.history.push(SnapshotNode {
            tick,
            timestamp: Instant::now(),
            players: map,
        });

        // Keep history reasonably small (e.g. 10 snapshots ~ 166ms at 60Hz)
        if self.history.len() > 20 {
            self.history.remove(0);
        }
    }

    /// Generate an interpolated view of all remote players for the current time.
    pub fn get_interpolated_state(&self) -> Vec<PlayerState> {
        let now = Instant::now();
        let render_time = match now.checked_sub(self.interpolation_delay) {
            Some(t) => t,
            None => return Vec::new(),
        };

        // Find the two snapshots bracketing render_time
        let mut idx0 = None;
        let mut idx1 = None;

        for i in 0..self.history.len() {
            if self.history[i].timestamp <= render_time {
                idx0 = Some(i);
            } else if self.history[i].timestamp > render_time {
                idx1 = Some(i);
                break;
            }
        }

        match (idx0, idx1) {
            // We have a perfect bracket
            (Some(i0), Some(i1)) => {
                let s0 = &self.history[i0];
                let s1 = &self.history[i1];

                let t0 = s0.timestamp;
                let t1 = s1.timestamp;

                // Calculate LERP fraction (must be 0.0..=1.0)
                let dt = t1.duration_since(t0).as_secs_f32();
                if dt <= 0.0 {
                    return s0.players.values().cloned().collect();
                }

                let t = render_time.duration_since(t0).as_secs_f32() / dt;
                let t = t.clamp(0.0, 1.0);

                let mut result = Vec::new();
                for (id, p0) in &s0.players {
                    if let Some(p1) = s1.players.get(id) {
                        result.push(Self::lerp_player(p0, p1, t));
                    } else {
                        // Player disappeared in next frame
                        result.push(p0.clone());
                    }
                }
                result
            }
            // Only newer states exist (we are severely lagging behind processing)
            (None, Some(i1)) => self.history[i1].players.values().cloned().collect(),
            // Only older states exist (extrapolation case)
            (Some(i0), None) => {
                // For simplicity, just snap to the latest available state instead of extrapolating.
                self.history[i0].players.values().cloned().collect()
            }
            (None, None) => Vec::new(),
        }
    }

    /// Linear interpolation between two player states.
    fn lerp_player(p0: &PlayerState, p1: &PlayerState, t: f32) -> PlayerState {
        // Angle interpolation must take the shortest path around the circle
        let mut angle_diff = p1.angle - p0.angle;
        while angle_diff < -std::f32::consts::PI {
            angle_diff += std::f32::consts::TAU;
        }
        while angle_diff > std::f32::consts::PI {
            angle_diff -= std::f32::consts::TAU;
        }

        PlayerState {
            id: p0.id,
            x: p0.x + (p1.x - p0.x) * t,
            y: p0.y + (p1.y - p0.y) * t,
            name: p0.name,
            angle: p0.angle + angle_diff * t,
            health: p0.health, // Don't interpolate discrete values
            frags: p0.frags,
            is_game_over: p0.is_game_over, // Don't interpolate discrete values
        }
    }
}
