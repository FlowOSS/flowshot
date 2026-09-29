# Codebase comparison: FlowShot vs Flameshot

How FlowShot's codebase compares to Flameshot's in size and density, measured
with the same tool on both trees. The numbers were taken on 2026-09-29:

- **FlowShot**: this repository, current checkout (pre-1.0).
- **Flameshot**: `flameshot-org/flameshot` master @ `2d47806`, version 15.0.0
  (the old `flameshot/flameshot` org URL 404s; the project lives under
  `flameshot-org` now). Depth-1 clone at `/tmp/flameshot-ref`.

## TL;DR

The honest headline is not "FlowShot has less code". At the application-source
level FlowShot has about **3x more** code than Flameshot (60,066 vs 20,119
lines), and about **1.85x more** once FlowShot's inline test modules are
factored out (~37,300 vs 20,119 production lines). FlowShot only looks smaller
at the whole-repo level (96,890 vs 193,227 total lines), and that gap is
almost entirely Flameshot's 161,052 lines of translation XML, which FlowShot
does not carry.

The size difference at the source level has three main causes, in order of
weight:

1. **Tests live in the source tree.** Roughly 23,000 of FlowShot's 60,066
   source lines are test code (Rust convention puts `#[cfg(test)]` modules
   next to the code they test). Flameshot ships essentially no automated
   tests: its `tests/` directory is two shell scripts.
