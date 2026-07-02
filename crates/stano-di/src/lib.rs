pub mod application_context;
pub mod container;
pub mod environment;

// Re-export commonly used types
pub use container::{Component, Container, ContainerError, DynComponent, Injectable, TraitObject};

#[doc(hidden)]
pub use inventory;

pub struct ServiceRegistration(pub fn(&mut container::Container));
inventory::collect!(ServiceRegistration);

pub fn register_all(container: &mut container::Container) {
    for reg in inventory::iter::<ServiceRegistration>() {
        (reg.0)(container);
    }
}
