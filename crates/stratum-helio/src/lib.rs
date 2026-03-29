//! Stratum-Helio Integration Bridge
//!
//! This crate provides the integration layer between Stratum's world partition
//! system and the Helio renderer. It handles:
//!
//! - Converting Stratum chunks to Helio Scene objects
//! - Managing Helio resource handles (meshes, materials, textures)
//! - Committing chunk visibility changes to the Scene
//! - Group-based chunk management for efficient culling
//! - Wrapping Helio renderer so users don't touch it directly

pub mod bridge;
pub mod asset_registry;
pub mod renderer;

pub use bridge::{stratum_camera_to_external, StratumHelioAdapter, GroupId, AdapterStats};
pub use asset_registry::{AssetRegistry, MeshHandle, MaterialHandle, TextureHandle, ObjectHandle};
pub use renderer::StratumRenderer;
