#![allow(clippy::type_complexity, clippy::map_clone, clippy::collapsible_if)]

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock};

/// Error type returned by fallible container lookups
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("Type '{0}' is not registered in the container")]
    NotRegistered(&'static str),
    #[error("Failed to downcast type '{0}'")]
    DowncastFailed(&'static str),
    #[error("Factory for '{0}' panicked during validation")]
    FactoryPanic(&'static str),
    #[error("Cyclic dependency detected: {}", .0.join(" → "))]
    CyclicDependency(Vec<&'static str>),
}

/// Marker trait for trait object components
/// Automatically implemented by #[component] on traits
pub trait DynComponent: Send + Sync + 'static {}

/// Trait for types that can be retrieved from the container
/// Automatically implemented by #[component] on traits
pub trait Injectable {
    fn get_from(container: &Container) -> Arc<Self>;
}

/// Trait for component registration
/// Automatically implemented by #[service] on impl structs
pub trait Component: Send + Sync + 'static {
    fn component_type_name() -> &'static str
    where
        Self: Sized;

    fn dependency_ids() -> Vec<TypeId>
    where
        Self: Sized;

    fn build(container: &Container) -> Arc<Self>
    where
        Self: Sized;

    fn register(container: &mut Container)
    where
        Self: Sized;
}

// Wrapper to make trait objects identifiable
pub struct TraitObject<T: ?Sized + 'static> {
    _phantom: PhantomData<T>,
}

impl<T: ?Sized + 'static> TraitObject<T> {
    pub fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }
}

pub struct Container {
    factories: HashMap<TypeId, Box<dyn Fn(&Container) -> Arc<dyn Any + Send + Sync> + Send + Sync>>,
    singletons: HashMap<TypeId, OnceLock<Arc<dyn Any + Send + Sync>>>,
    type_names: HashMap<TypeId, &'static str>,
    dependencies: HashMap<TypeId, Vec<TypeId>>,
}

