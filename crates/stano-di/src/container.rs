#![allow(clippy::type_complexity, clippy::map_clone, clippy::collapsible_if)]

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock};

/// Error type returned by fallible container lookups
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    /// No factory was registered for the requested type.
    #[error("Type '{0}' is not registered in the container")]
    NotRegistered(&'static str),
    /// The stored instance could not be downcast to the requested type.
    #[error("Failed to downcast type '{0}'")]
    DowncastFailed(&'static str),
    /// The factory function panicked while building the instance.
    #[error("Factory for '{0}' panicked during validation")]
    FactoryPanic(&'static str),
    /// A dependency cycle was detected among the listed type names.
    #[error("Cyclic dependency detected: {}", .0.join(" → "))]
    CyclicDependency(Vec<&'static str>),
}

/// Marker trait for trait object components
/// Automatically implemented by `#\[component\]` on traits
pub trait DynComponent: Send + Sync + 'static {}

/// Trait for types that can be retrieved from the container
/// Automatically implemented by `#\[component\]` on traits
pub trait Injectable {
    /// Resolve `Self` from the given container.
    fn get_from(container: &Container) -> Arc<Self>;
}

/// Trait for component registration
/// Automatically implemented by `#\[service\]` on impl structs
pub trait Component: Send + Sync + 'static {
    /// The type name used for diagnostics (cycle detection, error messages).
    fn component_type_name() -> &'static str
    where
        Self: Sized;

    /// The `TypeId`s of this component's dependencies, for cycle detection.
    fn dependency_ids() -> Vec<TypeId>
    where
        Self: Sized;

    /// Construct an instance by resolving its dependencies from the container.
    fn build(container: &Container) -> Arc<Self>
    where
        Self: Sized;

    /// Register this component's factory into the container.
    fn register(container: &mut Container)
    where
        Self: Sized;
}

/// Wrapper used to derive a stable [`TypeId`] for trait object components,
/// since `TypeId::of::<dyn Trait>()` is not directly available.
pub struct TraitObject<T: ?Sized + 'static> {
    _phantom: PhantomData<T>,
}

impl<T: ?Sized + 'static> TraitObject<T> {
    /// The stable `TypeId` used to key `dyn T` in the container's maps.
    pub fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }
}

/// TypeId-keyed registry of component factories and lazily-initialized singletons.
///
/// Components are registered with a factory closure and resolved lazily on first
/// access via [`Container::get`]/[`Container::try_get`] (or their `_trait` counterparts),
/// with the result cached as a singleton for subsequent lookups.
pub struct Container {
    factories: HashMap<TypeId, Box<dyn Fn(&Container) -> Arc<dyn Any + Send + Sync> + Send + Sync>>,
    singletons: HashMap<TypeId, OnceLock<Arc<dyn Any + Send + Sync>>>,
    type_names: HashMap<TypeId, &'static str>,
    dependencies: HashMap<TypeId, Vec<TypeId>>,
}

impl Container {
    /// Create an empty container with no registered components.
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

    /// Number of components registered in the container.
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Whether no components are registered in the container.
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

    /// Build a snapshot of the registered components and their dependencies, for diagnostics.
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

/// Equivalent to [`Container::new`].
impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn test_container_is_send_sync() {
        assert_send_sync::<Container>();
    }

    #[test]
    fn test_container_default_equals_new() {
        let container = Container::default();
        assert!(container.is_empty());
        assert_eq!(container.len(), 0);
    }

    #[derive(Clone)]
    struct Widget;

    fn widget_factory(_c: &Container) -> Arc<Widget> {
        Arc::new(Widget)
    }

    #[test]
    fn test_get_unregistered_type_returns_not_registered() {
        let container = Container::new();
        let result = container.try_get::<Widget>();
        assert!(matches!(result, Err(ContainerError::NotRegistered(_))));
    }

