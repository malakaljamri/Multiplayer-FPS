use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    /// Returns the (width, height) for the maze grid based on difficulty.
    /// Must be odd numbers for the recursive backtracker to work properly.
    pub fn dimensions(&self) -> (usize, usize) {
        match self {
            Difficulty::Easy => (21, 21),
            Difficulty::Medium => (31, 31),
            Difficulty::Hard => (41, 41),
        }
    }
}

/// A 2D grid representing the game map.
/// `true` means solid wall, `false` means empty floor.
#[derive(Debug, Clone)]
pub struct Map {
    pub width: usize,
    pub height: usize,
    pub grid: Vec<bool>,
    /// Precomputed spawn positions (centered in floor tiles).
    pub spawn_points: Vec<(f32, f32)>,
}

impl Map {
    fn compute_spawn_points(grid: &[bool], width: usize, height: usize, desired: usize) -> Vec<(f32, f32)> {
        let mut spawns = Vec::new();

        // Spread candidates across the map first for better distribution.
        let x_positions = [1usize, width / 4, width / 2, (width * 3) / 4, width.saturating_sub(2)];
        let y_positions = [1usize, height / 4, height / 2, (height * 3) / 4, height.saturating_sub(2)];

        for &y in &y_positions {
            for &x in &x_positions {
                if x > 0 && y > 0 && x < width - 1 && y < height - 1 && !grid[y * width + x] {
                    let pt = (x as f32 + 0.5, y as f32 + 0.5);
                    if !spawns.contains(&pt) {
                        spawns.push(pt);
                        if spawns.len() >= desired {
                            return spawns;
                        }
                    }
                }
            }
        }

        // Fallback: scan all floor tiles to ensure we always reach desired count if possible.
        for y in 1..height.saturating_sub(1) {
            for x in 1..width.saturating_sub(1) {
                if !grid[y * width + x] {
                    let pt = (x as f32 + 0.5, y as f32 + 0.5);
                    if !spawns.contains(&pt) {
                        spawns.push(pt);
                        if spawns.len() >= desired {
                            return spawns;
                        }
                    }
                }
            }
        }

        spawns
    }

    /// Generates a new maze using a deterministic seed and the Recursive Backtracker algorithm.
    pub fn generate(seed: u64, difficulty: Difficulty) -> Self {
        let (w, h) = difficulty.dimensions();
        // Initialize everything as a wall
        let mut grid = vec![true; w * h];

        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        // Directions for moving 2 cells at a time (up, right, down, left)
        let dirs: [(isize, isize); 4] = [(0, -2), (2, 0), (0, 2), (-2, 0)];

        // Start carving from (1, 1)
        let start_x = 1isize;
        let start_y = 1isize;

        grid[(start_y as usize) * w + (start_x as usize)] = false;

        // Stack holds (x, y) coordinates
        let mut stack = vec![(start_x, start_y)];

        while let Some(&(x, y)) = stack.last() {
            // Find valid uncarved neighbors
            let mut neighbors = Vec::new();
            for (dx, dy) in &dirs {
                let nx = x + dx;
                let ny = y + dy;

                if nx > 0 && nx < (w as isize - 1) && ny > 0 && ny < (h as isize - 1) {
                    let idx = (ny as usize) * w + (nx as usize);
                    if grid[idx] {
                        // if it's still a wall
                        neighbors.push((nx, ny, dx, dy));
                    }
                }
            }

            if neighbors.is_empty() {
                stack.pop();
            } else {
                // Pick a random neighbor
                let idx = rng.gen_range(0..neighbors.len());
                let (nx, ny, _dx, _dy) = neighbors[idx];

                // Carve through the wall in between
                let mid_x = (x + nx) / 2;
                let mid_y = (y + ny) / 2;
                grid[(mid_y as usize) * w + (mid_x as usize)] = false;

                // Carve the destination cell
                grid[(ny as usize) * w + (nx as usize)] = false;

                stack.push((nx, ny));
            }
        }

        // Add loops so the maze is more connected (fewer dead-end-only paths).
        let loop_chance = match difficulty {
            Difficulty::Easy => 0.20,
            Difficulty::Medium => 0.16,
            Difficulty::Hard => 0.12,
        };
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                if grid[y * w + x] && rng.gen_bool(loop_chance) {
                    grid[y * w + x] = false;
                }
            }
        }

        // Widen corridors/open spaces by carving around existing floors.
        let widen_chance = match difficulty {
            Difficulty::Easy => 0.28,
            Difficulty::Medium => 0.23,
            Difficulty::Hard => 0.18,
        };
        let mut widened = grid.clone();
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                if !grid[y * w + x] {
                    if grid[y * w + (x - 1)] && rng.gen_bool(widen_chance) {
                        widened[y * w + (x - 1)] = false;
                    }
                    if grid[y * w + (x + 1)] && rng.gen_bool(widen_chance) {
                        widened[y * w + (x + 1)] = false;
                    }
                    if grid[(y - 1) * w + x] && rng.gen_bool(widen_chance) {
                        widened[(y - 1) * w + x] = false;
                    }
                    if grid[(y + 1) * w + x] && rng.gen_bool(widen_chance) {
                        widened[(y + 1) * w + x] = false;
                    }
                }
            }
        }
        grid = widened;

        let spawn_points = Self::compute_spawn_points(&grid, w, h, 7);

        Self {
            width: w,
            height: h,
            grid,
            spawn_points,
        }
    }

    /// Check if a given map coordinate is a wall.
    /// Safely handles out-of-bounds queries by treating them as walls.
    pub fn is_wall(&self, x: f32, y: f32) -> bool {
        if x < 0.0 || y < 0.0 {
            return true;
        }

        let ix = x as usize;
        let iy = y as usize;

        if ix >= self.width || iy >= self.height {
            return true;
        }

        self.grid[iy * self.width + ix]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_generation() {
        let m1 = Map::generate(42, Difficulty::Easy);
        let m2 = Map::generate(42, Difficulty::Easy);
        assert_eq!(m1.grid, m2.grid);
    }

    #[test]
    fn dimensions_easy() {
        let m = Map::generate(1, Difficulty::Easy);
        assert_eq!(m.width, 21);
        assert_eq!(m.height, 21);
        assert_eq!(m.grid.len(), 21 * 21);
    }

    #[test]
    fn has_spawn_points() {
        let m = Map::generate(7, Difficulty::Easy);
        assert!(
            m.spawn_points.len() >= 7,
            "expected at least 7 spawn points, got {}",
            m.spawn_points.len()
        );
    }

    #[test]
    fn bounds_checking() {
        let m = Map::generate(1, Difficulty::Easy);
        // Map is enclosed by walls, so (0,0) and perimeter should be true (wall)
        assert!(m.is_wall(0.0, 0.0));
        assert!(m.is_wall((m.width as f32) - 0.1, 0.0));
        assert!(m.is_wall(-1.0, 5.0)); // Out of bounds = wall
        assert!(m.is_wall(99.0, 99.0));

        // Start carve position is always explicitly carved out
        assert!(!m.is_wall(1.5, 1.5));
    }
}
