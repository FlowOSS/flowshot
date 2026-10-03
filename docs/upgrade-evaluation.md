# Dependency Stack Upgrade Evaluation

**Date**: 2026-10-03
**Status**: Research complete — no code changes
**Purpose**: Evaluate the cost of bringing the flowshot workspace to the latest dependency stack, per the directive: "first we want to be on the latest stack, only then releases."

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [wgpu 0.20.1 → 30.x](#1-wgpu-0201--30x)
3. [egui 0.28.1 + egui-wgpu 0.28.1 → 0.36.x](#2-egui-0281--egui-wgpu-0281--036x)
4. [winit 0.30.13 → 0.31](#3-winit-03013--031)
5. [zbus 4.4.0 → 5.x](#4-zbus-440--5x)
6. [ashpd 0.10.3 → 0.13.x](#5-ashpd-0103--013x)
7. [reqwest 0.12 → 0.13](#6-reqwest-012--013)
8. [Recommended Upgrade Order](#recommended-upgrade-order)
9. [Overall Go/No-Go Read](#overall-gono-go-read)

---

## Executive Summary

| Stack | Current | Target | Effort | Risk | Blocker? |
|-------|---------|--------|--------|------|----------|
| wgpu | 0.20.1 | 30.x | **Significant** | Medium | No — but coupled to egui |
| egui/egui-wgpu | 0.28.1 | 0.36.x | **Moderate** | Low | No — pairs with wgpu 30 + winit 0.30 |
| winit | 0.30.13 | 0.31 | **HOLD** | High | **0.31 is still beta** |
| zbus | 4.4.0 | 5.x | **Moderate** | Low | No — unifies duplicate family |
| ashpd | 0.10.3 | 0.13.x | **Moderate** | Low | No — requires zbus 5 |
| reqwest | 0.12 | 0.13 | **Trivial** | Very Low | No |

**Key finding**: egui-wgpu 0.36 pairs with **wgpu 30** and **winit 0.30.13** (NOT winit 0.31). This means the wgpu/egui/winit chain can move as one unit to (wgpu 30, egui 0.36, winit 0.30.13) without touching winit 0.31 at all. winit 0.31 is still in beta (0.31.0-beta.3 as of 2026-09-04) and should be deferred.

**The wgpu 0.20 pin was NOT about needing 0.20 specifically** — it existed only for the egui-wgpu 0.28 pairing. With egui 0.36 pairing with wgpu 30, the pin dissolves.

---

## 1. wgpu 0.20.1 → 30.x

### Our API Surface (exhaustive inventory)

| Area | Files | wgpu APIs Used |
|------|-------|----------------|
| Instance creation | `gpu/instance.rs` | `Instance::new(InstanceDescriptor)`, `Backends::PRIMARY`, `InstanceFlags`, `InstanceFlags::from_build_config()` |
| Adapter/device | `gpu/context.rs` | `enumerate_adapters()`, `Adapter::get_info()`, `Adapter::features()`, `Adapter::limits()`, `Adapter::is_surface_supported()`, `Adapter::request_device()`, `DeviceDescriptor`, `Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` |
| Surface config | `gpu/surface.rs` | `Surface<'static>`, `SurfaceConfiguration`, `get_capabilities()`, `CompositeAlphaMode`, `PresentMode` (Mailbox/FifoRelaxed/Fifo), `TextureFormat`, `surface.configure()` |
| Render pipelines | `render/vector.rs`, `render/text_pipeline.rs`, `render/image_pipeline.rs`, `render/shadow.rs` | `create_shader_module()`, `create_pipeline_layout()`, `create_render_pipeline()`, `RenderPipelineDescriptor`, `VertexState`, `FragmentState`, `MultisampleState`, `DepthStencilState`, `BlendState`, `StencilState`, `vertex_attr_array!`, `ShaderSource::Wgsl`, `PipelineCompilationOptions` |
| MSAA + stencil | `render/target.rs` | `TextureDescriptor`, `TextureViewDescriptor`, sample count negotiation (4x/8x), `Stencil8` format |
| Texture atlas | `render/text_pipeline.rs`, `render/image.rs` | `create_texture()`, `create_view()`, `create_sampler()`, `create_bind_group()`, `create_bind_group_layout()`, `TextureUsages`, `BindingResource`, `FilterMode`, `AddressMode` |
| Sub-region sampling | `render/image.rs` | UV math (pure CPU), `TextureView` sampling in WGSL |
| Staging buffers | `render/staging.rs` | `create_buffer()`, `BufferDescriptor`, `BufferUsages`, `queue.write_buffer()`, `bytemuck::cast_slice()` |
| Render pass | `render/renderer.rs`, `render/pass.rs` | `create_command_encoder()`, `begin_render_pass()`, `RenderPassDescriptor`, `RenderPassColorAttachment`, `RenderPassDepthStencilAttachment`, `LoadOp::Clear`, `StoreOp::Store`/`StoreOp::Discard`, `set_pipeline()`, `set_stencil_reference()`, `set_bind_group()`, `set_vertex_buffer()`, `set_index_buffer()`, `draw_indexed()` |
| Offscreen RT | `render/renderer.rs` | `create_texture()` with `RENDER_ATTACHMENT | COPY_SRC` |
| Readback | `render/readback.rs` | `copy_texture_to_buffer()`, `ImageCopyTexture`, `ImageCopyBuffer`, `COPY_BYTES_PER_ROW_ALIGNMENT`, `BufferUsages::MAP_READ`, `map_async()`, `Maintain::wait()`, `get_mapped_range()`, `buffer.unmap()` |
| Texture upload | `render/image.rs` | `DeviceExt::create_texture_with_data()`, `TextureDataOrder::LayerMajor` |
| egui-wgpu bridge | `egui_host/surface.rs` | `egui_wgpu::Renderer::new()`, `update_texture()`, `update_buffers()`, `render()`, `free_texture()`, `ScreenDescriptor` |

### Breaking Changes That Affect Us (version by version)

#### v22.0.0 (2024-07-17) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v22.0.0)
- **`entry_point` is now `Option`**: Our pipeline descriptors use `entry_point: "vs_main"` / `"fs_main"` as `&str`. Must change to `Some("vs_main")`.
- **`set_bind_group` takes `Option`**: Our `pass.set_bind_group(0, bind_group, &[])` → `pass.set_bind_group(0, Some(bind_group), &[])`.
- **Arc-based resources**: `wgpu` objects (`Device`, `Buffer`, `Texture`, etc.) are now `Clone` via internal `Arc`. Our code wraps some in `Option<>` or passes by reference — this is compatible but we can simplify.
- **Render/compute pass lifetimes relaxed**: Our `RenderPass` usage already works; the lifetime enforcement was restored to v22 behavior in v24.

#### v23.0.0 (2024-10-25) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v23.0.0)
- No direct breakage for our usage patterns.

#### v24.0.0 (2025-01-15) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v24.0.0)
- **`Instance::new` takes `&InstanceDescriptor`**: Our `gpu/instance.rs` line 25: `wgpu::Instance::new(wgpu::InstanceDescriptor { ... })` → `wgpu::Instance::new(&wgpu::InstanceDescriptor { ... })`.
- **Backend options restructured**: Our `InstanceDescriptor` uses `..default()` so the new `backend_options` field is filled by default — **no change needed** unless we want to configure DX12/GL backends (we don't, Vulkan-only via `PRIMARY`).
- **`create_shader_module_unchecked` → `create_shader_module_trusted`**: We use `create_shader_module()` (the safe version) — **no change needed**.

#### v25.0.0 (2025-04-10) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v25.0.0)
- **`device.poll` reworked**: We use `device.poll(wgpu::Maintain::wait())` in `readback.rs`. The `Maintain` API was adjusted — must verify the new signature.

#### v26.0.0 (2025-07-09) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v26.0.0)
- **Error type restructuring**: `CommandEncoderError`, `CopyError` variants reshuffled. Our code uses `Result<_, UiError>` wrappers — the error types we match on (`BufferAsyncError`) are unchanged.

#### v27.0.0 (2025-10-01) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v27.0.0)
- **`EXPERIMENTAL_*` features require `unsafe`**: We only use `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` (a stable feature) — **no change needed**.
- **Buffer mapping APIs no longer have lifetime parameter**: Our `buffer.slice(..).map_async()` usage may need adjustment.

#### v28.0.0 (2025-12-17) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v28.0.0)
- **`enumerate_adapters` is now `async`**: Our `gpu/context.rs` line 82: `instance.enumerate_adapters(OVERLAY_BACKENDS)` → must become `instance.enumerate_adapters(OVERLAY_BACKENDS).await`. This is inside an `async fn` already, so the change is straightforward.
- **`MipmapFilterMode` split from `FilterMode`**: Our `create_sampler()` uses `mipmap_filter: wgpu::FilterMode::Nearest` → must become `mipmap_filter: wgpu::MipmapFilterMode::Nearest`. Affects `render/text_pipeline.rs` and `render/image.rs`.
- **`LoadOp::DontCare` added**: Our `LoadOp::Clear` usage is unaffected.
- **Push constants → "immediates" rename**: We don't use push constants — **no change needed**.

#### v29.0.0 (2026-03-26) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v29.0.0)
- **`InstanceDescriptor` initialization changes**: Our `InstanceDescriptor` usage with `..default()` should absorb this.
- **Bind group layouts now optional in `PipelineLayoutDescriptor`**: Our code passes `bind_group_layouts: &[]` — still valid.
- **`maxInterStageShaderComponents` → `maxInterStageShaderVariables`**: Internal limit rename; our code doesn't reference this limit directly.
- **Depth/stencil state changes**: Our `DepthStencilState` construction uses standard fields — must verify against new API.
- **`WriteOnly`**: New buffer usage flag; our `MAP_READ` usage is unaffected.

#### v30.0.0 (2026-06-25) — [Release](https://github.com/gfx-rs/wgpu/releases/tag/v30.0.0)
- **New `naga-types` crate**: Internal; shouldn't affect our WGSL shaders or pipeline creation.
- **MSRV 1.87**: Our workspace uses edition 2024 — must verify toolchain compatibility.

### Effort Assessment: **Significant** (~2-3 days)

The changes are numerous but mechanical. The highest-impact changes:
1. `Instance::new` by reference (1 call site)
2. `enumerate_adapters` becomes async (1 call site, already in async context)
3. `entry_point` → `Option` (5 pipeline creation sites)
4. `set_bind_group` → `Option` (3 call sites)
5. `MipmapFilterMode` split (2 sampler creation sites)
6. `device.poll` API rework (1 call site in readback)

### Test Coverage Assessment

- **GPU parity harness** (cross-rasterizer, golden fixtures): Will catch any rendering regression end-to-end.
- **`render_smoke` example**: Will catch pipeline creation failures.
- **Unit tests per pipeline module**: Vector, text, image, shadow pipeline tests verify construction.
- **Readback tests**: Verify the GPU→CPU path.
- **What tests WON'T catch**: Compositor-specific present mode behavior (Mailbox vs Fifo), MSAA quality differences, the swapchain semaphore fix (the known VUID bug) — these require live-GPU QA on Hyprland.

### Risk: **Medium**
The known upstream bug fix (swapchain semaphore reuse validation errors, VUID-vkQueueSubmit-pSignalSemaphores-00067) is a **positive risk** — upgrading fixes it. The main risk is subtle rendering regressions caught only by live-GPU QA.

---

## 2. egui 0.28.1 + egui-wgpu 0.28.1 → 0.36.x

### Version Compatibility Matrix (from [egui 0.36.0 workspace Cargo.toml](https://github.com/emilk/egui/blob/0.36.0/Cargo.toml))

| egui version | egui-wgpu wgpu dep | egui-winit winit dep |
|-------------|-------------------|---------------------|
| 0.28.1 (current) | wgpu ^0.20 | winit ^0.29 (INCOMPATIBLE with our 0.30) |
| 0.31.0 | wgpu 24 | winit 0.30 |
| 0.32.0 | wgpu 25 | winit 0.30 |
| 0.33.0 | wgpu 27 | winit 0.30 |
| 0.34.0 | wgpu 28/29 | winit 0.30 |
| **0.36.0** | **wgpu 30** | **winit 0.30.13** |

**Critical finding**: egui-wgpu 0.36 pairs with **winit 0.30.13**, NOT 0.31. This means we can upgrade the entire graphics stack (wgpu 30 + egui 0.36) while staying on winit 0.30. The winit 0.31 question becomes irrelevant for this upgrade cycle.

### Our API Surface

| Area | Files | egui APIs Used |
|------|-------|----------------|
| Context | `egui_host/surface.rs` | `Context::default()`, `ctx.set_fonts()`, `ctx.set_style()`, `ctx.run()`, `ctx.tessellate()`, `ctx.style()` |
| Renderer | `egui_host/surface.rs` | `egui_wgpu::Renderer::new(&device, format, None, 1)`, `update_texture()`, `update_buffers()`, `render()`, `free_texture()` |
| Screen descriptor | `egui_host/surface.rs` | `egui_wgpu::ScreenDescriptor { size_in_pixels, pixels_per_point }` |
| Input bridge | `egui_host/input.rs` | `RawInput`, `Event::*`, `Modifiers`, `PointerButton`, `MouseWheelUnit`, `ImeEvent`, `Rect`, `Pos2`, `Vec2` |
| Theme/fonts | `egui_host/theme/` | `FontDefinitions`, `FontFamily`, `Style`, `Visuals` |
| Cursor icons | `egui_host/present.rs` | `egui::output::CursorIcon` → `winit::window::CursorIcon` mapping (full enum match) |
| Keymap | `egui_host/keymap.rs` | `egui::Key`, `winit::keyboard::Key`/`PhysicalKey` → egui key mapping |

### Breaking Changes (0.28 → 0.36)

From the [egui CHANGELOG](https://github.com/emilk/egui/blob/main/CHANGELOG.md) and [egui-wgpu CHANGELOG](https://github.com/emilk/egui/blob/master/crates/egui-wgpu/CHANGELOG.md):

1. **`Renderer::new` signature**: In 0.33, `egui_wgpu::RendererOptions` was created. The `Renderer::new` API changed — must verify the new constructor signature.
2. **`Modifiers` removed from `RawInput`** (0.36.0): [Modifiers are now an `egui::Event`](https://github.com/emilk/egui/releases/tag/0.36.0). Our `take_raw_input()` sets `modifiers: self.modifiers` — must change to push a `Modifiers` event instead.
3. **Panel API changed** (0.34.0): `CentralPanel` API may have changed — our `frame_with()` passes a `CentralPanel` token.
4. **`clip_rect_margin` removed** (0.36.0): We don't use it — no impact.
5. **Cursor icon enum**: The `egui::output::CursorIcon` → `winit::window::CursorIcon` mapping in `present.rs` may need new variants if egui added any.
6. **wgpu objects no longer wrapped in `Arc`** (0.31.0): Since wgpu 24, wgpu objects are `Clone` natively. Our `GpuContext` stores `wgpu::Adapter`, `wgpu::Device`, `wgpu::Queue` directly — this is compatible.
7. **`capture` module behind feature flag** (0.34.0): We don't use the capture module — no impact.
8. **Stencil buffer attachment** (0.34.0): egui-wgpu now attaches a stencil buffer by default. Our renderer already uses stencil — potential interaction to verify.

### Effort Assessment: **Moderate** (~1-2 days)

The biggest change is the `Modifiers` → `Event` migration in the input bridge. The `Renderer::new` API change requires checking the new constructor. The cursor icon mapping may need a few new arms.

### Test Coverage Assessment

- **Settings/launcher/consent window tests**: Exercise the egui surface end-to-end.
- **Input bridge tests**: The keymap and input accumulation have dedicated tests.
- **What tests WON'T catch**: Visual rendering differences in egui itself (new version may render widgets slightly differently), IME behavior changes.

### Risk: **Low**
egui is used only for settings/launcher/consent windows (never the overlay/editor). A rendering regression in egui itself is cosmetic, not functional.

---

## 3. winit 0.30.13 → 0.31

### Status: **BETA — RECOMMEND HOLD**

As of 2026-10-03:
- **Latest stable**: 0.30.13 (released 2026-03-02) — [crates.io](https://crates.io/crates/winit/versions)
- **Latest pre-release**: 0.31.0-beta.3 (released 2026-09-04)
- **No stable 0.31 release exists**

### Breaking Changes (if/when 0.31 stabilizes)

From the [winit 0.31 changelog](https://github.com/rust-windowing/winit/blob/ca573df5/winit/src/changelog/v0.31.md):

1. **Pointer event overhaul**: `CursorMoved` → `PointerMoved`, `CursorEntered` → `PointerEntered`, `CursorLeft` → `PointerLeft`, `MouseInput` → `PointerButton`. New `PointerKind`, `PointerSource`, `ButtonSource`, `FingerId` types.
2. **`inner_size` → `surface_size` rename**: Throughout `Window`, `WindowAttributes`, `WindowEvent`.
3. **`ActiveEventLoop` and `Window` become traits**: With `cast_ref`/`cast_mut`/`cast` methods for backend extraction.
4. **`ApplicationHandler` changes**: `user_event` → `user_wake_up`, `EventLoopProxy::send_event` → `wake_up`.
5. **`can_create_surfaces`/`destroy_surfaces` split** from `resumed`/`suspended`.
6. **File drag-and-drop API rework**: `DroppedFile`/`HoveredFile`/`HoveredFileCancelled` → `DragEntered`/`DragMoved`/`DragDropped`/`DragLeft`.
7. **`WindowEvent::Resized` → `SurfaceResized`**.
8. **`Touch` removed**: Folded into `PointerKind::Touch`.
9. **`NamedKey::Space` removed**: Match on `Key::Character(" ")` instead.
10. **IME changes**: `set_ime_allowed`/`set_ime_cursor_area`/`set_ime_purpose` deprecated; new `request_ime_update` atomic API. `Ime::DeleteSurrounding` added.
11. **MSRV bump**: 1.70 → 1.85.

### Our winit API Surface (82 files, 152 import sites)

| Area | Key APIs Used |
|------|---------------|
| Event loop | `ApplicationHandler`, `EventLoop`, `ActiveEventLoop`, `ControlFlow`, `EventLoopProxy` |
| Window events | `WindowEvent::Resized`, `CursorMoved`, `CursorLeft`, `MouseInput`, `MouseWheel`, `KeyboardInput`, `Ime`, `Focused`, `CloseRequested`, `ScaleFactorChanged`, `ModifiersChanged` |
| Window creation | `WindowAttributes`, `WindowLevel`, `Fullscreen`, `drag_window()` |
| Keyboard | `KeyCode`, `PhysicalKey`, `Key`, `NamedKey`, `ModifiersState`, `ElementState` |
| Platform | `WindowAttributesExtWayland` (name_with_app_id) |
| Monitor | `MonitorHandle` |
| DPI | `PhysicalSize`, `LogicalSize` |
| Cursor | `CursorIcon` (full enum mapping from egui) |

### Effort Assessment: **Significant** (~3-5 days) IF 0.31 stabilizes

The pointer event overhaul alone touches every event handler in the codebase (82 files). The `inner_size` → `surface_size` rename is a mechanical search-and-replace but touches many files. The `ActiveEventLoop`/`Window` becoming traits may require type annotation changes.

### Recommendation: **HOLD — do not adopt 0.31 beta**

- winit 0.31 is still in beta (beta.3 as of Sep 2026)
- egui-winit 0.36 pairs with winit **0.30.13**, not 0.31
- The breaking changes are extensive and touch 82+ files
- No functional benefit for our use case (the changes are mostly API hygiene)
- **Revisit when 0.31 stabilizes AND egui-winit pairs with it**

### Risk: **High** (if adopted prematurely)
Beta API may change again before stable. 82 files to touch means high merge-conflict risk with ongoing development.

---

## 4. zbus 4.4.0 → 5.x

### Our API Surface

| Area | Files | zbus APIs Used |
|------|-------|----------------|
| Bus service | `daemon/bus.rs` | `#[zbus::interface]`, `fdo::Result`, `fdo::Error`, `zvariant::OwnedValue`, `zvariant::Value`, `zvariant::Str` |
| Instance/single-instance | `daemon/instance.rs` | `Connection`, `ConnectionBuilder`, `Connection::session()`, `request_name_with_flags()`, `RequestNameFlags`, `RequestNameReply`, `call_method()`, `connection.close()` |
| SNI tray | `daemon/tray/item.rs`, `daemon/tray/watcher.rs`, `daemon/tray/dbusmenu.rs` | `#[zbus::interface]`, `#[zbus(property)]`, `fdo::DBusProxy`, `receive_name_owner_changed_with_args()`, `object_server().at()`, `object_server().remove()` |
| P2P (KWin) | `capture-wayland` (p2p feature) | `ConnectionBuilder::address()`, p2p connections for hermetic tests |
| CLI client | `cli/` | `Connection`, `call_method()` |
| Error handling | `daemon/instance.rs`, `daemon/bus.rs` | `zbus::Error::MethodError`, `zbus::Error::NameTaken` |

### Breaking Changes (zbus 4 → 5)

From the [zbus 5.0.0 release notes](https://github.com/dbus2/zbus/releases/tag/zbus-5.0.0):

1. **`zvariant` 5.0**: Massive performance improvements to message encoding/decoding. `OwnedValue` changes.
2. **Message body signature now mandatory**: We construct messages in tests — must verify.
3. **`proxy` macro respects visibility**: Generated proxies are now private by default. Our `#[zbus::interface]` usage is for server-side objects, not client proxies — minimal impact.
4. **`SignalContext` → `SignalEmitter`**: We don't emit signals from interface methods — no impact.
5. **Drop `DBUS_COOKIE_SHA1` auth**: Not used — no impact.
6. **`proxy::ProxyDefault` → `proxy::Defaults`**: Internal rename — no impact on our `#[zbus::interface]` usage.
7. **Drop API deprecated in 4.0**: We don't use deprecated APIs.
8. **`fdo` module restructured**: `fdo::DBusProxy`, `fdo::Result`, `fdo::Error` moved to submodules. Our imports may need adjustment.
9. **`ObjectServer` now implements `Clone`**: We don't need this but it's nice.
10. **Feature-gated blocking API**: `blocking-api` feature. We use the async API — no impact.
11. **MSRV 1.81**: Our edition 2024 workspace should be compatible.

### Executor Model Change

**Critical for our architecture**: zbus 4 uses `async-io` as its default executor (with a dedicated driver thread per connection). zbus 5 adds tight tokio integration via the `tokio` feature. Our daemon currently uses zbus 4 with async-io driven from a tokio runtime (the documented "private executor per connection" model from `daemon/Cargo.toml` comments).

With zbus 5's `tokio` feature, we could simplify to native tokio integration — **but this is optional**. The async-io model still works in zbus 5.

### Unification Benefit

Currently the workspace ships **both** zbus 4 (daemon, CLI, capture-wayland) and zbus 5 (carried internally by ashpd 0.10+ and notify-rust). Upgrading to zbus 5 unifies the family, eliminating the duplicate dependency.

### Effort Assessment: **Moderate** (~1-2 days)

The `#[zbus::interface]` macro usage is the core surface. The macro syntax is largely unchanged. The main work is:
1. Verify `fdo` module import paths
2. Verify `zvariant::OwnedValue` / `Value` / `Str` API compatibility
3. Test the p2p connection path (KWin ScreenShot2 stub tests)
4. Decide whether to adopt the `tokio` feature (simplification) or stay with async-io

### Test Coverage Assessment

- **Bus integration tests**: `bus.rs` has extensive tests using `spawn_service_stub` — these exercise the full zbus round-trip.
- **Instance tests**: Single-instance handshake, argv forwarding, p2p ownership.
- **Tray tests**: SNI registration, watcher registration, menu dispatch.
- **What tests WON'T catch**: Subtle message encoding differences between zvariant 4 and 5, executor threading model differences.

### Risk: **Low**
Our zbus usage is standard (interface macros, connection management, method calls). The zbus project provides good migration documentation. The unification benefit (eliminating the duplicate zbus 4+5 family) is a clear win.

---

## 5. ashpd 0.10.3 → 0.13.x

### Our API Surface

| Area | Files | ashpd APIs Used |
|------|-------|----------------|
| Screenshot portal | `portal/screenshot.rs` | `Screenshot::request()`, `.interactive()`, `.modal()`, `.send()`, `response()`, `response.uri()` |
| ScreenCast portal | `portal/screencast.rs` | `Screencast::new()`, `create_session()`, `select_sources()`, `CursorMode`, `SourceType`, `PersistMode`, `start()`, `open_pipe_wire_remote()`, `Stream::pipe_wire_node_id()`, `Stream::mapping_id()`, `Stream::position()`, `Stream::size()` |
| GlobalShortcuts portal | `daemon/shortcut/portal.rs` | `GlobalShortcuts::new()`, `create_session()`, `bind_shortcuts()`, `NewShortcut::new()`, `.preferred_trigger()`, `receive_activated()`, `Activated::shortcut_id()`, `Session::close()` |
| OpenURI portal | `daemon/notify/desktop.rs` | `OpenFileRequest::default()`, `.send_uri()` |
| Error handling | `portal/run.rs`, `daemon/shortcut/portal.rs` | `ashpd::Error::PortalNotFound`, `Error::Response`, `Error::Zbus`, `Error::NoResponse`, `ResponseError::Cancelled` |

### Version History

| ashpd version | zbus dep | Key changes |
|--------------|----------|-------------|
| 0.10.3 (current) | zbus 4 | Our pin |
| 0.11.0 (2025-02-15) | **zbus 5.0** | Major: moved to zbus 5 |
| 0.12.0 (2025-08-23) | zbus 5 | Incremental |
| 0.13.0 (2026-02-21) | zbus 5.13 | Per-portal features, reduced deps, pass DBus connection to portal calls, `restore_data` type fixes |

### Breaking Changes (0.10 → 0.13)

From the [ashpd 0.13.0 release](https://github.com/bilelmoussaoui/ashpd/releases/tag/0.13.0):

1. **Per-portal feature flags**: Every portal is now a separate Cargo feature. We use Screenshot, ScreenCast, GlobalShortcuts, OpenURI — must enable these features explicitly.
2. **Pass DBus connection to portal calls**: New API allows passing a custom `zbus::Connection` to every portal call. Our code uses the default session connection — the old API should still work as default.
3. **`restore_data` type fixes** (screencast backend): The `screencast` `restore_data` type was fixed. Our code uses `PersistMode::DoNot` — should be compatible.
4. **Removed unnecessary lifetime parameters on proxy types**: Our `Session<'static, GlobalShortcuts<'static>>` types may simplify.
5. **`Session::Closed` signal fix**: Previously silently discarded — now properly delivered. Our `session.close()` usage is compatible.
6. **`NameLost` signal watching** (backend feature): We don't use the backend feature.
7. **zbus 5 dependency**: ashpd 0.11+ requires zbus 5. This is the **primary reason** to upgrade zbus in tandem.

### Effort Assessment: **Moderate** (~1 day)

The API surface we use is stable across versions. The main changes:
1. Enable per-portal feature flags in `Cargo.toml`
2. Verify `Screenshot`, `Screencast`, `GlobalShortcuts`, `OpenFileRequest` API signatures
3. Update error matching if `ashpd::Error` variants changed
4. The zbus 5 dependency forces the zbus upgrade

### Test Coverage Assessment

- **Portal screenshot tests**: Unit tests for crop/restrict logic; integration test stubs.
- **Portal screencast tests**: Stream matching, frame assembly.
- **GlobalShortcuts tests**: `dispatch_activated` pure tests, error classification tests.
- **What tests WON'T catch**: Live portal behavior differences across portal backends (XDPH, GNOME, KDE), PipeWire stream negotiation.

### Risk: **Low**
Our ashpd usage is well-tested and the API surface is narrow. The per-portal feature flag change is a `Cargo.toml`-only change.

---

## 6. reqwest 0.12 → 0.13

### Our API Surface

| Area | Files | reqwest APIs Used |
|------|-------|------------------|
| Imgur uploader | `actions/upload/imgur.rs` | `Client::builder()`, `.timeout()`, `.build()`, `Client::new()`, `.post()`, `.header()`, `.multipart()`, `.send()`, `response.status()`, `response.text()`, `response.json()` |
| Multipart | `actions/upload/imgur.rs` | `multipart::Form`, `multipart::Part::bytes()`, `.file_name()`, `.text()` |
| Features used | `actions/Cargo.toml` | `rustls-tls`, `multipart` |

### Breaking Changes (0.12 → 0.13)

From the [reqwest 0.13.0 release](https://github.com/seanmonstar/reqwest/releases/tag/v0.13.0):

1. **`rustls` is now the default TLS backend** (was `native-tls`): We explicitly enable `rustls-tls` — this feature is **renamed to `rustls`**.
2. **`rustls-tls` feature renamed to `rustls`**: Our `Cargo.toml` line 18: `features = ["rustls-tls", "multipart"]` → `features = ["rustls", "multipart"]`.
3. **`rustls` crypto provider defaults to `aws-lc` instead of `ring`**: Transparent to us — reqwest handles this internally.
4. **`query` and `form` are now crate features, disabled by default**: We don't use `.query()` or `.form()` — **no impact**.
5. **TLS-related method renames** (soft-deprecated): We use `.timeout()`, `.build()`, `.post()`, `.header()`, `.multipart()`, `.send()` — none renamed.
6. **Long-deprecated methods removed**: We don't use any deprecated methods.

### Effort Assessment: **Trivial** (~15 minutes)

One `Cargo.toml` change: `"rustls-tls"` → `"rustls"`. No source code changes needed.

### Test Coverage Assessment

- **Imgur upload tests**: Full wiremock-based test suite covering success, 429, 400, 500, invalid JSON, auth header.
- **What tests WON'T catch**: TLS handshake differences between rustls/aws-lc and the previous rustls/ring combination — but this is transparent to the application layer.

### Risk: **Very Low**
The API surface we use is unchanged. The feature rename is the only breaking change that affects us.

---

## Recommended Upgrade Order

The dependencies have a dependency graph that constrains the order:

```
reqwest (standalone) → trivial, do first

zbus 5 ← ashpd 0.11+ requires it
       ← notify-rust already carries it internally
       
ashpd 0.13 → requires zbus 5

wgpu 30 ← egui-wgpu 0.36 requires it
egui 0.36 → requires wgpu 30, pairs with winit 0.30.13
winit 0.30.13 → stays (0.31 is beta)
```

### Phase 1: Standalone (no dependencies between them)

| Step | Stack | Effort | Rationale |
|------|-------|--------|-----------|
| 1a | **reqwest 0.12 → 0.13** | Trivial | One Cargo.toml feature rename. Zero risk. Immediate win. |
| 1b | **zbus 4 → 5** | Moderate | Unifies the duplicate zbus family. Required before ashpd upgrade. |

### Phase 2: Portal stack (depends on zbus 5)

| Step | Stack | Effort | Rationale |
|------|-------|--------|-----------|
| 2 | **ashpd 0.10 → 0.13** | Moderate | Requires zbus 5 from Phase 1b. Unifies portal stack. |

### Phase 3: Graphics stack (coupled unit)

| Step | Stack | Effort | Rationale |
|------|-------|--------|-----------|
| 3a | **wgpu 0.20 → 30** | Significant | The big one. Must move with egui. |
| 3b | **egui 0.28 → 0.36** | Moderate | Pairs with wgpu 30. Must move with wgpu. |
| 3c | **winit stays at 0.30.13** | None | egui-winit 0.36 pairs with 0.30. 0.31 is beta. |

Steps 3a and 3b must be done **in the same commit/PR** because egui-wgpu 0.36 requires wgpu 30.

### Phase 4: Deferred

| Step | Stack | Condition |
|------|-------|-----------|
| 4 | **winit 0.30 → 0.31** | When 0.31 stabilizes AND egui-winit pairs with it |

---

## Overall Go/No-Go Read

### **GO** — with caveats

**The upgrade is feasible and recommended.** The total effort is approximately **5-8 days** of focused work, dominated by the wgpu 0.20→30 migration. The key insights:

1. **The wgpu 0.20 pin dissolves**: It existed only for the egui-wgpu 0.28 pairing. With egui 0.36 pairing with wgpu 30, the pin is no longer needed.

2. **winit 0.31 is a non-issue for now**: egui-winit 0.36 pairs with winit 0.30.13. We stay on 0.30 and defer 0.31 until it stabilizes and the ecosystem catches up.

3. **The zbus unification is a clear win**: We currently ship both zbus 4 and zbus 5 (via ashpd/notify-rust). Upgrading to zbus 5 eliminates the duplicate family.

4. **The known wgpu bug fix is a positive outcome**: The swapchain semaphore reuse validation error (VUID-vkQueueSubmit-pSignalSemaphores-00067) is fixed in newer wgpu versions.

5. **Test coverage is strong**: 1337 tests including GPU parity suites, portal stubs, bus integration tests, and wiremock upload tests cover the upgrade surface well. The main gap is live-compositor QA (Hyprland, KDE, GNOME) which cannot be automated.

### Caveats

- **Live-GPU QA is mandatory** after the wgpu upgrade: The test suite catches API-level regressions but not compositor-specific present mode behavior, MSAA quality, or the semaphore fix verification.
- **The winit 0.31 upgrade is a separate, larger effort** (82 files, pointer event overhaul) that should be planned independently.
- **MSRV may need verification**: wgpu 30 requires Rust 1.87, egui 0.36 requires 1.95. Our `rust-toolchain.toml` must be checked.

### Upstream Sources

- wgpu CHANGELOG: https://github.com/gfx-rs/wgpu/blob/main/CHANGELOG.md
- wgpu releases: https://github.com/gfx-rs/wgpu/releases
- egui CHANGELOG: https://github.com/emilk/egui/blob/main/CHANGELOG.md
- egui-wgpu CHANGELOG: https://github.com/emilk/egui/blob/master/crates/egui-wgpu/CHANGELOG.md
- egui 0.36.0 workspace: https://github.com/emilk/egui/blob/0.36.0/Cargo.toml
- winit 0.31 changelog: https://github.com/rust-windowing/winit/blob/main/winit/src/changelog/v0.31.md
- winit crates.io: https://crates.io/crates/winit/versions
- zbus 5.0.0 release: https://github.com/dbus2/zbus/releases/tag/zbus-5.0.0
- ashpd 0.13.0 release: https://github.com/bilelmoussaoui/ashpd/releases/tag/0.13.0
- ashpd 0.13.0 Cargo.toml: https://github.com/bilelmoussaoui/ashpd/blob/0.13.0/Cargo.toml (confirms zbus 5.13.2)
- reqwest 0.13.0 release: https://github.com/seanmonstar/reqwest/releases/tag/v0.13.0
