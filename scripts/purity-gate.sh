#!/usr/bin/env bash
# Platform purity gate (plan todo 42a, ADR-006, draft D2/D5/F20).
#
# Asserts the platform-free crates carry ZERO platform coupling in lib code:
#
#   gated crates:  flowshot-core, flowshot-capture, flowshot-ui
#   platform crates: flowshot-capture-wayland (everything compositor-facing)
#                   and flowshot-capture-x11 (everything X-server-facing);
#                   neither is gated. Every future platform sibling must be
#                   added to BOTH banned lists below.
#   out of scope:  flowshot-cli / flowshot-daemon / flowshot-actions are the
#                  binary/wiring layer and the platform-native clipboard
#                  owner by design (Wayland via wl-clipboard-rs, X11 via
#                  x11rb ICCCM; plan todo 1, Oracle r2 F-5, ADR-004/006)
#
# Checks per gated crate:
#   1. Cargo.toml: no platform crate in ANY *dependencies section, except
#      entries recorded in scripts/purity-allowlist.txt (today: flowshot-ui's
#      two dev-dependencies used for QA-harness composition in examples/ and
#      tests/ only - the lib never imports them).
#   2. Cargo.toml: no [target.'cfg(...)'.dependencies] platform-conditional
#      dependency tables.
#   3. src/**/*.rs (lib code, inline tests included; crate tests/ and
#      examples/ are dev surfaces, out of scope): no `use`/`extern crate`/
#      path references to Wayland, X11, D-Bus, PipeWire, or unix-FFI crates;
#      no winit platform-extension imports (`winit::platform::*`, `*ExtWayland`
#      ...); no cfg(target_os/target_family/target_arch/target_env/unix/
#      windows/macos). Full-line `//` comments are stripped before matching
#      (doc comments may NAME the forbidden seam, e.g. the WindowCustomizer
#      rationale); code lines are matched case-sensitively, so portable
#      env-var probes ("WAYLAND_DISPLAY" via std::env::var_os), platform
#      enum vocabulary (BackendKind::X11, serde rename "x11"), and
#      diagnostic string literals naming a banned platform crate (e.g.
#      "X11 (xcb GetImage)", where `xcb` is followed by a space, not `::`)
#      stay invisible by construction - they are values and identifiers,
#      not imports.
#
# winit itself is NOT banned: it is the cross-platform windowing seam the UI
# is built on (draft D1, ADR-003). The plan's gate list is "wayland/x11
# imports or cfg(target_os)"; the winit vector that matters is its
# platform-extension modules, which ARE banned.
#
# Allowlist format (scripts/purity-allowlist.txt): `crate|location|substring`
# - a reported line is allowlisted when it contains all three fields. Every
# allowlist entry must carry a `#` comment line above it recording WHY.
#
# Exit 0 = pure. Exit 1 = violations (printed as `VIOLATION ...`).
# Usage: scripts/purity-gate.sh
set -uo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
allowlist_file="$repo_root/scripts/purity-allowlist.txt"
gated_crates=(flowshot-core flowshot-capture flowshot-ui)

# Platform crates, Cargo.toml package-name form (hyphens).
banned_deps_alt='wayland-client|wayland-protocols|wayland-protocols-wlr|wayland-backend|wayland-scanner|smithay-client-toolkit|sctk|calloop|calloop-wayland-source|wl-clipboard-rs|x11|x11rb|x11rb-protocol|xcb|xcap|xkbcommon|zbus|zvariant|ashpd|ksni|dbus|notify-rust|nix|rustix|libc|memfd|pipewire|pipewire-sys|libspa|libspa-sys|fontconfig|flowshot-capture-wayland|flowshot-capture-x11|flowshot-actions'

