# Learnings — flowshot

Conventions, patterns, and successful approaches discovered during work on this plan.

_Auto-scaffolded by /ulw-execute. Append new entries below - never overwrite._

---

## Todo 2: Design tokens + TOML config + versioned migration (flowshot-core)

**What landed**
- `tokens.rs`: `DesignTokens { palette, spacing, radii, shadows, typography, easings }`, all serde + `Default`. Accent `#2AA198` (provisional), contrast `#1A1A2E`, dim 190. 4 easing curves (standard/decelerate/accelerate/sharp) exposed via `Easing::standard()` etc. + `DesignTokens::easing(name)` lookup.
- `config.rs`: grouped schema `[capture] [save] [editor] [tools.*] [pin] [upload] [ui] [daemon]` with `CONFIG_VERSION = 2`, migration registry `MIGRATIONS: &[(from_version, fn)]`, `ConfigError` (thiserror), `load/save/from_toml_str/to_toml_string`, `FromStr` impl.
- `lib.rs`: exposes `config`/`tokens` modules + re-exports `Config`, `ConfigError`, `CONFIG_VERSION`, `DesignTokens`.

**Key patterns that worked**
- Byte-stable TOML: scalar `config_version` declared FIRST in `Config` (toml crate errors on values-after-tables); container-level `#[serde(default)]` on every group so partial configs fill defaults; `Option<Region>` with `skip_serializing_if = "Option::is_none"` keeps `last_region` out of output until first capture.
- Migration driver: parse to `toml::Value`, missing `config_version` => 0 (legacy), reject `> CONFIG_VERSION` with `FutureVersion`, loop registry steps rewriting the table, stamp version, then `Value::try_into::<Config>()`. v1->v2 renames `save.filename_template` -> `filename_pattern` without clobbering an explicit new key.
- UiConfig defaults are seeded from `tokens::Palette`/`Typography` — single source of truth between tokens and config.
- No-unwrap tests: `#[test] fn x() -> Result<(), ConfigError>` for happy paths, `assert!(matches!(..))` for error paths (clippy `unwrap_used`/`expect_used` are workspace-deny and apply to test targets too).

**Gotchas hit**
- `text.contains("last_region")` false-positives on `save_last_region` — assert on parsed `toml::Table::contains_key` instead of substrings.
- Edition 2024 let-chains required by `clippy::collapsible_if` (deny via `clippy::all`): collapse `if let && let && cond` instead of nesting.
- `clippy::derivable_impls` (deny) flags manual `Default` impls equal to derive — use `#[derive(Default)]` for `DaemonConfig`/`ToolsConfig`/`ArrowToolConfig` + `#[default]` enum variants; manual impls only where values differ from type defaults.
- `clippy::unnecessary_wraps` (pedantic) fires on always-`Ok` migration fns; kept fallible signature for registry uniformity with `#[allow]` + justification.
- `struct_excessive_bools` (pedantic) on `EditorConfig` — allowed with reason: flags are spec-mandated independent settings.

**Verification**: `cargo test -p flowshot-core` 18 unit + 1 doctest green (13 under `config::`), clippy `--all-targets` 0 warnings, `cargo fmt --check` clean, default-config write->reload byte-stable (string + file level tests).

