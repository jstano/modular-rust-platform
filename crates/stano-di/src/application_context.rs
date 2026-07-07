use crate::container::{Component, Container, ContainerError, DynComponent};
use crate::environment::Environment;
use std::sync::Arc;

/// App-level wiring façade combining a [`Container`] with an [`Environment`].
///
/// Applications typically build one `ApplicationContext` at startup, register
/// components and/or call [`ApplicationContext::register_all`] to pick up
/// `#[service]`-registered components, then call [`ApplicationContext::validate`]
/// before serving traffic.
pub struct ApplicationContext {
    environment: Arc<dyn Environment>,
    container: Container,
}

impl ApplicationContext {
    /// Create a new context wrapping the given environment, with an empty container.
    pub fn new(environment: Arc<dyn Environment>) -> Self {
        Self {
            environment,
            container: Container::new(),
        }
    }

    /// Register a concrete component type. See [`Container::register`].
    pub fn register<T: Send + Sync + 'static>(&mut self, factory: fn(&Container) -> Arc<T>) {
        self.container.register(factory);
    }

    /// Register a trait object component. See [`Container::register_trait`].
    pub fn register_trait<T: DynComponent + ?Sized>(&mut self, factory: fn(&Container) -> Arc<T>)
    where
        Arc<T>: Send + Sync,
    {
        self.container.register_trait(factory);
    }

    /// Register a pre-built instance. See [`Container::register_instance`].
    pub fn register_instance<T: Send + Sync + 'static>(&mut self, instance: Arc<T>) {
        self.container.register_instance(instance);
    }

    /// Register a component using its generated impl. See [`Container::register_component`].
    pub fn register_component<T: Component>(&mut self) {
        self.container.register_component::<T>();
    }

    /// Get a concrete component type. Panics if not registered. See [`Container::get`].
    pub fn get<T: Send + Sync + Clone + 'static>(&self) -> Arc<T> {
        self.container.get()
    }

    /// Get a trait object component. Panics if not registered. See [`Container::get_trait`].
    pub fn get_trait<T: DynComponent + ?Sized>(&self) -> Arc<T> {
        self.container.get_trait()
    }

    /// Try to get a concrete component type. See [`Container::try_get`].
    pub fn try_get<T: Send + Sync + Clone + 'static>(&self) -> Result<Arc<T>, ContainerError> {
        self.container.try_get()
    }

    /// Try to get a trait object component. See [`Container::try_get_trait`].
    pub fn try_get_trait<T: DynComponent + ?Sized>(&self) -> Result<Arc<T>, ContainerError>
    where
        Arc<T>: Clone,
    {
        self.container.try_get_trait()
    }

    /// The environment this context was constructed with.
    pub fn environment(&self) -> &Arc<dyn Environment> {
        &self.environment
    }

    /// Eagerly resolve every registered singleton. See [`Container::validate`].
    pub fn validate(&self) -> Result<(), Vec<ContainerError>> {
        self.container.validate()
    }

    /// Register every component collected via `#[service]`'s `inventory::submit!`.
    pub fn register_all(&mut self) {
        crate::register_all(&mut self.container);
    }

    /// Escape hatch for direct mutable access to the underlying [`Container`].
    pub fn container_mut(&mut self) -> &mut Container {
        &mut self.container
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::MockEnvironment;

    #[derive(Clone)]
    struct Widget(i32);

    fn widget_factory(_c: &Container) -> Arc<Widget> {
        Arc::new(Widget(7))
    }

    #[derive(Clone)]
    struct Gadget;

    impl Component for Gadget {
        fn component_type_name() -> &'static str {
            "Gadget"
        }

        fn dependency_ids() -> Vec<std::any::TypeId> {
            vec![]
        }

        fn build(_container: &Container) -> Arc<Self> {
            Arc::new(Gadget)
        }

        fn register(container: &mut Container) {
            container.register_with_deps(
                Self::build,
                Self::component_type_name(),
                Self::dependency_ids(),
            );
        }
    }

    trait Greeter: DynComponent {
        fn greet(&self) -> String;
    }

    struct EnglishGreeter;

    impl DynComponent for EnglishGreeter {}

    impl Greeter for EnglishGreeter {
        fn greet(&self) -> String {
            "hi".to_string()
        }
    }

    fn greeter_factory(_c: &Container) -> Arc<dyn Greeter> {
        Arc::new(EnglishGreeter)
    }

    fn context() -> ApplicationContext {
        ApplicationContext::new(Arc::new(MockEnvironment::new().with_var("KEY", "value")))
    }

    #[test]
    fn test_register_and_get_delegate_to_container() {
        let mut ctx = context();
        ctx.register(widget_factory);
        let widget = ctx.get::<Widget>();
        assert_eq!(widget.0, 7);
    }

    #[test]
    fn test_register_trait_delegates() {
        let mut ctx = context();
        ctx.register_trait(greeter_factory);
        let greeter = ctx.get_trait::<dyn Greeter>();
        assert_eq!(greeter.greet(), "hi");
    }

    #[test]
    fn test_register_instance_delegates() {
        let mut ctx = context();
        let instance = Arc::new(Widget(9));
        ctx.register_instance(instance.clone());
        let resolved = ctx.get::<Widget>();
        assert!(Arc::ptr_eq(&instance, &resolved));
    }

    #[test]
    fn test_register_component_delegates() {
        let mut ctx = context();
        ctx.register_component::<Gadget>();
        let _gadget = ctx.get::<Gadget>();
    }

    #[test]
    fn test_try_get_try_get_trait_delegate() {
        let ctx = context();
        assert!(matches!(
            ctx.try_get::<Widget>(),
            Err(ContainerError::NotRegistered(_))
        ));
        assert!(matches!(
            ctx.try_get_trait::<dyn Greeter>(),
            Err(ContainerError::NotRegistered(_))
        ));
    }

    #[test]
    fn test_environment_returns_same_arc() {
        let env: Arc<dyn Environment> = Arc::new(MockEnvironment::new().with_var("K", "V"));
        let ctx = ApplicationContext::new(env.clone());
        assert!(Arc::ptr_eq(&env, ctx.environment()));
    }

    #[test]
    fn test_validate_delegates() {
        let mut ctx = context();
        ctx.register(widget_factory);
        assert!(ctx.validate().is_ok());
    }

    fn app_context_test_registration(container: &mut Container) {
        container.register(widget_factory);
    }

    crate::inventory::submit! {
        crate::ServiceRegistration(app_context_test_registration)
    }

    #[test]
    fn test_register_all_picks_up_inventory_submitted_registration() {
        let mut ctx = context();
        ctx.register_all();
        assert!(ctx.try_get::<Widget>().is_ok());
    }

    #[test]
    fn test_container_mut_allows_post_construction_mutation() {
        let mut ctx = context();
        ctx.container_mut().register(widget_factory);
        let widget = ctx.get::<Widget>();
        assert_eq!(widget.0, 7);
    }
}