2. **FlowShot implements its own UI toolkit.** Flameshot builds on Qt
   (QWidget, QPainter, Qt's text layout and IME), so the toolkit layer is
   millions of lines that live outside the repo. FlowShot renders its own UI
   on wgpu; `flowshot-ui` alone (33,545 code lines) is larger than all of
   Flameshot's application code.
3. **Wayland protocol code is hand-rolled.** `flowshot-capture-wayland`
   (9,793 code lines) speaks ext-image-copy-capture, wlr-screencopy, and
   friends directly. Flameshot delegates capture to Qt and portal APIs.

## Summary table

| Metric | FlowShot | Flameshot | Ratio (FS/F) |
|---|---|---|---|
| App source, code lines | 60,066 (Rust, 7 crates) | 20,119 (C++ .cpp + .h) | 2.99x more |
| App source, files | 347 | 225 | 1.54x more |
| App source, comments (line comments) | 1,826 | 1,686 | similar |
| App source, doc comments | 12,009 (rustdoc) | n/a (not separated) | - |
| App source, blanks | 6,183 | 3,480 | - |
| Estimated production-only code | ~37,300 | 20,119 | ~1.85x more |
| Dedicated test files, code lines | 13,502 | 104 (2 shell scripts) | ~130x more |
| Test functions | 1,213 grep-visible (1,218 per test runner) | 0 automated | - |
| Test-to-production-code ratio | ~0.74 (est.), 0.22 (dedicated files only, vs all src) | ~0.005 | - |
| Avg source file size (code lines) | 173 | 89 | 1.94x denser files |
| Largest source file (code lines) | 1,339 (`geometry.rs`) | 1,740 (`capturewidget.cpp`) | FlowShot's is smaller |
| Function/method count (rough grep) | 3,117 `fn` | ~1,111 `Class::method(` defs | ~2.8x more |
| Whole repo, total lines | 96,890 | 193,227 | 0.50x (half) |
| Translation files | 0 | 49 files, 161,052 lines | Flameshot only |
| Docs (Markdown lines) | 1,529 (15 files) + README/CONTRIBUTING | release notes + dev/ (not fully counted) | - |

## Per-area breakdown

### FlowShot, application source (`crates/*/src`, tokei "Code" column)

| Crate | Files | Code | What it owns |
|---|---|---|---|
| flowshot-ui | 198 | 33,545 | wgpu-rendered UI: overlay, editor, pins, settings, launcher |
| flowshot-capture-wayland | 49 | 9,793 | Wayland capture backends and protocol clients |
| flowshot-daemon | 49 | 7,710 | Session daemon, D-Bus service, tray, shortcuts |
| flowshot-core | 10 | 3,985 | Geometry, scene model, config schema |
| flowshot-actions | 20 | 2,458 | Export actions (save, copy, pin, upload, notify) |
| flowshot-cli | 12 | 1,381 | Command-line surface |
| flowshot-capture | 9 | 1,194 | Capture backend trait and shared types |
| **Total** | **347** | **60,066** | |

### Flameshot, application source (`src/`, tokei "Code" column)

| Area | Files | Code |
|---|---|---|
| C++ sources (.cpp) | 113 | 16,469 |
| C++ headers (.h) | 112 | 3,650 |
| CMakeLists (build glue inside src/) | 9 | 642 |
| **Total** | **234** | **20,761** |

Qt-adjacent artifacts not in the code count: 4 `.ui` designer files, 86
`Q_OBJECT` headers that go through moc, and one `.qrc` resource manifest.

## Density and cleanliness metrics

Same formulas on both sides. "Comment ratio" is line comments divided by code
lines; FlowShot additionally reports rustdoc separately because tokei splits
doc comments out as Markdown children of Rust.

| Metric | FlowShot | Flameshot |
|---|---|---|
| Comment ratio (line comments / code) | 3.0% | 8.4% |
| Comment ratio (incl. rustdoc) | 23.0% | n/a |
| Avg file size | 173 code lines | 89 code lines |
| Median-ish shape | many mid-size modules, UI crate dominates | many small Qt widget pairs (.cpp + .h) |
| Largest file | `geometry.rs`, 1,339 code lines | `capturewidget.cpp`, 1,740 code lines |
| 2nd / 3rd largest | `scene.rs` 1,119; `editor/tests.rs` 980 (test file) | `generalconf.cpp` 857; `screengrabber.cpp` 719 |

Where FlowShot looks worse, said plainly:

- FlowShot's average file is nearly twice as long. Rust files pack module,
  types, impls, and tests into one file; C++ splits interface (.h) from
  implementation (.cpp), which halves the visible file count and size. Some
  of this is language shape, not cleanliness.
- `flowshot-ui` at 33.5k code lines is a monolith compared to anything in
  Flameshot. It is internally split into ~198 files, but the crate is doing
  the work Qt does for Flameshot, so the comparison is unkind by
  construction.
- FlowShot's largest files (`geometry.rs` 1,339, `scene.rs` 1,119) are well
  past the size this project would accept for new work, even though neither
  beats Flameshot's `capturewidget.cpp` (1,740).

Where FlowShot looks better:

- Tests. 1,213 test functions, ~13.5k lines in dedicated test files plus
  roughly 14k more inline, against two shell scripts upstream. Flameshot has
  no unit test suite in the repo at all; regressions there are caught by
  users.
- Documentation in code: 12,009 lines of rustdoc against sparse header
  comments (459 comment lines across 112 headers).
- No moc, no `.ui` XML, no resource compiler: the build graph is plain Cargo.

## Scope differences (read this before quoting the numbers)

"Less code" is partly density and partly scope, and the scope cut runs both
ways.

**Flameshot carries things FlowShot deliberately does not:**

- Four platforms. 156 `Q_OS_WIN` / `Q_OS_MAC` / `Q_OS_LINUX` conditionals
  across 33 files, X11-specific code in 6 files, Wayland workarounds in 12.
  FlowShot is Wayland-only v1; the porting gates live in
  `docs/architecture/adr-006-cross-platform-gates.md`.
- 49 translation files, 161,052 lines of Qt Linguist XML. FlowShot is
  English-only so far.
- Features FlowShot cut on purpose: update checker, upload history dialog,
  and the accumulated compatibility surface of a project shipping since 2017.
  FlowShot also does not do screen recording, OCR, or scroll capture.
- Vendored dependencies (`Qt-Color-Widgets`, `KDSingleApplication`) are
  pulled in as submodules/FetchContent. They are absent from the depth-1
  clone used here, so they are excluded automatically; counting them would
  make Flameshot bigger still.

**FlowShot carries things Flameshot gets for free or never built:**

- A GPU-rendered UI toolkit on wgpu (rendering, text, IME plumbing) because
  Qt was rejected for the overlay (see `docs/architecture/adr-003-ui-stack.md`).
- Direct Wayland protocol implementations with per-compositor quirk handling.
- A full test suite and the harness around it (parity matrix TOML, golden
  fixtures, stub D-Bus buses).
- 15 design and verification documents plus 6 ADRs.

## Methodology

Tool: `tokei v15.0.0` (neither tokei nor cloc was installed system-wide;
tokei was installed with `cargo install tokei --locked`). Same tool, same
counting rules on both sides. Code read-only; no files outside `docs/` were
touched.

Counting boundary: Flameshot's own code is `src/` (the app) plus `tests/`
(2 shell scripts). Excluded: `data/translations/` (49 Qt Linguist XML files,
counted separately for scope), `data/img/` (99 SVG icons), `docs/`,
`packaging/`, and the vendored `external/` subtree (absent in a depth-1
clone, noted above). FlowShot's own code is `crates/*/src` plus
`crates/*/tests`, `crates/*/examples`, `build.rs`, root `tests/`, and
`docs/`.

### Verbatim commands and outputs

Flameshot application source:

```text
$ tokei /tmp/flameshot-ref/src --sort code
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Language              Files        Lines         Code     Comments       Blanks
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 C++                     113        20155        16469         1227         2459
 C Header                112         5130         3650          459         1021
 CMake                     9          775          642           37           96
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Total                   234        26060        20761         1723         3576
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Flameshot tests (the entire directory):

```text
$ tokei /tmp/flameshot-ref/tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Language              Files        Lines         Code     Comments       Blanks
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Shell                     2          177          104           38           35
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Total                     2          177          104           38           35
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

FlowShot application source (all seven crate `src/` trees in one run):

```text
$ tokei crates/flowshot-core/src crates/flowshot-capture/src \
    crates/flowshot-capture-wayland/src crates/flowshot-ui/src \
    crates/flowshot-actions/src crates/flowshot-cli/src \
    crates/flowshot-daemon/src -t=Rust
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Language              Files        Lines         Code     Comments       Blanks
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Rust                    347        68075        60066         1826         6183
 |- Markdown             347        13268            0        12009         1259
 (Total)                            81343        60066        13835         7442
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 Total                   347        81343        60066        13835         7442
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

(The "Markdown" child row is rustdoc: `///` doc comments. tokei reports them
separately from `//` line comments.)

