//! The `before_send` sanitization hook: the single funnel every event
//! passes through before it leaves the process.
//!
//! Three jobs, in order:
//!
//! 1. **Identifier strip** (unconditional): `server_name` (the default
//!    contexts integration fills it from the hostname), the `user` module,
//!    and `request`/env headers are removed from EVERY event, in both
//!    tiers - they are never part of the `FlowShot` payload.
//! 2. **Path scrubbing** (unconditional): the `scrub` module's rules
//!    (`~/...` home mapping, temp-basename DROP) applied to the event
//!    message/culprit, exception values, every stack-frame
//!    `filename`/`abs_path` (exceptions, the deprecated top-level
//!    stacktrace, and thread stacktraces), breadcrumb messages + data
//!    strings, and `extra` strings.
//! 3. **Tier filter**: without `include_technical_details` the technical
//!    contexts are dropped or scrubbed - the `os` context loses
//!    `kernel_version`/`version` (the exact kernel release is tier 2), any
//!    `gpu` context is removed, and a tier-2 `flowshot` context is removed
//!    even when an upstream producer smuggled one in. With the flag, the
//!    `flowshot` context carries the tier-2 payload.
//!
//! The taxonomy tags (tier 1) and the `surface` tag are injected here as
//! well, so they reach every event regardless of which thread captured it
//! (scope propagation across threads is NOT relied upon).

use std::borrow::Cow;
use std::sync::Arc;

use sentry::protocol::{Context, Event, Value};

use super::Facts;
use super::environment::Environment;
use super::scrub::scrub_paths;

/// The context key carrying the tier-2 technical payload.
const TECHNICAL_CONTEXT: &str = "flowshot";

/// The event processor: built once at init, shared with the `before_send`
/// closure through an [`Arc`].
pub(crate) struct Sanitizer {
    surface: &'static str,
    home: Option<String>,
    temp: Option<String>,
    facts: Arc<Facts>,
}

impl Sanitizer {
    pub(crate) fn new(surface: &'static str, facts: Arc<Facts>) -> Self {
        Self {
            surface,
            home: std::env::var("HOME").ok().filter(|home| !home.is_empty()),
            temp: temp_dir_string(),
            facts,
        }
    }

    /// The `before_send` processor. Sanitization scrubs; it never drops
    /// whole events (the error signal is the point of the report), so the
    /// `Option` the sentry callback contract demands is added at the
    /// closure site in [`super::init_with_transport`].
    pub(crate) fn sanitize(&self, mut event: Event<'static>) -> Event<'static> {
        event.server_name = None;
        event.user = None;
        event.request = None;

        if self.tier2() {
            self.inject_technical_context(&mut event);
        } else {
            strip_technical_contexts(&mut event);
        }
        self.scrub_event(&mut event);
        self.inject_tags(&mut event);
        event
    }

    /// Tier 2 is active exactly when the payload was probed at init.
    fn tier2(&self) -> bool {
        self.facts.payload.is_some()
    }

    fn scrub_event(&self, event: &mut Event<'static>) {
        if let Some(message) = event.message.take() {
            event.message = Some(self.scrub(&message).into_owned());
        }
        if let Some(culprit) = event.culprit.take() {
            event.culprit = Some(self.scrub(&culprit).into_owned());
        }
        for exception in &mut event.exception.values {
            if let Some(value) = exception.value.take() {
                exception.value = Some(self.scrub(&value).into_owned());
            }
            let stacktraces = [&mut exception.stacktrace, &mut exception.raw_stacktrace];
            for stacktrace in stacktraces.into_iter().flatten() {
                self.scrub_frames(&mut stacktrace.frames);
            }
        }
        if let Some(stacktrace) = &mut event.stacktrace {
            self.scrub_frames(&mut stacktrace.frames);
        }
        for thread in &mut event.threads.values {
            let stacktraces = [&mut thread.stacktrace, &mut thread.raw_stacktrace];
            for stacktrace in stacktraces.into_iter().flatten() {
                self.scrub_frames(&mut stacktrace.frames);
            }
        }
        for breadcrumb in &mut event.breadcrumbs.values {
            if let Some(message) = breadcrumb.message.take() {
                breadcrumb.message = Some(self.scrub(&message).into_owned());
            }
            for value in breadcrumb.data.values_mut() {
                scrub_value(value, &|text| self.scrub(text));
            }
        }
        for value in event.extra.values_mut() {
            scrub_value(value, &|text| self.scrub(text));
        }
    }

    fn scrub_frames(&self, frames: &mut [sentry::protocol::Frame]) {
        for frame in frames {
            if let Some(filename) = frame.filename.take() {
                frame.filename = Some(self.scrub(&filename).into_owned());
            }
            if let Some(abs_path) = frame.abs_path.take() {
                frame.abs_path = Some(self.scrub(&abs_path).into_owned());
            }
        }
    }

    fn scrub<'t>(&self, text: &'t str) -> Cow<'t, str> {
        scrub_paths(text, self.home.as_deref(), self.temp.as_deref())
    }

    fn inject_tags(&self, event: &mut Event<'static>) {
        let environment: &Environment = &self.facts.environment;
        let tags = [
            ("surface", self.surface),
            ("distro", environment.distro.as_str()),
            ("arch", environment.arch),
            ("package_manager", environment.package_manager.as_str()),
            ("session_type", environment.session_type.as_str()),
            ("desktop", environment.desktop.as_str()),
            ("gpu_family", environment.gpu_family.as_str()),
        ];
        for (key, value) in tags {
            event.tags.insert(key.to_owned(), value.to_owned());
        }
        if let Some(version) = &environment.compositor_version {
            event
                .tags
                .insert("compositor_version".to_owned(), version.clone());
        }
    }

    fn inject_technical_context(&self, event: &mut Event<'static>) {
        let Some(payload) = &self.facts.payload else {
            return;
        };
        let mut context = sentry::protocol::Map::new();
        context.insert(
            "kernel_release".to_owned(),
            Value::String(payload.kernel_release.clone()),
        );
        if let Some(monitors) = &payload.monitors {
            context.insert("monitors".to_owned(), Value::String(monitors.clone()));
        }
        if let Some(install_id) = &payload.install_id {
            context.insert("install_id".to_owned(), Value::String(install_id.clone()));
        }
        if let Some(gpu) = self.facts.gpu_adapter().or(payload.gpu_adapter.clone()) {
            context.insert("gpu_adapter".to_owned(), Value::String(gpu));
        }
        event
            .contexts
            .insert(TECHNICAL_CONTEXT.to_owned(), Context::Other(context));
    }
}

