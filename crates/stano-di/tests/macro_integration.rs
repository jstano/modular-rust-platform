use stano_di::application_context::ApplicationContext;
use stano_di::container::Container;
use stano_di::environment::Environment;
use stano_di_macros::{component, service};
use std::collections::HashMap;
use std::sync::Arc;

struct TestEnvironment(HashMap<String, String>);

impl Environment for TestEnvironment {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

#[component]
pub trait Greeter: Send + Sync {
    fn greet(&self) -> String;
}

#[service(dyn Greeter)]
pub struct EnglishGreeter;

impl Greeter for EnglishGreeter {
    fn greet(&self) -> String {
        "hello".to_string()
    }
}

#[test]
fn test_component_and_service_macros_register_and_resolve_via_container() {
    let mut container = Container::new();
    container.register_component::<EnglishGreeter>();
    let greeter = container.get_trait::<dyn Greeter>();
    assert_eq!(greeter.greet(), "hello");
}

#[test]
fn test_component_and_service_macros_register_and_resolve_via_application_context() {
    let mut ctx = ApplicationContext::new(Arc::new(TestEnvironment(HashMap::new())));
    ctx.register_component::<EnglishGreeter>();
    let greeter = ctx.get_trait::<dyn Greeter>();
    assert_eq!(greeter.greet(), "hello");
}

// A concrete (non-trait) dependency that a `#[service]` struct can hold via an
// `Arc<T>` field.
#[service]
pub struct Logger;

impl Clone for Logger {
    fn clone(&self) -> Self {
        Logger
    }
}

impl Logger {
    fn log(&self, msg: &str) -> String {
        format!("[log] {msg}")
    }
}

// `#[service(dyn Trait)]` with a real `Arc<T>` dependency field, exercising the
// macro's concrete-field dependency-resolution codegen (not just unit structs).
#[service(dyn Greeter)]
pub struct LoggingGreeter {
    logger: Arc<Logger>,
}

impl Greeter for LoggingGreeter {
    fn greet(&self) -> String {
        self.logger.log("hello")
    }
}

#[test]
fn test_service_with_arc_dependency_field_resolves_via_container() {
    let mut container = Container::new();
    container.register_component::<Logger>();
    container.register_component::<LoggingGreeter>();
    let greeter = container.get_trait::<dyn Greeter>();
    assert_eq!(greeter.greet(), "[log] hello");
}

// Bare `#[service]` (no trait arg) registers as its own concrete type, exercising
// the non-trait registration branch of the macro's codegen.
#[service]
pub struct PlainWidget;

impl Clone for PlainWidget {
    fn clone(&self) -> Self {
        PlainWidget
    }
}

impl PlainWidget {
    fn value(&self) -> i32 {
        42
    }
}

#[test]
fn test_bare_service_without_trait_registers_as_concrete_type() {
    let mut container = Container::new();
    container.register_component::<PlainWidget>();
    let widget = container.get::<PlainWidget>();
    assert_eq!(widget.value(), 42);
}

// Bare `#[service]` with an `Arc<T>` dependency field, combining both branches.
#[service]
pub struct Combo {
    logger: Arc<Logger>,
}

impl Clone for Combo {
    fn clone(&self) -> Self {
        Combo {
            logger: self.logger.clone(),
        }
    }
}

impl Combo {
    fn describe(&self) -> String {
        self.logger.log("combo")
    }
}

#[test]
fn test_bare_service_with_arc_dependency_field() {
    let mut container = Container::new();
    container.register_component::<Logger>();
    container.register_component::<Combo>();
    let combo = container.get::<Combo>();
    assert_eq!(combo.describe(), "[log] combo");
}