## Todo 2: scene graph + snapshot undo + counter renumbering (flowshot-core::scene)
- `Box<dyn ToolObject>` clone pattern: supertrait `ToolObjectClone` with blanket `impl<T: ToolObject + Clone + 'static>` — concrete types only derive Clone.
- Deref coercion `&Box<dyn T> -> &dyn T` does NOT auto-apply in return position; use `let object = self.objects.get(id)?; Some(&**object)`. Explicit `&Box<...>` type annotation triggers clippy::borrowed_box (deny via clippy::all); closure `.map(|o| &mut **o)` hits a WF lifetime error (`'1 must outlive 'static`) — the let-? + `&mut **object` form compiles clean.
- `f32: From<u32>` does not exist (precision loss); use `chars as f32` + `#[allow(clippy::cast_precision_loss)]`.
- Edition 2024 let-chains required by clippy::collapsible_if (deny): `if let Some(x) = opt && cond { }`.
- UndoStack: full (before, after) snapshot pairs + cursor; undo/redo return Option<Scene> clones, None = silent no-op; push truncates redo tail then evicts oldest while over limit; limit=0 disables history.
- Counter numbering lives ONLY in Scene: add_object auto-numbers count==0 with max+1; remove_object decrements counts > removed; restore_object force-assigns max+1 (undo-restore rule). Snapshot undo restores exact pre-op numbering; max+1 applies to re-add flows.
- Scene ids are dense arena indices; remove compacts + remaps z_order (invariant: z_order is a permutation of 0..len — proptest-verified over random add/remove/raise/lower sequences).
- serde: `#[serde(tag = "type")]` enum ToolObjectData bridges dyn objects; tags match type_id() strings; Scene Serialize/Deserialize delegate via SceneData; toml crate roundtrips internally-tagged enums fine (test uses toml, no serde_json dep needed).
- config bridge: `UndoStack::from_undo_limit(EditorConfig::undo_limit)`; DEFAULT_UNDO_LIMIT=100 matches config default (test asserts the tie).
- proptest is in flowshot-core [dependencies] (not dev-deps) — usable from #[cfg(test)] directly; import `proptest::prop_assert_eq` explicitly inside test mod.

## Todo 2: geometry (flowshot-core::geometry) — 2026-09-25
- **Space-separated newtypes**: `PhysicalPx(i32)` / `Logical(f64)`; `Point<C>`/`Size<C>`/`Rect<C>` generic over coord with aliases (`PhysicalRect`, `LogicalRect`, ...). `ToLogical` implemented ONLY for physical types, `ToPhysical` ONLY for logical → double scaling is a compile error. Exactly one conversion fn per direction (trait method), also for compound types.
- **Conversions are total**: invalid scale (NaN/≤0/inf) falls back to 1.0 in `to_logical`/`to_physical` (no panics in lib); validated construction via `OutputInfo::new` → `Result<_, GeometryError>` (thiserror). `to_physical` rounds half-away-from-zero, saturates at i32 bounds.
- **Physical-first crops**: `OutputLayout::crop_rects(region)` intersects in logical space per output, then converts EDGES (not widths) with THAT output's scale into its post-transform buffer (`buffer_size()` = transform.apply_to_size(physical_size)). Averaged-scale bug is regression-tested (`mixed_scales_crop_per_output_not_averaged`).
- **Transform**: 8 wl_output-convention variants (flip first, then rotate CCW). `map_point` (src→dst, usize, saturating), `remap_buffer<T: Copy>` (channels-generic, Result on size errors), `inverse()`, `TryFrom<u32>` wire values 0..=7. Flips are involutions; t⁴ = identity for all.
- **f64 gotcha (important for future geometry work)**: `x + (right - x) != right` in f64 (1 ulp). Exact-equality proptests for rect algebra (self-intersection, union absorption, clamp idempotency) MUST use integral-valued f64 grids; fractional layouts need ~1e-9 tolerance. Layouts anchored at x=0,y=0 keep union bounds exact (adding 0.0 is exact).
- **Rects are half-open** [x, x+w) — boundary points belong to the output starting there; makes `output_at` deterministic for adjacent outputs.
- **proptest in nested `prop_flat_map`**: closures are `Fn`, can't move captured values — clone per nesting level (`let layout_x = layout.clone(); move |x| ...`).
- CI gate is `cargo clippy -- -D warnings` with workspace pedantic lints → lib needs `#[must_use]` on pure fns (incl. trait methods returning Self), `# Errors` doc sections, no bare `as` casts (use `f64::from`, `usize::try_from`, or one allowed helper `round_to_i32`).
- proptest moved to `[dev-dependencies]` in flowshot-core (workspace lists it under Testing); lib stays pure: std + serde + thiserror only.
- Result: 41 geometry tests green (28 proptest fns × 256 cases + 13 unit), clippy --all-targets -D warnings clean, fmt clean.
