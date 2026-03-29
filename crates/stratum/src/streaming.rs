use std::time::Duration;

/// Streaming budget constraints for controlling load performance.
pub use crate::world_partition::StreamingBudget;

/// Streaming policy for determining which chunks to load/unload.
#[derive(Debug, Clone)]
pub struct StreamingPolicy {
    /// Minimum time between LOD evaluations (ms).
    pub min_eval_interval_ms: u64,
    /// Distance scale factor for screen-space size calculation.
    pub distance_scale: f32,
    /// Hysteresis factor to prevent LOD thrashing (0.0-1.0).
    pub lod_hysteresis: f32,
    /// Enable predictive loading based on camera velocity.
    pub predictive_loading: bool,
    /// Time ahead to predict for preloading (seconds).
    pub prediction_time: f32,
}

impl Default for StreamingPolicy {
    fn default() -> Self {
        Self {
            min_eval_interval_ms: 16, // ~60 FPS
            distance_scale: 1.0,
            lod_hysteresis: 0.1,
            predictive_loading: true,
            prediction_time: 2.0, // 2 seconds ahead
        }
    }
}

impl StreamingPolicy {
    /// Create a conservative policy for low-end hardware.
    pub fn conservative() -> Self {
        Self {
            min_eval_interval_ms: 33, // ~30 FPS
            distance_scale: 0.8,
            lod_hysteresis: 0.2,
            predictive_loading: false,
            prediction_time: 1.0,
        }
    }

    /// Create an aggressive policy for high-end hardware.
    pub fn aggressive() -> Self {
        Self {
            min_eval_interval_ms: 8, // ~120 FPS
            distance_scale: 1.2,
            lod_hysteresis: 0.05,
            predictive_loading: true,
            prediction_time: 3.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_policy_presets() {
        let conservative = StreamingPolicy::conservative();
        assert_eq!(conservative.min_eval_interval_ms, 33);

        let aggressive = StreamingPolicy::aggressive();
        assert_eq!(aggressive.min_eval_interval_ms, 8);
        assert!(aggressive.predictive_loading);
    }
}
