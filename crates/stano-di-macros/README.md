# stano-di-macros

Procedural macros for automatic DI boilerplate generation: `#[component]` for trait definitions and `#[service]` for struct implementations. Generates `Component`, `Injectable`, and `DynComponent` trait impls and auto-registers via `inventory::submit!`.

## Install

```toml
[dependencies]
stano-di-macros = { path = "../stano-di-macros" }
```

## Macros

### `#[component]`

Marks a **trait** as an injectable component. Generates `DynComponent` and `Injectable` impls for the trait object.

```rust
use stano_di_macros::component;

#[component]
pub trait MyService: Send + Sync {
    fn do_something(&self) -> String;
}
```

**Requirements:**
- Must be applied to a **trait**.
- Trait **must explicitly declare `Send + Sync`** supertraits (error otherwise).
- Generates `impl DynComponent for dyn MyService {}` and `impl Injectable for dyn MyService { ... }`.

### `#[service]` and `#[service(dyn Trait)]`

Marks a **struct** as a service/component. Generates a `Component` impl and auto-registers via `inventory::submit!`.

**Without a trait argument** — registers as a concrete type:
```rust
use stano_di_macros::service;

#[service]
pub struct MyServiceImpl {
    dependency: Arc<dyn SomeTrait>,
}
```

**With a trait argument** — registers under a trait object:
```rust
#[service(dyn MyService)]
pub struct MyServiceImpl {
    dependency: Arc<dyn SomeTrait>,
}

impl MyService for MyServiceImpl { ... }
```

**Requirements:**
- Must be applied to a **struct**.
- Only **named-field or unit structs** supported (no tuple structs; error otherwise).
- Every field **must be typed `Arc<T>`** (unwrapped dependencies; error if field is `T` or `Box<T>`).
- Generated `impl Component` provides:
  - `fn build(container: &Container) -> Arc<Self>` — auto-fetches dependencies and constructs via `MyServiceImpl::new(dep1, dep2, ...)`.
  - `fn dependency_ids() -> Vec<TypeId>` — auto-detects field types.
  - `fn register(container: &mut Container)` — registers the factory (as trait object if trait argument given).
- Auto-registration via `inventory::submit! { ServiceRegistration(|c| MyServiceImpl::register(c)) }` — use `container.register_all()` to activate.

## Usage Example

```rust
use stano_di_macros::{component, service};
use stano_di::Container;
use std::sync::Arc;

// Trait marked as component.
#[component]
#[component]
pub trait Logger: Send + Sync {
    fn log(&self, msg: &str);
}

// Concrete implementation.
#[service(dyn Logger)]
pub struct ConsoleLogger;

impl Logger for ConsoleLogger {
    fn log(&self, msg: &str) {
        println!("{}", msg);
    }
}

// Service that depends on Logger.
#[component]
pub trait UserService: Send + Sync {
    fn create_user(&self, name: &str);
}

#[service(dyn UserService)]
pub struct UserServiceImpl {
    logger: Arc<dyn Logger>,
}

impl UserService for UserServiceImpl {
    fn create_user(&self, name: &str) {
        self.logger.log(&format!("Creating user: {}", name));
    }
}

// Register and use (in a consuming app).
fn main() {
    let mut container = Container::new();
    // Manually register for this example; in practice, use register_all() if the app depends
    // on one of the recognized starter crates (stano-starter, stano-starter-domain, etc.)
    container.register_trait::<dyn Logger>(|_| Arc::new(ConsoleLogger));
    // container.register_all() would auto-register UserServiceImpl here
    
    let service: Arc<dyn UserService> = container.get_trait();
    service.create_user("Alice");
}
```

## Notes

- **Macro path resolution** — `#[component]` and `#[service]` macros work only in crates that depend on one of these five hardcoded crate names: `stano-di`, `stano-starter`, `stano-starter-domain`, `stano-starter-rest`, or `stano-starter-service`. If your app uses a different name, the macros will panic with instructions to add one of these as a dependency.
- **Field requirement** — all fields in a `#[service]` struct must be `Arc<T>` to enable safe concurrent access and lazy dependency resolution.
- **Trait object registration** — when using `#[service(dyn Trait)]`, ensure the struct implements that trait; the macro only generates the registration code, not the trait impl.
- **No feature flags** — all macros available.

See also: [`stano-di`](../stano-di) for the `Container` and `ApplicationContext` types that use these generated impls.
