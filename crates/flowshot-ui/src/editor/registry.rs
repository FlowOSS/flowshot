//! The tool registry (the F27 factory-switch equivalent).
//!
//! Flameshot's `ToolFactory` is a compile-time switch over the type enum;
//! `FlowShot` inverts it: the binary layer and the QA harnesses
//! REGISTER factories per [`ToolKind`] - concrete tools land incrementally
//! without the framework knowing them, and an unregistered
//! kind is a typed `None` (warn-logged at activation), never a panic.
//! Factories are plain `fn` pointers: tools are default-constructible and
//! receive color/size/config through the [`Tool`] lifecycle hooks.

use std::collections::HashMap;

use super::kind::ToolKind;
use super::tool::Tool;

/// A tool constructor (no captures - the registry stays `Debug + Clone`).
pub type ToolFactory = fn() -> Box<dyn Tool>;

/// Kind -> factory map. Empty by construction; the composition root
/// registers the tools the build ships.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    factories: HashMap<ToolKind, ToolFactory>,
}

impl ToolRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) the factory for `kind`.
    pub fn register(&mut self, kind: ToolKind, factory: ToolFactory) {
        self.factories.insert(kind, factory);
    }

    /// Whether `kind` has a registered factory.
    #[must_use]
    pub fn contains(&self, kind: ToolKind) -> bool {
        self.factories.contains_key(&kind)
    }

    /// Creates a fresh tool instance for `kind`; `None` when unregistered.
    #[must_use]
    pub fn create(&self, kind: ToolKind) -> Option<Box<dyn Tool>> {
        self.factories.get(&kind).map(|factory| factory())
    }

    /// The registered kinds, in [`ToolKind::ALL`] order (the toolbar/shortcut
    /// iteration order).
    #[must_use]
    pub fn registered(&self) -> Vec<ToolKind> {
        ToolKind::ALL
            .into_iter()
            .filter(|kind| self.contains(*kind))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[derive(Debug)]
    struct Stub;
    impl Tool for Stub {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }

        fn kind(&self) -> ToolKind {
            ToolKind::Pencil
        }
    }

    #[test]
    fn unregistered_kind_creates_nothing() {
        let registry = ToolRegistry::new();
        assert!(registry.create(ToolKind::Pencil).is_none());
        assert!(!registry.contains(ToolKind::Pencil));
        assert!(registry.registered().is_empty());
    }

    #[test]
    fn register_create_replace_roundtrip() {
        let mut registry = ToolRegistry::new();
        registry.register(ToolKind::Pencil, || Box::new(Stub));
        assert!(registry.contains(ToolKind::Pencil));
        assert_eq!(
            registry.create(ToolKind::Pencil).map(|t| t.kind()),
            Some(ToolKind::Pencil)
        );
        // Fresh instance per create (Flameshot copy-per-stroke model).
        let (a, b) = (
            registry.create(ToolKind::Pencil),
            registry.create(ToolKind::Pencil),
        );
        assert!(a.is_some() && b.is_some());
        // Replace is idempotent-safe.
        registry.register(ToolKind::Pencil, || Box::new(Stub));
        assert_eq!(registry.registered(), vec![ToolKind::Pencil]);
    }
}
