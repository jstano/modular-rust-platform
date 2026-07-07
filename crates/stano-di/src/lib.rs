//! Lightweight dependency injection container with lazy singleton resolution,
//! cycle detection, and async-safe validation.
#![warn(missing_docs)]

/// App-level wiring façade combining a [`container::Container`] with an environment.
pub mod application_context;
/// The [`container::Container`] type and its registration/retrieval traits.
pub mod container;
/// Abstraction over configuration/environment variable lookup.
pub mod environment;

// Re-export commonly used types
pub use container::{Component, Container, ContainerError, DynComponent, Injectable, TraitObject};

#[doc(hidden)]
pub use inventory;

/// A component registration collected via `#[service]`'s `inventory::submit!`, applied by [`register_all`].
pub struct ServiceRegistration(
    /// Callback that registers the component's factory into the container.
    pub fn(&mut container::Container),
);
inventory::collect!(ServiceRegistration);

/// Register every component collected via `#[service]`'s `inventory::submit!` into `container`.
pub fn register_all(container: &mut container::Container) {
    for reg in inventory::iter::<ServiceRegistration>() {
        (reg.0)(container);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[derive(Clone)]
    struct LibTestWidget;

    static LIB_TEST_REGISTRATION_CALLED: AtomicBool = AtomicBool::new(false);

    fn lib_test_widget_factory(_c: &container::Container) -> Arc<LibTestWidget> {
        Arc::new(LibTestWidget)
    }

    fn lib_test_registration(container: &mut container::Container) {
        LIB_TEST_REGISTRATION_CALLED.store(true, Ordering::SeqCst);
        container.register(lib_test_widget_factory);
    }

    inventory::submit! {
        ServiceRegistration(lib_test_registration)
    }

    #[test]
    fn test_register_all_invokes_manually_submitted_registration() {
        let mut container = container::Container::new();
        register_all(&mut container);
        assert!(LIB_TEST_REGISTRATION_CALLED.load(Ordering::SeqCst));
        assert!(container.has::<LibTestWidget>());
        let _ = container.get::<LibTestWidget>();
    }
}
