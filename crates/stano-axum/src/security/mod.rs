//! Spring-Security-style declarative request authorization, built via
//! [`AuthorizationBuilder`] and applied via [`AuthorizationLayer`].
mod authorization;
mod layer;
mod pattern;

pub use authorization::{AuthorizationBuildError, AuthorizationBuilder, RequestMatcherBuilder};
pub use layer::AuthorizationLayer;