/// The tier-1 context filter: the `os` context degrades to its family
/// name, and GPU/technical contexts are removed outright.
fn strip_technical_contexts(event: &mut Event<'static>) {
    if let Some(Context::Os(os)) = event.contexts.get_mut("os") {
        os.kernel_version = None;
        os.version = None;
    }
    event.contexts.remove("gpu");
    event.contexts.remove(TECHNICAL_CONTEXT);
}

/// Recursively scrubs every string inside a JSON-ish protocol value.
fn scrub_value(value: &mut Value, scrub: &dyn Fn(&str) -> Cow<'_, str>) {
    match value {
        Value::String(text) => {
            let scrubbed = scrub(text);
            if let Cow::Owned(owned) = scrubbed {
                *text = owned;
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub_value(item, scrub);
            }
        }
        Value::Object(entries) => {
            for entry in entries.values_mut() {
                scrub_value(entry, scrub);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn temp_dir_string() -> Option<String> {
    let path = std::env::temp_dir();
    if path == std::path::Path::new("/tmp") {
        return None;
    }
    path.to_str()
        .map(str::to_owned)
        .filter(|temp| !temp.is_empty())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::RwLock;

    use sentry::protocol::{Breadcrumb, Exception, Frame, GpuContext, OsContext, Stacktrace};

    use super::*;
    use crate::telemetry::payload::Tier2Payload;

    fn synthetic_event() -> Event<'static> {
        let mut contexts = sentry::protocol::Map::new();
        contexts.insert(
            "os".to_owned(),
            Context::Os(Box::new(OsContext {
                name: Some("Linux".to_owned()),
                kernel_version: Some("#1 SMP PREEMPT_DYNAMIC".to_owned()),
                version: Some("6.10.5-arch1-1".to_owned()),
                ..Default::default()
            })),
        );
        contexts.insert(
            "gpu".to_owned(),
            Context::Gpu(Box::new(GpuContext {
                name: "NVIDIA GeForce RTX 3080 Ti".to_owned(),
                ..Default::default()
            })),
        );
        let mut technical = sentry::protocol::Map::new();
        technical.insert(
            "gpu_adapter".to_owned(),
            Value::String("NVIDIA GeForce RTX 3080 Ti".to_owned()),
        );
        contexts.insert(TECHNICAL_CONTEXT.to_owned(), Context::Other(technical));
        Event {
            server_name: Some("alice-workstation".into()),
            message: Some("failed on /home/alice/secret/file.png".to_owned()),
            exception: vec![Exception {
                ty: "ExecuteError".to_owned(),
                value: Some("export failed for /home/alice/secret/file.png".to_owned()),
                module: None,
                stacktrace: Some(Stacktrace {
                    frames: vec![Frame {
                        filename: Some("/home/alice/src/flowshot/main.rs".to_owned()),
                        abs_path: Some("/tmp/flowshot-build/x.rs".to_owned()),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }]
            .into(),
            breadcrumbs: vec![Breadcrumb {
                message: Some("saving /home/alice/secret/file.png".to_owned()),
                ..Default::default()
            }]
            .into(),
            contexts,
            ..Default::default()
        }
    }

    fn facts(tier2: bool) -> Arc<Facts> {
        Arc::new(Facts {
            environment: Arc::new(Environment {
                distro: "Arch Linux".to_owned(),
                arch: "x86_64",
                package_manager: "pacman".to_owned(),
                session_type: "wayland".to_owned(),
                desktop: "hyprland".to_owned(),
                compositor_version: Some("0.56.2".to_owned()),
                gpu_family: "nvidia".to_owned(),
            }),
            payload: tier2.then(|| {
                Arc::new(Tier2Payload {
                    kernel_release: "6.10.5-arch1-1".to_owned(),
                    monitors: Some("DP-1 2560x1440@1.5".to_owned()),
                    install_id: Some("0e1c9f2a-0000-4000-8000-000000000000".to_owned()),
                    gpu_adapter: Some("nvidia 10de:2204".to_owned()),
                })
            }),
            gpu_slot: Arc::new(RwLock::new(None)),
        })
    }

    #[test]
    fn tier1_strips_identifiers_paths_and_technical_contexts() {
        // Given: a synthetic event with a home path, a server name, and
        // the full GPU string in both the gpu and flowshot contexts.
        let event = synthetic_event();
        // When: the tier-1 sanitizer runs.
        let sanitized = Sanitizer::new("cli", facts(false)).sanitize(event);
        // Then: identifiers are gone, paths are scrubbed, GPU detail is
        // gone, the os context keeps its name but loses the kernel, and
        // the tier-1 tags are present.
        assert_eq!(sanitized.server_name, None);
        assert_eq!(sanitized.user, None);
        assert_eq!(sanitized.request, None);
        assert_eq!(
            sanitized.message.as_deref(),
            Some("failed on ~/secret/file.png")
        );
        let exception = &sanitized.exception.values[0];
        assert_eq!(
            exception.value.as_deref(),
            Some("export failed for ~/secret/file.png")
        );
        let frame = &exception.stacktrace.as_ref().unwrap().frames[0];
        assert_eq!(frame.filename.as_deref(), Some("~/src/flowshot/main.rs"));
        assert_eq!(frame.abs_path.as_deref(), Some("/tmp/<redacted>"));
        assert_eq!(
            sanitized.breadcrumbs.values[0].message.as_deref(),
            Some("saving ~/secret/file.png")
        );
        assert!(!sanitized.contexts.contains_key("gpu"));
        assert!(!sanitized.contexts.contains_key(TECHNICAL_CONTEXT));
        let Context::Os(os) = &sanitized.contexts["os"] else {
            panic!("os context must survive as Os");
        };
        assert_eq!(os.name.as_deref(), Some("Linux"));
        assert_eq!(os.kernel_version, None);
        assert_eq!(os.version, None);
        assert_eq!(sanitized.tags["surface"], "cli");
        assert_eq!(sanitized.tags["distro"], "Arch Linux");
        assert_eq!(sanitized.tags["gpu_family"], "nvidia");
        assert_eq!(sanitized.tags["compositor_version"], "0.56.2");
        let json = serde_json::to_string(&sanitized).unwrap();
        assert!(!json.contains("RTX 3080"), "tier 1 leaks no GPU detail");
        assert!(!json.contains("6.10.5"), "tier 1 leaks no kernel release");
        assert!(!json.contains("alice-workstation"));
        assert!(!json.contains("/home/alice"));
    }

    #[test]
    fn tier2_keeps_the_technical_payload_but_still_strips_identifiers() {
        // Given: the same synthetic event.
        let event = synthetic_event();
        // When: the tier-2 sanitizer runs.
        let sanitized = Sanitizer::new("cli", facts(true)).sanitize(event);
        // Then: identifiers and paths are STILL stripped (unconditional),
        // the os kernel survives, and the flowshot context carries the
        // tier-2 payload (the injection replaces the smuggled context).
        assert_eq!(sanitized.server_name, None);
        assert_eq!(
            sanitized.message.as_deref(),
            Some("failed on ~/secret/file.png")
        );
        let Context::Os(os) = &sanitized.contexts["os"] else {
            panic!("os context must survive as Os");
        };
        assert_eq!(os.version.as_deref(), Some("6.10.5-arch1-1"));
        let Context::Other(technical) = &sanitized.contexts[TECHNICAL_CONTEXT] else {
            panic!("tier 2 must inject the flowshot context");
        };
        assert_eq!(
            technical["kernel_release"],
            Value::String("6.10.5-arch1-1".to_owned())
        );
        assert_eq!(
            technical["monitors"],
            Value::String("DP-1 2560x1440@1.5".to_owned())
        );
        assert_eq!(
            technical["gpu_adapter"],
            Value::String("nvidia 10de:2204".to_owned())
        );
        assert!(technical.get("install_id").is_some());
    }

    #[test]
    fn the_gpu_slot_pass_through_wins_over_the_sysfs_fallback() {
        let facts = facts(true);
        *facts.gpu_slot.write().unwrap() = Some("NVIDIA GeForce RTX 3080 Ti".to_owned());
        let sanitized = Sanitizer::new("overlay", facts).sanitize(Event::default());
        let Context::Other(technical) = &sanitized.contexts[TECHNICAL_CONTEXT] else {
            panic!("tier 2 must inject the flowshot context");
        };
        assert_eq!(
            technical["gpu_adapter"],
            Value::String("NVIDIA GeForce RTX 3080 Ti".to_owned())
        );
    }
}