FlowShot dedicated test directories and in-source test files:

```text
$ tokei crates/flowshot-core/tests crates/flowshot-cli/tests \
    crates/flowshot-daemon/tests crates/flowshot-ui/tests -t=Rust
 Rust                     18         5393         4781          224          388

$ find crates -path "*/src/*" \( -name "tests.rs" -o -name "*_tests.rs" \
    -o -name "test_*.rs" \) | xargs tokei -t=Rust
 Rust                     19        10127         8721          665          741
```

Inline test estimate: 138 source files contain `#[cfg(test)]`; summing lines
from the marker to end of file gives ~15,916 raw lines (~14,000 code lines at
the tree's 0.88 code-to-line ratio). This is an estimate, not a tokei count.

Test functions: `grep -rn "#\[test\]" crates --include="*.rs"` gives 1,150,
plus 63 `#[tokio::test]`, total 1,213. The test runner reports 1,218; the
difference is doctests.

Function counts (both rough greps, caveated): FlowShot `fn` definitions in
`src/` excluding dedicated test files: 3,117. Flameshot `Class::method(`
definitions in `.cpp`: 1,111 (misses inline header definitions and
templates; treat as order-of-magnitude only).

Whole-repo totals (for the scope discussion):

```text
$ tokei . --exclude target          # FlowShot
 Total                   447        96890        71343        16631         8916

$ tokei /tmp/flameshot-ref          # Flameshot, incl. translations
 Total                   435       193227        184125         4422         4680
```

## Caveats

- **The comparison is not apples to apples.** Flameshot's code rides on Qt;
  FlowShot reimplements the toolkit layer. Comparing `src/` line counts
  without that context punishes FlowShot for a deliberate architectural
  choice.
- **Inline test attribution is estimated.** tokei cannot split a `.rs` file
  into test and non-test regions; the ~14k inline-test figure comes from the
  marker-to-EOF heuristic described above and slightly overcounts (it
  includes the module's blanks and comments).
- **Comment ratios measure culture, not quality.** Rust style pushes prose
  into rustdoc (counted separately here); Qt-era C++ uses `//` blocks. The
  3.0% vs 8.4% line-comment gap mostly reflects that split.
- **Depth-1 clone.** Flameshot history, submodules, and vendored code are
  not represented. Master @ `2d47806` is a moving target; rerun the commands
  to refresh.
- **The C++ function count is the weakest metric here** and is included only
  because it was cheap. Trust the line counts; treat the function counts as
  confirmation of the same trend.
