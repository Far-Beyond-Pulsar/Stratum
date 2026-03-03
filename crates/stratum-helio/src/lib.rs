//! `stratum-helio` — Integration bridge between Stratum and Helio.
//!
//! This crate is the only place where Stratum types and Helio/wgpu types meet.
//!
//! ## Responsibilities
//!
//! * **`AssetRegistry`** — maps `MeshHandle → GpuMesh`; owned by the host
//!   application and shared with `HelioIntegration`.
//! * **`bridge`** — pure translation functions: `RenderView → Camera`,
//!   `EntityStore → Scene`.
//! * **`HelioIntegration`** — wraps `Renderer + AssetRegistry` and exposes a
//!   single `submit_frame()` call that drives Helio for the whole frame.
//!
//! ## Abstraction guarantee
//!
//! Neither the `stratum` crate nor any `Level` / `Entity` type is visible
//! to Helio. Helio receives only `Camera`, `Scene`, and `wgpu::TextureView`.

pub mod asset_registry;
pub mod bridge;
pub mod integration;

pub use asset_registry::AssetRegistry;
pub use integration::HelioIntegration;
