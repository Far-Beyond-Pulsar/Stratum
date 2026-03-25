//! `stratum-helio` — Integration bridge between Stratum and Helio.
//!
//! This crate is the only place where Stratum types and Helio/wgpu types meet.
//!
//! ## Responsibilities
//!
//! * **`AssetRegistry`** — maps Stratum handles → Helio scene IDs; owned by the
//!   host application and shared with `HelioIntegration`.
//! * **`bridge`** — pure translation functions: `RenderView → Camera`,
//!   `LightData → GpuLight`.
//! * **`HelioIntegration`** — wraps `Renderer + AssetRegistry` and exposes a
//!   single `submit_frame()` call that drives Helio for the whole frame.
//!
//! ## Abstraction guarantee
//!
//! Neither the `stratum` crate nor any `Level` / `Entity` type is visible
//! to Helio. Helio receives only `Camera`, GPU primitives, and `wgpu::TextureView`.

pub mod asset_registry;
pub mod bridge;
pub mod integration;

pub use asset_registry::AssetRegistry;
pub use integration::HelioIntegration;

// ── Helio public re-exports ───────────────────────────────────────────────────
// Host applications use these without a direct `helio` dep.
pub use helio::{
    // Mesh / geometry
    MeshUpload,
    PackedVertex,
    MeshId,
    MaterialId,
    // Virtual geometry (GPU-driven LOD)
    VirtualMeshUpload,
    VirtualMeshId,
    VirtualObjectDescriptor,
    // Materials
    GpuMaterial,
    MaterialAsset,
    MaterialTextures,
    MaterialTextureRef,
    // Textures
    TextureUpload,
    TextureId,
    // Lighting
    GpuLight,
    // Groups / visibility masks
    GroupId,
    GroupMask,
    // Billboards
    BillboardInstance,
    // Render configuration
    RendererConfig,
    ShadowQuality,
    // Result types
    Result,
};

// GI config.
pub use helio::GiConfig;