    #[test]
    fn test_get_panics_on_missing_registration() {
        let container = Container::new();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| container.get::<Widget>()))
                .is_err()
        );
    }

    #[test]
    fn test_try_get_returns_result() {
        let container = Container::new();
        assert!(container.try_get::<Widget>().is_err());
    }

    #[test]
    fn test_singleton_reuses_same_arc_instance() {
        let mut container = Container::new();
        container.register(widget_factory);
        let a = container.get::<Widget>();
        let b = container.get::<Widget>();
        assert!(Arc::ptr_eq(&a, &b));
    }

    static FACTORY_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone)]
    struct Counted;

    fn counted_factory(_c: &Container) -> Arc<Counted> {
        FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);
        Arc::new(Counted)
    }

    #[test]
    fn test_factory_invoked_only_once() {
        FACTORY_CALLS.store(0, Ordering::SeqCst);
        let mut container = Container::new();
        container.register(counted_factory);
        let _a = container.get::<Counted>();
        let _b = container.get::<Counted>();
        assert_eq!(FACTORY_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_register_instance_returns_exact_arc_without_factory() {
        let mut container = Container::new();
        let instance = Arc::new(Widget);
        container.register_instance(instance.clone());
        let resolved = container.get::<Widget>();
        assert!(Arc::ptr_eq(&instance, &resolved));
    }

    trait Greeter: DynComponent {
        fn greet(&self) -> String;
    }

    struct EnglishGreeter;

    impl DynComponent for EnglishGreeter {}

    impl Greeter for EnglishGreeter {
        fn greet(&self) -> String {
            "hello".to_string()
        }
    }

    fn greeter_factory(_c: &Container) -> Arc<dyn Greeter> {
        Arc::new(EnglishGreeter)
    }

    #[test]
    fn test_get_trait_panics_on_missing_registration() {
        let container = Container::new();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || container.get_trait::<dyn Greeter>()
            ))
            .is_err()
        );
    }

    #[test]
    fn test_try_get_trait_returns_ok_when_registered() {
        let mut container = Container::new();
        container.register_trait(greeter_factory);
        let greeter = container.try_get_trait::<dyn Greeter>().unwrap();
        assert_eq!(greeter.greet(), "hello");
    }

    #[test]
    fn test_get_trait_returns_registered_instance() {
        let mut container = Container::new();
        container.register_trait(greeter_factory);
        let greeter = container.get_trait::<dyn Greeter>();
        assert_eq!(greeter.greet(), "hello");
    }

    // Mirrors what `#[component]` generates for a trait, to test `Injectable`
    // directly rather than only through macro-generated code.
    impl Injectable for dyn Greeter {
        fn get_from(container: &Container) -> Arc<Self> {
            container.get_trait::<dyn Greeter>()
        }
    }

    #[test]
    fn test_injectable_get_from_delegates_to_container_get_trait() {
        let mut container = Container::new();
        container.register_trait(greeter_factory);
        let greeter = <dyn Greeter as Injectable>::get_from(&container);
        assert_eq!(greeter.greet(), "hello");
    }

    #[test]
    fn test_injectable_get_from_panics_when_unregistered() {
        let container = Container::new();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                <dyn Greeter as Injectable>::get_from(&container)
            }))
            .is_err()
        );
    }

    fn panics_factory_a(_c: &Container) -> Arc<Widget> {
        panic!("boom a");
    }

    #[derive(Clone)]
    struct OtherPanicking;

    fn panics_factory_b(_c: &Container) -> Arc<OtherPanicking> {
        panic!("boom b");
    }

    #[test]
    fn test_validate_eager_resolves_and_aggregates_multiple_errors() {
        let mut container = Container::new();
        container.register(panics_factory_a);
        container.register(panics_factory_b);
        let errors = container.validate().unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(
            errors
                .iter()
                .all(|e| matches!(e, ContainerError::FactoryPanic(_)))
        );
    }

    #[derive(Clone)]
    struct Healthy(i32);

    fn healthy_factory(_c: &Container) -> Arc<Healthy> {
        Arc::new(Healthy(1))
    }

    #[test]
    fn test_factory_panic_caught_by_validate_without_poisoning_other_entries() {
        let mut container = Container::new();
        container.register(panics_factory_a);
        container.register(healthy_factory);
        assert!(container.validate().is_err());
        let healthy = container.get::<Healthy>();
        assert_eq!(healthy.0, 1);
    }

    #[test]
    fn test_validate_succeeds_when_all_factories_are_healthy() {
        let mut container = Container::new();
        container.register(healthy_factory);
        assert!(container.validate().is_ok());
    }

    #[derive(Clone)]
    struct CycleA;
    #[derive(Clone)]
    struct CycleB;
    #[derive(Clone)]
    struct CycleC;

    fn cycle_a_factory(c: &Container) -> Arc<CycleA> {
        let _ = c.get::<CycleB>();
        Arc::new(CycleA)
    }

    fn cycle_b_factory(c: &Container) -> Arc<CycleB> {
        let _ = c.get::<CycleA>();
        Arc::new(CycleB)
    }

    #[test]
    fn test_cycle_detection_two_node() {
        let mut container = Container::new();
        container.register_with_deps(cycle_a_factory, "CycleA", vec![TypeId::of::<CycleB>()]);
        container.register_with_deps(cycle_b_factory, "CycleB", vec![TypeId::of::<CycleA>()]);
        let errors = container.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ContainerError::CyclicDependency(_)))
        );
    }

    fn cycle3_a_factory(c: &Container) -> Arc<CycleA> {
        let _ = c.get::<CycleB>();
        Arc::new(CycleA)
    }
    fn cycle3_b_factory(c: &Container) -> Arc<CycleB> {
        let _ = c.get::<CycleC>();
        Arc::new(CycleB)
    }
    fn cycle3_c_factory(c: &Container) -> Arc<CycleC> {
        let _ = c.get::<CycleA>();
        Arc::new(CycleC)
    }

    #[test]
    fn test_cycle_detection_three_node() {
        let mut container = Container::new();
        container.register_with_deps(cycle3_a_factory, "CycleA", vec![TypeId::of::<CycleB>()]);
        container.register_with_deps(cycle3_b_factory, "CycleB", vec![TypeId::of::<CycleC>()]);
        container.register_with_deps(cycle3_c_factory, "CycleC", vec![TypeId::of::<CycleA>()]);
        let errors = container.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ContainerError::CyclicDependency(_)))
        );
    }

    #[derive(Clone)]
    struct SelfCycle;

    fn self_cycle_factory(c: &Container) -> Arc<SelfCycle> {
        let _ = c.get::<SelfCycle>();
        Arc::new(SelfCycle)
    }

    #[test]
    fn test_cycle_detection_self_cycle() {
        let mut container = Container::new();
        container.register_with_deps(
            self_cycle_factory,
            "SelfCycle",
            vec![TypeId::of::<SelfCycle>()],
        );
        let errors = container.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ContainerError::CyclicDependency(_)))
        );
    }

    #[derive(Clone)]
    struct Leaf;
    #[derive(Clone)]
    struct Root;

    fn leaf_factory(_c: &Container) -> Arc<Leaf> {
        Arc::new(Leaf)
    }
    fn root_factory(c: &Container) -> Arc<Root> {
        let _ = c.get::<Leaf>();
        Arc::new(Root)
    }

    #[test]
    fn test_dependency_graph_display_formatting() {
        let mut container = Container::new();
        container.register_with_deps(leaf_factory, "Leaf", vec![]);
        container.register_with_deps(root_factory, "Root", vec![TypeId::of::<Leaf>()]);
        let text = container.dependency_graph().to_string();
        assert!(text.contains("Root"));
        assert!(text.contains("Leaf"));
        assert!(text.contains("└─"));
        assert!(container.validate().is_ok());
    }

    #[derive(Clone)]
    struct DiamondA;
    #[derive(Clone)]
    struct DiamondB;
    #[derive(Clone)]
    struct DiamondC;
    #[derive(Clone)]
    struct DiamondD;

    fn diamond_d_factory(_c: &Container) -> Arc<DiamondD> {
        Arc::new(DiamondD)
    }
    fn diamond_b_factory(c: &Container) -> Arc<DiamondB> {
        let _ = c.get::<DiamondD>();
        Arc::new(DiamondB)
    }
    fn diamond_c_factory(c: &Container) -> Arc<DiamondC> {
        let _ = c.get::<DiamondD>();
        Arc::new(DiamondC)
    }
    fn diamond_a_factory(c: &Container) -> Arc<DiamondA> {
        let _ = c.get::<DiamondB>();
        let _ = c.get::<DiamondC>();
        Arc::new(DiamondA)
    }

    #[test]
    fn test_validate_succeeds_for_diamond_shaped_dependency_graph() {
        let mut container = Container::new();
        container.register_with_deps(diamond_d_factory, "DiamondD", vec![]);
        container.register_with_deps(
            diamond_b_factory,
            "DiamondB",
            vec![TypeId::of::<DiamondD>()],
        );
        container.register_with_deps(
            diamond_c_factory,
            "DiamondC",
            vec![TypeId::of::<DiamondD>()],
        );
        container.register_with_deps(
            diamond_a_factory,
            "DiamondA",
            vec![TypeId::of::<DiamondB>(), TypeId::of::<DiamondC>()],
        );
        assert!(container.validate().is_ok());
    }

    struct UnregisteredMarker;

    #[test]
    fn test_dependency_graph_unknown_type_id_fallback() {
        let mut container = Container::new();
        container.register_with_deps(
            leaf_factory,
            "Leaf",
            vec![TypeId::of::<UnregisteredMarker>()],
        );
        let text = container.dependency_graph().to_string();
        assert!(text.contains("unknown"));
    }

    #[test]
    fn test_len_is_empty_has_has_trait_bookkeeping() {
        let mut container = Container::new();
        assert!(container.is_empty());
        assert_eq!(container.len(), 0);
        assert!(!container.has::<Widget>());

        container.register(widget_factory);
        assert!(!container.is_empty());
        assert_eq!(container.len(), 1);
        assert!(container.has::<Widget>());

        assert!(!container.has_trait::<dyn Greeter>());
        container.register_trait(greeter_factory);
        assert!(container.has_trait::<dyn Greeter>());
        assert_eq!(container.len(), 2);
    }

    #[test]
    fn test_concurrent_get_calls_factory_once() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);

        #[derive(Clone)]
        struct Shared;

        fn shared_factory(_c: &Container) -> Arc<Shared> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(5));
            Arc::new(Shared)
        }

        let mut container = Container::new();
        container.register(shared_factory);
        let container = Arc::new(container);

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let container = container.clone();
                std::thread::spawn(move || {
                    let _ = container.get::<Shared>();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}