# Same crates, Rust identifier form (underscores), import-shaped patterns.
banned_ids_alt='wayland_[a-z_]+|wl_clipboard[a-z_]*|smithay[a-z_]*|sctk[a-z_]*|calloop[a-z_]*|x11[a-z_0-9]*|xcb|xkbcommon|zbus|zvariant|ashpd|ksni|dbus|notify_rust|nix|rustix|libc|memfd|pipewire|libspa|fontconfig|flowshot_capture_wayland|flowshot_capture_x11|flowshot_actions'

src_patterns=(
    "(^|[^A-Za-z0-9_])(pub[[:space:]]+)?(use|extern[[:space:]]+crate)[[:space:]]+(${banned_ids_alt})(::|[[:space:]]*;|[[:space:]]+as[[:space:]]|\\\{)"
    "(^|[^A-Za-z0-9_])(${banned_ids_alt})::"
    "winit::platform::"
    "Ext(Wayland|X11|Android|Ios|Macos|Windows|Web|Orbital)([^A-Za-z0-9_]|\$)"
    "cfg(_attr)?[[:space:]]*\([^)]*(target_os|target_family|target_arch|target_env|target_abi|unix|windows|macos)"
)

violations=0
allowlisted_hits=0

allowlisted() { # <report-line>
    [[ -f "$allowlist_file" ]] || return 1
    local line="$1" entry crate loc sub
    while IFS='|' read -r crate loc sub; do
        [[ -z "${crate:-}" || "$crate" == \#* ]] && continue
        if [[ "$line" == *"$crate"* && "$line" == *"$loc"* && "$line" == *"$sub"* ]]; then
            return 0
        fi
    done < "$allowlist_file"
    return 1
}

report() { # <report-line>
    if allowlisted "$1"; then
        printf 'ALLOWLISTED %s\n' "$1"
        allowlisted_hits=$((allowlisted_hits + 1))
    else
        printf 'VIOLATION   %s\n' "$1"
        violations=$((violations + 1))
    fi
}

for crate in "${gated_crates[@]}"; do
    crate_dir="$repo_root/crates/$crate"
    if [[ ! -d "$crate_dir" ]]; then
        printf 'VIOLATION   %s: crate directory missing\n' "$crate"
        violations=$((violations + 1))
        continue
    fi

    # 1+2. Cargo.toml dependency sections and target-cfg tables.
    while IFS= read -r hit; do
        report "$crate Cargo.toml $hit"
    done < <(
        awk -v banned="$banned_deps_alt" '
            /^\[/ {
                sec = $0
                if (sec ~ /^\[target\./ && sec ~ /cfg\(/ && sec ~ /dependencies\]/)
                    printf "%s: platform-conditional dependency table %s\n", sec, sec
                next
            }
            sec ~ /dependencies\]/ && $0 ~ ("^[ \t]*(" banned ")([ \t]*=|\\.)") {
                printf "%s: %s\n", sec, $0
            }
        ' "$crate_dir/Cargo.toml"
    )

    # 3. lib sources, full-line comments stripped, line numbers preserved.
    while IFS= read -r hit; do
        report "$crate $hit"
    done < <(
        find "$crate_dir/src" -name '*.rs' -print0 |
            xargs -0 awk '
                { code = $0; sub(/^[ \t]*\/\/.*/, "", code) }
                code ~ /[^ \t]/ { printf "%s:%d:%s\n", FILENAME, FNR, code }
            ' |
            {
                out="$(mktemp)"
                cat > "$out"
                for pat in "${src_patterns[@]}"; do
                    grep -E "$pat" "$out" || true
                done
                rm -f "$out"
            } | sort -u
    )
done

printf '\npurity-gate: %d crate(s) gated, %d violation(s), %d allowlisted hit(s)\n' \
    "${#gated_crates[@]}" "$violations" "$allowlisted_hits"
if (( violations > 0 )); then
    printf 'purity-gate: FAIL - platform coupling leaked into a platform-free crate\n'
    exit 1
fi
printf 'purity-gate: PASS - core/capture/ui are platform-free\n'
exit 0
