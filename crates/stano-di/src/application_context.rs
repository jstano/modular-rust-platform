use crate::container::{Component, Container, ContainerError, DynComponent};
use crate::environment::Environment;
use std::sync::Arc;

pub struct ApplicationContext {
    environment: Arc<dyn Environment>,
    container: Container,
}

impl ApplicationContext {
    pub fn new(environment: Arc<dyn Environment>) -> Self {
        Self {
            environment,
            container: Container::new(),
        }
    }

    pub fn register<T: Send + Sync + 'static>(&mut self, factory: fn(&Container) -> Arc<T>) {
        self.container.register(factory);
    }

    pub fn register_trait<T: DynComponent + ?Sized>(&mut self, factory: fn(&Container) -> Arc<T>)
    where
        Arc<T>: Send + Sync,
    {
        self.container.register_trait(factory);
    }

    pub fn register_instance<T: Send + Sync + 'static>(&mut self, instance: Arc<T>) {
        self.container.register_instance(instance);
    }

    pub fn register_component<T: Component>(&mut self) {
        self.container.register_component::<T>();
    }

    pub fn get<T: Send + Sync + Clone + 'static>(&self) -> Arc<T> {
        self.container.get()
    }

    pub fn get_trait<T: DynComponent + ?Sized>(&self) -> Arc<T> {
        self.container.get_trait()
    }

    pub fn try_get<T: Send + Sync + Clone + 'static>(&self) -> Result<Arc<T>, ContainerError> {
        self.container.try_get()
    }

    pub fn try_get_trait<T: DynComponent + ?Sized>(&self) -> Result<Arc<T>, ContainerError>
    where
        Arc<T>: Clone,
    {
        self.container.try_get_trait()
    }

    pub fn environment(&self) -> &Arc<dyn Environment> {
        &self.environment
    }

    pub fn validate(&self) -> Result<(), Vec<ContainerError>> {
        self.container.validate()
    }

    pub fn register_all(&mut self) {
        crate::register_all(&mut self.container);
    }

    pub fn container_mut(&mut self) -> &mut Container {
        &mut self.container
    }
}
