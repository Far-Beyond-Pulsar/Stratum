# Stratum

**World orchestration layer for the Pulsar engine.**

Stratum sits between game/editor code and the [Helio](https://github.com/Far-Beyond-Pulsar/Helio) renderer. It manages levels, spatial streaming, cameras, simulation modes, and render view production — without touching a single GPU type.

```
Game / Editor code
      │
      ▼
 ┌─────────────────────────────────────────────────┐
 │  Stratum                                        │
 │  ├── Level       entities + spatial partition   │
 │  ├── Cameras     editor + game, all modes       │
 │  └── Mode        Editor | Game (hot-switchable) │
 └────────────────────────┬────────────────────────┘
         Vec<RenderView>  │
                          ▼
 ┌─────────────────────────────────────────────────┐
 │  stratum-helio   (integration bridge)           │
 │  ├── AssetRegistry   MeshHandle → GpuMesh       │
 │  └── HelioIntegration  submit_frame()           │
 └────────────────────────┬────────────────────────┘
                          ▼
                    Helio Renderer
              (knows nothing about Levels)
```

---

## Crates

| Crate | Description |
|---|---|
| `stratum` | Core world layer — zero GPU dependencies |
| `stratum-helio` | Thin integration bridge between Stratum and Helio |
| `examples` | `stratum_basic` — feature-parity demo with `render_v2_basic` |

---

## Architecture

### Hard GPU Boundary

`stratum` has **zero** wgpu/Helio imports. Entities reference meshes via opaque `MeshHandle(u64)` handles resolved at render-time by the `AssetRegistry` in `stratum-helio`. Helio never receives a `Level`, `Entity`, or `Camera` type — only `Scene`, `Camera`, and `wgpu::TextureView`.

### Data Flow — One Frame

```rust
// 1. Advance world state (partition activation, simulation clock)
stratum.tick(delta_time);

// 2. Produce render views (pure — no side effects)
let views: Vec<RenderView> = stratum.build_views(width, height, time);

// 3. Translate and submit to Helio
integration.submit_frame(&views, level, &surface_view, delta_time)?;
```

`build_views` runs per-camera frustum culling against the active world partition and returns a priority-sorted list of `RenderView`s. Helio consumes only the plain-data result.

---

## Core Concepts

### `Stratum`

Top-level orchestrator. Owns:
- All `Level`s
- The `CameraRegistry`
- The current `SimulationMode`

```rust
let mut stratum = Stratum::new(SimulationMode::Game);
let level_id = stratum.create_level("main", 16.0, 48.0); // chunk_size, activation_radius
let cam_id   = stratum.register_camera(StratumCamera { ... });
stratum.tick(dt);
let views = stratum.build_views(1280, 720, time);
```

### `Level`

A structured world container. Owns entities and the world partition.

```rust
let level = stratum.level_mut(level_id).unwrap();

// Spawn an entity — automatically placed into the correct spatial chunk
let id = level.spawn_entity(
    Components::new()
        .with_transform(Transform::from_position(Vec3::new(0.0, 1.0, 0.0)))
        .with_mesh(mesh_handle)
        .with_light(LightData::Point { color: [1.0, 0.8, 0.3], intensity: 5.0, range: 8.0 })
);

// Force-activate all chunks (no streaming required for small levels)
level.activate_all_chunks();
```

### `WorldPartition`

Grid-based spatial streaming. Each frame, chunks within `activation_radius` of any active camera are resident; others are evicted.

```rust
// Driven automatically by stratum.tick() — but can be called manually:
level.activate_partition_around(&[cam_pos_a, cam_pos_b]);
```

Every state transition is marked with a `// STREAMING HOOK` comment — splice in async disk IO without restructuring anything else.

### `CameraRegistry` + `StratumCamera`

Cameras are world-level, not level-local. They survive level loads/unloads.

```rust
// Game camera (renders in SimulationMode::Game)
let game_cam = stratum.register_camera(StratumCamera {
    id:            CameraId::PLACEHOLDER,
    kind:          CameraKind::GameCamera { tag: "main".into() },
    position:      Vec3::new(0.0, 3.0, 10.0),
    yaw:           0.0,
    pitch:         -0.2,
    projection:    Projection::perspective(FRAC_PI_4, 0.1, 1000.0),
    render_target: RenderTargetHandle::PrimarySurface,
    viewport:      Viewport::full(),
    priority:      0,
    active:        true,
});

// Editor camera (renders in SimulationMode::Editor)
let ed_cam = stratum.register_camera(StratumCamera {
    kind: CameraKind::EditorPerspective,
    viewport: Viewport::full(),
    ..
});
```

| Camera kind | Editor mode | Game mode |
|---|---|---|
| `EditorPerspective` | ✓ renders | ✗ |
| `EditorOrthographic` | ✓ renders | ✗ |
| `GameCamera` | ✗ | ✓ renders |

### `SimulationMode`

Hot-switchable at any point during a session.

```rust
stratum.set_mode(SimulationMode::Editor);
stratum.toggle_mode(); // Editor → Game
```

### `RenderView`

The contract between Stratum and the renderer. Plain data, no GPU handles.

```rust
pub struct RenderView {
    pub camera_id:        CameraId,
    pub view_proj:        Mat4,
    pub camera_position:  Vec3,
    pub time:             f32,
    pub render_target:    RenderTargetHandle,
    pub viewport:         Viewport,
    pub visible_entities: Vec<EntityId>,
    pub priority:         i32,
}
```

### `Viewport`

Normalized (0..1) rectangle within a render target. Enables split-screen, PiP, and multi-viewport editor layouts without any GPU code.

```rust
Viewport::full()              // 0,0 → 1,1
Viewport::left_half()         // 0,0 → 0.5,1
Viewport::right_half()        // 0.5,0 → 1,1
Viewport::top_right(0.3)      // inset PiP at top-right
```

### `Frustum`

Six-plane frustum extracted from a view-projection matrix (Gribb/Hartmann method, wgpu NDC). Used by `build_views` for per-camera entity culling.

---

## `stratum-helio` Integration

### `AssetRegistry`

Maps `MeshHandle → GpuMesh`. Populated once by the host application at startup.

```rust
let mut assets = AssetRegistry::new();
let h = assets.add(GpuMesh::cube(&device, [0.0, 0.5, 0.0], 0.5));
// h: MeshHandle — pass to Components::with_mesh(h)
```

### `HelioIntegration`

Wraps `Renderer + AssetRegistry`. One call per frame:

```rust
let mut integration = HelioIntegration::new(renderer, assets);
integration.submit_frame(&views, level, &surface_view, dt)?;
```

Internally, for each `RenderView`:
1. Resolves `RenderTargetHandle` → `&wgpu::TextureView`
2. Translates `visible_entities + EntityStore → helio Scene`
3. Translates `RenderView → helio Camera`
4. Calls `renderer.render_scene()`

---

## Multi-Camera Layout Examples

```rust
// ── Split-screen co-op ─────────────────────────────────────────────────────
stratum.register_camera(StratumCamera {
    kind: CameraKind::GameCamera { tag: "p1".into() },
    viewport: Viewport::left_half(),
    priority: 0,
    ..
});
stratum.register_camera(StratumCamera {
    kind: CameraKind::GameCamera { tag: "p2".into() },
    viewport: Viewport::right_half(),
    priority: 0,
    ..
});

// ── Editor with minimap ────────────────────────────────────────────────────
stratum.register_camera(StratumCamera {
    kind: CameraKind::EditorPerspective,
    viewport: Viewport::full(),
    priority: 0,
    ..
});
stratum.register_camera(StratumCamera {
    kind: CameraKind::EditorOrthographic,
    viewport: Viewport::top_right(0.3),
    priority: 1, // renders on top
    ..
});

// ── Render-to-texture (reflections, portals) ───────────────────────────────
stratum.register_camera(StratumCamera {
    render_target: RenderTargetHandle::OffscreenTexture("reflection".into()),
    viewport: Viewport::full(),
    ..
});
```

---

## Building

```powershell
# From the Stratum workspace root
cargo build --workspace

# Run the demo example (requires Vulkan / DX12 with ray-tracing support)
cargo run -p examples --bin stratum_basic

# Run all tests
cargo test --workspace

# Check only (fast)
cargo check --workspace
```

### Prerequisites

- Rust stable (2021 edition)
- GPU with ray-tracing support (same requirement as Helio `render_v2_basic`)
- The [Helio](https://github.com/Far-Beyond-Pulsar/Helio) repository checked out as a sibling directory:

```
genesis/
├── Helio/      ← renderer
└── Stratum/    ← this repo
```

The `Cargo.toml` workspace references Helio via a relative path:
```toml
helio-render-v2 = { path = "../../Helio/crates/helio-render-v2" }
```

---

## Module Reference

| Module | Exports | Description |
|---|---|---|
| `stratum` | `Stratum` | Top-level orchestrator |
| `level` | `Level`, `LevelId`, `StreamingState` | World container |
| `partition` | `WorldPartition` | Grid-based spatial streaming |
| `chunk` | `Chunk`, `ChunkCoord`, `ChunkState`, `Aabb` | Spatial cell primitives |
| `entity` | `EntityId`, `EntityStore`, `Components`, `Transform`, `MeshHandle`, `LightData` | Minimal entity model |
| `camera` | `CameraId`, `StratumCamera`, `CameraKind`, `Projection` | Camera types |
| `camera_registry` | `CameraRegistry` | Camera ownership and lookup |
| `render_view` | `RenderView`, `RenderTargetHandle`, `Viewport` | Renderer contract |
| `visibility` | `Frustum` | Frustum extraction and culling |
| `mode` | `SimulationMode` | Editor / Game mode |
| `render_graph` | _(internal)_ | Per-frame view assembly |
| `stratum_helio::asset_registry` | `AssetRegistry` | `MeshHandle → GpuMesh` |
| `stratum_helio::bridge` | _(internal)_ | Stratum → Helio translation |
| `stratum_helio::integration` | `HelioIntegration` | Frame submission |

---

## Design Principles

1. **Zero GPU in `stratum`** — no `wgpu`, no `Arc<Buffer>`, no handles. GPU ownership is fully in `stratum-helio`.
2. **Cameras are world-level** — not level-local; survive level transitions.
3. **`build_views` is pure** — reads state, no mutations, no allocations beyond the returned `Vec`.
4. **Explicit streaming hooks** — every chunk state transition is a labelled callsite ready for async IO.
5. **No global mutable state** — all state is owned by `Stratum` instances.
6. **Zero unsafe** — the entire codebase is safe Rust.

---

## License

See [LICENSE](LICENSE).
