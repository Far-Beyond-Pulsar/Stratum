use std::time::Duration;

/// Telemetry and performance metrics for world partition streaming.
pub use crate::world_partition::WorldPartitionMetrics;

impl WorldPartitionMetrics {
    /// Get average chunks loaded per second.
    pub fn chunks_loaded_per_second(&self) -> f64 {
        if self.frames == 0 {
            return 0.0;
        }
        let seconds = self.frames as f64 / 60.0; // Assuming 60 FPS
        self.chunks_loaded as f64 / seconds
    }

    /// Get average chunks evicted per second.
    pub fn chunks_evicted_per_second(&self) -> f64 {
        if self.frames == 0 {
            return 0.0;
        }
        let seconds = self.frames as f64 / 60.0;
        self.chunks_evicted as f64 / seconds
    }

    /// Get failure rate as a percentage.
    pub fn failure_rate_percent(&self) -> f64 {
        let total_attempts = self.chunks_loaded + self.chunks_failed;
        if total_attempts == 0 {
            return 0.0;
        }
        (self.chunks_failed as f64 / total_attempts as f64) * 100.0
    }

    /// Format metrics as a human-readable string.
    pub fn format_summary(&self) -> String {
        format!(
            "Frames: {} | Loaded: {} | Evicted: {} | Failed: {} | Pending: {} | Last Commit: {:.2}ms",
            self.frames,
            self.chunks_loaded,
            self.chunks_evicted,
            self.chunks_failed,
            self.pending_load_tasks,
            self.last_commit_ms.as_secs_f64() * 1000.0
        )
    }

    /// Reset all metrics to zero.
    pub fn reset(&mut self) {
        self.frames = 0;
        self.chunks_loaded = 0;
        self.chunks_evicted = 0;
        self.chunks_failed = 0;
        self.pending_load_tasks = 0;
        self.last_commit_ms = Duration::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_calculations() {
        let mut metrics = WorldPartitionMetrics::default();
        metrics.frames = 60;
        metrics.chunks_loaded = 10;
        metrics.chunks_failed = 2;

        assert!(metrics.chunks_loaded_per_second() > 0.0);
        assert!(metrics.failure_rate_percent() > 0.0);
        assert!(metrics.failure_rate_percent() < 100.0);
    }

    #[test]
    fn test_metrics_reset() {
        let mut metrics = WorldPartitionMetrics::default();
        metrics.chunks_loaded = 100;
        metrics.reset();
        assert_eq!(metrics.chunks_loaded, 0);
    }
}