impl Container {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            singletons: HashMap::new(),
            type_names: HashMap::new(),
            dependencies: HashMap::new(),
        }
    }

    /// Register a concrete component type with dependency tracking
    pub fn register_with_deps<T: Send + Sync + 'static>(
        &mut self,
        factory: fn(&Container) -> Arc<T>,
        type_name: &'static str,
        deps: Vec<TypeId>,
    ) {
        let type_id = TypeId::of::<T>();
        let wrapped_factory =
            Box::new(move |container: &Container| -> Arc<dyn Any + Send + Sync> {
                let instance = factory(container);
                instance as Arc<dyn Any + Send + Sync>
            });
        self.factories.insert(type_id, wrapped_factory);
        self.singletons.insert(type_id, OnceLock::new());
        self.type_names.insert(type_id, type_name);
        self.dependencies.insert(type_id, deps);
    }

    /// Register a trait object component with dependency tracking
    pub fn register_trait_with_deps<T: DynComponent + ?Sized>(
        &mut self,
        factory: fn(&Container) -> Arc<T>,
        type_name: &'static str,
        deps: Vec<TypeId>,
    ) where
        Arc<T>: Send + Sync,
    {
        let type_id = TraitObject::<T>::type_id();
        let wrapped_factory =
            Box::new(move |container: &Container| -> Arc<dyn Any + Send + Sync> {
                let instance = factory(container);
                Arc::new(instance) as Arc<dyn Any + Send + Sync>
            });
        self.factories.insert(type_id, wrapped_factory);
        self.singletons.insert(type_id, OnceLock::new());
        self.type_names.insert(type_id, type_name);
        self.dependencies.insert(type_id, deps);
    }

    /// Register a concrete component type
    pub fn register<T: Send + Sync + 'static>(&mut self, factory: fn(&Container) -> Arc<T>) {
        self.register_with_deps(factory, std::any::type_name::<T>(), vec![]);
    }

    /// Register a trait object component
    pub fn register_trait<T: DynComponent + ?Sized>(&mut self, factory: fn(&Container) -> Arc<T>)
    where
        Arc<T>: Send + Sync,
    {
        self.register_trait_with_deps(factory, std::any::type_name::<T>(), vec![]);
    }

    /// Register a pre-built instance
    pub fn register_instance<T: Send + Sync + 'static>(&mut self, instance: Arc<T>) {
        let type_id = TypeId::of::<T>();
        let instance_clone = instance.clone();
        let wrapped_factory = Box::new(
            move |_container: &Container| -> Arc<dyn Any + Send + Sync> {
                instance_clone.clone() as Arc<dyn Any + Send + Sync>
            },
        );
        self.factories.insert(type_id, wrapped_factory);
        let once_lock = OnceLock::new();
        once_lock.set(instance as Arc<dyn Any + Send + Sync>).ok();
        self.singletons.insert(type_id, once_lock);
        self.type_names.insert(type_id, std::any::type_name::<T>());
        self.dependencies.insert(type_id, vec![]);
    }

    /// Register a component using its generated impl
    pub fn register_component<T: Component>(&mut self) {
        T::register(self);
    }

    // ── Fallible accessors ────────────────────────────────────────────────

    /// Try to get a concrete component type; returns `Err` if not registered or downcast fails.
    pub fn try_get<T: Send + Sync + Clone + 'static>(&self) -> Result<Arc<T>, ContainerError> {
        let type_id = TypeId::of::<T>();
        let singleton = self
            .singletons
            .get(&type_id)
            .ok_or_else(|| ContainerError::NotRegistered(std::any::type_name::<T>()))?;
        let instance = singleton.get_or_init(|| {
            tracing::debug!("Lazy initializing {}", std::any::type_name::<T>());
            let factory = self
                .factories
                .get(&type_id)
                .unwrap_or_else(|| panic!("Factory for {} not found", std::any::type_name::<T>()));
            factory(self)
        });
        instance
            .clone()
            .downcast::<T>()
            .map_err(|_| ContainerError::DowncastFailed(std::any::type_name::<T>()))
    }

    /// Try to get a trait object component; returns `Err` if not registered or downcast fails.
    pub fn try_get_trait<T: DynComponent + ?Sized>(&self) -> Result<Arc<T>, ContainerError>
    where
        Arc<T>: Clone,
    {
        let type_id = TraitObject::<T>::type_id();
        let singleton = self
            .singletons
            .get(&type_id)
            .ok_or_else(|| ContainerError::NotRegistered(std::any::type_name::<T>()))?;
        let instance = singleton.get_or_init(|| {
            tracing::debug!("Lazy initializing trait {}", std::any::type_name::<T>());
            let factory = self.factories.get(&type_id).unwrap_or_else(|| {
                panic!("Factory for trait {} not found", std::any::type_name::<T>())
            });
            factory(self)
        });
        instance
            .downcast_ref::<Arc<T>>()
            .map(|a| a.clone())
            .ok_or_else(|| ContainerError::DowncastFailed(std::any::type_name::<T>()))
    }

    // ── Panicking accessors (delegates to try_get*) ───────────────────────

    /// Get a concrete component type. Panics if not registered.
    pub fn get<T: Send + Sync + Clone + 'static>(&self) -> Arc<T> {
        self.try_get::<T>().unwrap_or_else(|e| panic!("{}", e))
    }

    /// Get a trait object component. Panics if not registered.
    pub fn get_trait<T: DynComponent + ?Sized>(&self) -> Arc<T>
    where
        Arc<T>: Clone,
    {
        self.try_get_trait::<T>()
            .unwrap_or_else(|e| panic!("{}", e))
    }

    // ── Existence checks ──────────────────────────────────────────────────

    /// Check if a concrete type is registered
    pub fn has<T: 'static>(&self) -> bool {
        self.factories.contains_key(&TypeId::of::<T>())
    }

    /// Check if a trait object is registered
    pub fn has_trait<T: DynComponent + ?Sized>(&self) -> bool {
        self.factories.contains_key(&TraitObject::<T>::type_id())
    }

    pub fn len(&self) -> usize {
        self.factories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    // ── Startup validation ────────────────────────────────────────────────

    fn detect_cycles(&self) -> Vec<ContainerError> {
        use std::collections::HashMap as Map;

        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Gray,
            Black,
        }

        let mut colors: Map<TypeId, Color> = self
            .factories
            .keys()
            .map(|&id| (id, Color::White))
            .collect();
        let mut errors: Vec<ContainerError> = Vec::new();

        for &root in self.factories.keys() {
            if colors[&root] != Color::White {
                continue;
            }

            let mut stack: Vec<(TypeId, usize)> = vec![(root, 0)];
            let mut path: Vec<TypeId> = vec![root];
            colors.insert(root, Color::Gray);

            while let Some((node, child_idx)) = stack.last_mut() {
                let node = *node;
                let deps = self
                    .dependencies
                    .get(&node)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);

                if *child_idx >= deps.len() {
                    colors.insert(node, Color::Black);
                    stack.pop();
                    path.pop();
                    continue;
                }

                let dep = deps[*child_idx];
                *child_idx += 1;

                match colors.get(&dep).copied().unwrap_or(Color::White) {
                    Color::Gray => {
                        let start = path.iter().position(|&x| x == dep).unwrap_or(0);
                        let cycle: Vec<&'static str> = path[start..]
                            .iter()
                            .map(|&id| self.type_names.get(&id).copied().unwrap_or("unknown"))
                            .collect();
                        errors.push(ContainerError::CyclicDependency(cycle));
                    }
                    Color::White => {
                        colors.insert(dep, Color::Gray);
                        path.push(dep);
                        stack.push((dep, 0));
                    }
                    Color::Black => {}
                }
            }
        }

        errors
    }

    /// Eagerly resolve every registered singleton, collecting all errors.
    ///
    /// Call this after all registrations are complete and before serving traffic.
    /// Returns `Ok(())` if every factory succeeds, or `Err` with a list of all
    /// types that failed to resolve.
    pub fn validate(&self) -> Result<(), Vec<ContainerError>> {
        let mut errors = self.detect_cycles();

        if !errors.is_empty() {
            return Err(errors);
        }

        for (&type_id, singleton) in &self.singletons {
            if singleton.get().is_none() {
                if let Some(factory) = self.factories.get(&type_id) {
                    let name = self.type_names.get(&type_id).copied().unwrap_or("unknown");
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        singleton.get_or_init(|| factory(self));
                    }));
                    if result.is_err() {
                        errors.push(ContainerError::FactoryPanic(name));
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn dependency_graph(&self) -> DependencyGraph {
        let nodes: Vec<(&'static str, Vec<&'static str>)> = self
            .type_names
            .iter()
            .map(|(id, &name)| {
                let dep_names: Vec<&'static str> = self
                    .dependencies
                    .get(id)
                    .map(|dep_ids| {
                        dep_ids
                            .iter()
                            .map(|did| self.type_names.get(did).copied().unwrap_or("unknown"))
                            .collect()
                    })
                    .unwrap_or_default();
                (name, dep_names)
            })
            .collect();

        DependencyGraph { nodes }
    }
}

/// Represents the dependency graph of registered components
pub struct DependencyGraph {
    nodes: Vec<(&'static str, Vec<&'static str>)>,
}

impl std::fmt::Display for DependencyGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (name, deps) in &self.nodes {
            writeln!(f, "{}", name)?;
            for dep in deps {
                writeln!(f, "  └─ {}", dep)?;
            }
        }
        Ok(())
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}
