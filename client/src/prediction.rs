use shared::maze::Map;
use shared::physics::apply_input;
use shared::protocol::{InputData, PlayerState, TICK_DURATION};

/// A local input that hasn't been acknowledged by the server yet.
#[derive(Debug, Clone)]
pub struct PendingInput {
    pub sequence: u32,
    pub input: InputData,
}

/// Manages client-side prediction and server reconciliation for the local player.
pub struct PredictionLocal {
    pub local_state: PlayerState,
    pub unacked_inputs: Vec<PendingInput>,
    pub latest_server_tick: u32,
    pub map: Option<Map>,
}

impl PredictionLocal {
    pub fn new() -> Self {
        Self {
            local_state: PlayerState {
                id: 0,
                x: 0.0,
                y: 0.0,
                name: [0; 24],
                angle: 0.0,
                health: 100,
                frags: 0,
                is_game_over: false,
            },
            unacked_inputs: Vec::new(),
            latest_server_tick: 0,
            map: None,
        }
    }

    /// Reset local state from a server snapshot. This is the "reconciliation" step.
    pub fn reconcile(
        &mut self,
        server_state: &PlayerState,
        server_tick: u32,
        acked_input_seq: u32,
    ) {
        if server_tick <= self.latest_server_tick {
            // Out of order or duplicate snapshot; ignore.
            return;
        }
        self.latest_server_tick = server_tick;

        // Snap to authoritative state.
        self.local_state = server_state.clone();

        // Discard inputs that the server has already processed.
        self.unacked_inputs.retain(|p| p.sequence > acked_input_seq);

        // Re-apply any remaining unacknowledged inputs.
        if let Some(map) = &self.map {
            for pending in &self.unacked_inputs {
                let (new_x, new_y, new_angle) = apply_input(
                    self.local_state.x,
                    self.local_state.y,
                    self.local_state.angle,
                    &pending.input,
                    TICK_DURATION as f32, // Apply exact duration the server uses
                    map,
                );
                self.local_state.x = new_x;
                self.local_state.y = new_y;
                self.local_state.angle = new_angle;
            }
        }
    }

    /// Apply an input locally immediately (client-side prediction) and buffer it.
    pub fn add_input(&mut self, sequence: u32, input: InputData) {
        if let Some(map) = &self.map {
            let (new_x, new_y, new_angle) = apply_input(
                self.local_state.x,
                self.local_state.y,
                self.local_state.angle,
                &input,
                TICK_DURATION as f32,
                map,
            );
            self.local_state.x = new_x;
            self.local_state.y = new_y;
            self.local_state.angle = new_angle;

            self.unacked_inputs.push(PendingInput { sequence, input });
        }
    }
}
