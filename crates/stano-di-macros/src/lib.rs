//! Proc macros for [`stano-di`](https://docs.rs/stano-di): `#[component]` and
//! `#[service]` remove the boilerplate of wiring a trait/struct into a
//! [`stano_di::container::Container`](../stano_di/container/struct.Container.html).
//!
//! ```ignore
//! #[component]
//! pub trait Greeter: Send + Sync {
//!     fn greet(&self) -> String;
//! }
//!
//! #[service(dyn Greeter)]
//! pub struct EnglishGreeter {
//!     // fields must be Arc<T>, and are resolved from the container
//! }
//!
//! impl Greeter for EnglishGreeter {
//!     fn greet(&self) -> String {
//!         "hello".to_string()
//!     }
//! }
//! ```
#![warn(missing_docs)]

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::quote;
use syn::{parse_macro_input, Item, Type};

fn stano_di_path() -> TokenStream2 {
    // A direct dependency on `stano-di` already resolves to the crate itself,
    // so no further path segment should be appended (unlike the `stano-starter`
    // family below, which re-export `stano_di` nested inside themselves via
    // `pub extern crate stano_di`).
    if let Ok(found) = crate_name("stano-di") {
        return match found {
            FoundCrate::Itself => quote!(crate),
            FoundCrate::Name(name) => {
                let ident = Ident::new(&name, Span::call_site());
                quote!(::#ident)
            }
        };
    }

    let candidates: &[(&str, &[&str])] = &[
        ("stano-starter", &[]),
        ("stano-starter-domain", &["stano_starter"]),
        ("stano-starter-rest", &[]),
        ("stano-starter-service", &["stano_starter"]),
    ];

    for (pkg, extra) in candidates {
        if let Ok(found) = crate_name(pkg) {
            let root = match found {
                FoundCrate::Itself => quote!(crate),
                FoundCrate::Name(name) => {
                    let ident = Ident::new(&name, Span::call_site());
                    quote!(::#ident)
                }
            };
            let extra_idents: Vec<_> = extra
                .iter()
                .map(|s| Ident::new(s, Span::call_site()))
                .collect();
            if extra_idents.is_empty() {
                return quote! { #root::stano_di };
            } else {
                return quote! { #root #(::#extra_idents)* ::stano_di };
            }
        }
    }

    panic!(
        "stano-di-macros: add `stano-di`, `stano-starter`, `stano-starter-domain`, \
         `stano-starter-service`, or `stano-starter-rest` as a dependency"
    );
}

/// Marks a trait as an injectable component.
///
/// Requires the trait to have `Send + Sync` supertraits. Generates
/// `DynComponent` and `Injectable` impls for `dyn Trait`, so it can be
/// registered and resolved as a trait object via the container (e.g. by a
/// struct annotated with `#[service(dyn Trait)]`).
///
/// Can only be applied to traits — structs do not need this annotation.
#[proc_macro_attribute]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);
    component_impl(input).into()
}

fn component_impl(input: Item) -> TokenStream2 {
    match input {
        Item::Trait(trait_def) => component_trait(trait_def),
        other => syn::Error::new_spanned(
            &other,
            "#[component] can only be used on traits. Structs do not need this annotation — use register() or register_instance() in the container setup.",
        )
        .to_compile_error(),
    }
}

fn component_trait(trait_def: syn::ItemTrait) -> TokenStream2 {
    let trait_name = &trait_def.ident;

    // Verify trait has Send + Sync bounds
    let has_send = trait_def
        .supertraits
        .iter()
        .any(|bound| matches!(bound, syn::TypeParamBound::Trait(t) if t.path.is_ident("Send")));
    let has_sync = trait_def
        .supertraits
        .iter()
        .any(|bound| matches!(bound, syn::TypeParamBound::Trait(t) if t.path.is_ident("Sync")));

    if !has_send || !has_sync {
        return syn::Error::new_spanned(
            &trait_def.ident,
            "#[component] trait must have Send + Sync supertraits",
        )
        .to_compile_error();
    }

    let stano_di = stano_di_path();

    quote! {
        #trait_def

        impl #stano_di::DynComponent for dyn #trait_name {}

        impl #stano_di::Injectable for dyn #trait_name {
            fn get_from(container: &#stano_di::Container) -> std::sync::Arc<Self> {
                container.get_trait::<dyn #trait_name>()
            }
        }
    }
}

/// Marks a struct as a DI-managed component, auto-registering it with the container.
///
/// All fields must be typed `Arc<T>` — each is resolved from the container
/// when the component is built. Generates a `new()` constructor taking the
/// resolved dependencies as parameters, a `Component` impl, and registers the
/// component at startup via `inventory::submit!`.
///
/// Use `#[service(dyn Trait)]` to register the component as the trait object
/// implementation of a `#[component]`-annotated trait; use bare `#[service]`
/// to register it as its own concrete type.
///
/// Can only be applied to structs (named-field or unit structs).
#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);
    service_impl(TokenStream2::from(attr), input).into()
}

fn service_impl(attr: TokenStream2, input: Item) -> TokenStream2 {
    match input {
        Item::Struct(struct_def) => {
            let attr_type = if attr.is_empty() {
                None
            } else {
                match syn::parse2::<Type>(attr) {
                    Ok(ty) => Some(ty),
                    Err(e) => {
                        return syn::Error::new_spanned(
                            &struct_def,
                            format!("Failed to parse service trait type: {}", e),
                        )
                        .to_compile_error();
                    }
                }
            };
            service_struct(struct_def, attr_type)
        }
        other => syn::Error::new_spanned(&other, "#[service] can only be used on structs")
            .to_compile_error(),
    }
}

fn service_struct(struct_def: syn::ItemStruct, trait_type: Option<Type>) -> TokenStream2 {
    let struct_name = &struct_def.ident;
    let struct_vis = &struct_def.vis;
    let stano_di = stano_di_path();

    // Handle unit structs (no fields) and named field structs
    let (field_names, field_types, dependency_ids, get_calls) = match &struct_def.fields {
        syn::Fields::Named(fields) => {
            let mut field_names = Vec::new();
            let mut field_types = Vec::new();
            let mut dependency_ids = Vec::new();
            let mut get_calls = Vec::new();

            for field in fields.named.iter() {
                let field_name = field.ident.as_ref().unwrap();
                field_names.push(field_name.clone());

                // Check if field is Arc<T>
                let inner_type = extract_arc_inner(&field.ty);
                if inner_type.is_none() {
                    return syn::Error::new_spanned(
                        &field.ty,
                        "Fields in #[service] structs must be typed as Arc<T>",
                    )
                    .to_compile_error();
                }
                let inner_type = inner_type.unwrap();
                field_types.push(field.ty.clone());

                // Determine if it's dyn Trait or concrete, and generate appropriate code
                if is_trait_object(&inner_type) {
                    // dyn Trait
                    let inner_type_clone = inner_type.clone();
                    dependency_ids.push(quote! {
                        #stano_di::TraitObject::<#inner_type_clone>::type_id()
                    });
                    get_calls.push(quote! {
                        container.get_trait::<#inner_type_clone>()
                    });
                } else {
                    // Concrete type
                    let inner_type_clone = inner_type.clone();
                    dependency_ids.push(quote! {
                        std::any::TypeId::of::<#inner_type_clone>()
                    });
                    get_calls.push(quote! {
                        container.get::<#inner_type_clone>()
                    });
                }
            }
            (field_names, field_types, dependency_ids, get_calls)
        }
        syn::Fields::Unit => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
        syn::Fields::Unnamed(_) => {
            return syn::Error::new_spanned(
                &struct_def,
                "#[service] only supports structs with named fields or unit structs",
            )
            .to_compile_error();
        }
    };

    // Generate new() parameter list
    let new_params = field_names
        .iter()
        .zip(field_types.iter())
        .map(|(name, ty)| {
            quote! { #name: #ty }
        });

    // Generate struct field initialization (skip for unit structs)
    let field_inits = if field_names.is_empty() {
        quote! {}
    } else {
        let inits = field_names.iter().map(|name| quote! { #name });
        quote! { #(#inits),* }
    };

    // Generate the constructor body
    let constructor_body = if field_names.is_empty() {
        quote! { Self }
    } else {
        quote! {
            Self {
                #field_inits
            }
        }
    };

    // Generate new() params and get_calls invocation
    let new_call = if field_names.is_empty() {
        quote! { Self::new() }
    } else {
        quote! { Self::new(#(#get_calls),*) }
    };

    // Generate Component impl
    let component_impl = if let Some(trait_ty) = trait_type {
        quote! {
            impl #stano_di::Component for #struct_name {
                fn component_type_name() -> &'static str {
                    stringify!(#struct_name)
                }

                fn dependency_ids() -> Vec<std::any::TypeId> {
                    vec![#(#dependency_ids),*]
                }

                fn build(container: &#stano_di::Container) -> std::sync::Arc<Self> {
                    std::sync::Arc::new(#new_call)
                }

                fn register(container: &mut #stano_di::Container) {
                    fn factory(c: &#stano_di::Container) -> std::sync::Arc<#trait_ty> {
                        #struct_name::build(c) as std::sync::Arc<#trait_ty>
                    }
                    container.register_trait_with_deps::<#trait_ty>(
                        factory,
                        stringify!(#struct_name),
                        #struct_name::dependency_ids(),
                    );
                }
            }
        }
    } else {
        quote! {
            impl #stano_di::Component for #struct_name {
                fn component_type_name() -> &'static str {
                    stringify!(#struct_name)
                }

                fn dependency_ids() -> Vec<std::any::TypeId> {
                    vec![#(#dependency_ids),*]
                }

                fn build(container: &#stano_di::Container) -> std::sync::Arc<Self> {
                    std::sync::Arc::new(#new_call)
                }

                fn register(container: &mut #stano_di::Container) {
                    fn factory(c: &#stano_di::Container) -> std::sync::Arc<#struct_name> {
                        #struct_name::build(c)
                    }
                    container.register_with_deps::<#struct_name>(
                        factory,
                        stringify!(#struct_name),
                        #struct_name::dependency_ids(),
                    );
                }
            }
        }
    };

    let generics = &struct_def.generics;

    // Manually reconstruct fields to avoid including the service attribute
    let reconstructed_fields = match &struct_def.fields {
        syn::Fields::Named(fields) => {
            let named_fields = fields.named.iter();
            quote! {
                {
                    #(#named_fields),*
                }
            }
        }
        syn::Fields::Unit => {
            quote! { ; }
        }
        syn::Fields::Unnamed(fields) => {
            let unnamed_fields = fields.unnamed.iter();
            quote! {
                (#(#unnamed_fields),*);
            }
        }
    };

    quote! {
        #struct_vis struct #struct_name #generics #reconstructed_fields

        impl #struct_name {
            pub fn new(#(#new_params),*) -> Self {
                #constructor_body
            }
        }

        #component_impl

        #stano_di::inventory::submit! {
            #stano_di::ServiceRegistration(|container| {
                <#struct_name as #stano_di::Component>::register(container)
            })
        }
    }
}

fn extract_arc_inner(ty: &Type) -> Option<Type> {
    if let Type::Path(type_path) = ty
        && let Some(last_segment) = type_path.path.segments.last()
        && last_segment.ident == "Arc"
        && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return Some(inner.clone());
    }
    None
}

fn is_trait_object(ty: &Type) -> bool {
    matches!(ty, Type::TraitObject(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn item(src: &str) -> Item {
        syn::parse_str(src).unwrap()
    }

    fn ty(src: &str) -> Type {
        syn::parse_str(src).unwrap()
    }

    // `stano_di_path()` reads the real `CARGO_MANIFEST_DIR`/Cargo.toml unless a test
    // below overrides it, and it's called unconditionally near the top of
    // `service_struct` (before any field validation) and on the success path of
    // `component_trait`. Any test that reaches either of those must hold this lock
    // so it can't run concurrently with a test that's temporarily pointed
    // `CARGO_MANIFEST_DIR` at a fixture manifest.
    static MANIFEST_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_manifest_dir() -> std::sync::MutexGuard<'static, ()> {
        MANIFEST_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn test_extract_arc_inner_returns_inner_type_for_arc() {
        let inner = extract_arc_inner(&ty("Arc<Logger>")).unwrap();
        assert_eq!(quote!(#inner).to_string(), quote!(Logger).to_string());
    }

    #[test]
    fn test_extract_arc_inner_returns_none_for_non_arc() {
        assert!(extract_arc_inner(&ty("Logger")).is_none());
    }

    #[test]
    fn test_is_trait_object_true_for_dyn_trait() {
        assert!(is_trait_object(&ty("dyn Greeter")));
    }

    #[test]
    fn test_is_trait_object_false_for_concrete_type() {
        assert!(!is_trait_object(&ty("Logger")));
    }

    #[test]
    fn test_component_trait_generates_dyn_component_impl_for_valid_trait() {
        let _lock = lock_manifest_dir();
        let Item::Trait(trait_def) =
            item("pub trait Greeter: Send + Sync { fn greet(&self) -> String; }")
        else {
            panic!("expected a trait item");
        };
        let expanded = component_trait(trait_def).to_string();
        assert!(expanded.contains("DynComponent"));
        assert!(expanded.contains("Injectable"));
    }

    #[test]
    fn test_component_trait_rejects_trait_missing_send_and_sync() {
        let Item::Trait(trait_def) = item("pub trait Greeter { fn greet(&self) -> String; }")
        else {
            panic!("expected a trait item");
        };
        let expanded = component_trait(trait_def).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("Send + Sync"));
    }

    #[test]
    fn test_component_trait_rejects_trait_with_only_send() {
        let Item::Trait(trait_def) = item("pub trait Greeter: Send { fn greet(&self) -> String; }")
        else {
            panic!("expected a trait item");
        };
        let expanded = component_trait(trait_def).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("Send + Sync"));
    }

    #[test]
    fn test_component_trait_rejects_trait_with_only_sync() {
        let Item::Trait(trait_def) = item("pub trait Greeter: Sync { fn greet(&self) -> String; }")
        else {
            panic!("expected a trait item");
        };
        let expanded = component_trait(trait_def).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("Send + Sync"));
    }

    #[test]
    fn test_component_rejects_non_trait_item() {
        let expanded = component_impl(item("pub struct NotATrait;")).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("can only be used on traits"));
    }

    #[test]
    fn test_component_impl_dispatches_valid_trait_to_component_trait() {
        let _lock = lock_manifest_dir();
        let expanded =
            component_impl(item("pub trait Greeter: Send + Sync { fn greet(&self) -> String; }"))
                .to_string();
        assert!(expanded.contains("DynComponent"));
        assert!(expanded.contains("Injectable"));
    }

    #[test]
    fn test_service_struct_generates_component_impl_for_unit_struct() {
        let _lock = lock_manifest_dir();
        let Item::Struct(struct_def) = item("pub struct Widget;") else {
            panic!("expected a struct item");
        };
        let expanded = service_struct(struct_def, None).to_string();
        assert!(expanded.contains("Component for Widget"));
        assert!(expanded.contains("fn build"));
    }

    #[test]
    fn test_service_struct_without_trait_type_uses_register_with_deps() {
        let _lock = lock_manifest_dir();
        let Item::Struct(struct_def) = item("pub struct Widget;") else {
            panic!("expected a struct item");
        };
        let expanded = service_struct(struct_def, None).to_string();
        assert!(expanded.contains("register_with_deps"));
        assert!(!expanded.contains("register_trait_with_deps"));
    }

    #[test]
    fn test_service_struct_with_concrete_arc_field_uses_typeid_and_get() {
        let _lock = lock_manifest_dir();
        let Item::Struct(struct_def) = item("pub struct Widget { logger: Arc<Logger> }") else {
            panic!("expected a struct item");
        };
        let expanded = service_struct(struct_def, None).to_string();
        assert!(expanded.contains("TypeId :: of"));
        assert!(expanded.contains("container . get ::"));
        assert!(!expanded.contains("get_trait"));
    }

    #[test]
    fn test_service_struct_rejects_non_arc_field() {
        let _lock = lock_manifest_dir();
        let Item::Struct(struct_def) = item("pub struct Widget { logger: Logger }") else {
            panic!("expected a struct item");
        };
        let expanded = service_struct(struct_def, None).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("must be typed as Arc"));
    }

    #[test]
    fn test_service_struct_with_trait_object_field_uses_get_trait() {
        let _lock = lock_manifest_dir();
        let Item::Struct(struct_def) = item("pub struct Widget { greeter: Arc<dyn Greeter> }")
        else {
            panic!("expected a struct item");
        };
        let expanded = service_struct(struct_def, None).to_string();
        assert!(expanded.contains("get_trait"));
        assert!(expanded.contains("TraitObject"));
    }

    #[test]
    fn test_service_struct_with_trait_type_registers_via_register_trait_with_deps() {
        let _lock = lock_manifest_dir();
        let Item::Struct(struct_def) = item("pub struct Widget;") else {
            panic!("expected a struct item");
        };
        let expanded = service_struct(struct_def, Some(ty("dyn Greeter"))).to_string();
        assert!(expanded.contains("register_trait_with_deps"));
    }

    #[test]
    fn test_service_struct_rejects_tuple_struct() {
        let _lock = lock_manifest_dir();
        let Item::Struct(struct_def) = item("pub struct Widget(u32);") else {
            panic!("expected a struct item");
        };
        let expanded = service_struct(struct_def, None).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("named fields or unit structs"));
    }

    #[test]
    fn test_service_rejects_non_struct_item() {
        let expanded =
            service_impl(TokenStream2::new(), item("pub trait NotAStruct {}")).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("can only be used on structs"));
    }

    #[test]
    fn test_service_rejects_bad_trait_type_token() {
        let attr = TokenStream2::from_str("123").unwrap();
        let expanded = service_impl(attr, item("pub struct Widget;")).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("Failed to parse service trait type"));
    }

    #[test]
    fn test_service_impl_with_empty_attr_dispatches_to_service_struct() {
        let _lock = lock_manifest_dir();
        let expanded = service_impl(TokenStream2::new(), item("pub struct Widget;")).to_string();
        assert!(!expanded.contains("compile_error"));
        assert!(expanded.contains("Component for Widget"));
        assert!(expanded.contains("register_with_deps"));
    }

    #[test]
    fn test_service_impl_with_valid_trait_attr_dispatches_to_service_struct() {
        let _lock = lock_manifest_dir();
        let attr = TokenStream2::from_str("dyn Greeter").unwrap();
        let expanded = service_impl(attr, item("pub struct Widget;")).to_string();
        assert!(!expanded.contains("compile_error"));
        assert!(expanded.contains("register_trait_with_deps"));
    }

    #[test]
    fn test_stano_di_path_resolves_to_dev_dependency() {
        let _lock = lock_manifest_dir();
        let path = stano_di_path().to_string();
        assert!(path.contains("stano_di"));
    }

    struct ManifestDirGuard {
        original: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ManifestDirGuard {
        fn set(path: &str) -> Self {
            let lock = lock_manifest_dir();
            let original = std::env::var_os("CARGO_MANIFEST_DIR");
            unsafe {
                std::env::set_var("CARGO_MANIFEST_DIR", path);
            }
            ManifestDirGuard {
                original,
                _lock: lock,
            }
        }
    }

    impl Drop for ManifestDirGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.original {
                    Some(v) => std::env::set_var("CARGO_MANIFEST_DIR", v),
                    None => std::env::remove_var("CARGO_MANIFEST_DIR"),
                }
            }
        }
    }

    #[test]
    fn test_stano_di_path_falls_back_to_stano_starter_family() {
        let manifest_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stano_starter_consumer");
        let _guard = ManifestDirGuard::set(manifest_dir);
        let path = stano_di_path().to_string();
        assert!(path.contains("stano_di"));
    }

    #[test]
    fn test_stano_di_path_panics_when_no_known_dependency_present() {
        let manifest_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/no_di_consumer");
        let _guard = ManifestDirGuard::set(manifest_dir);
        let result = std::panic::catch_unwind(stano_di_path);
        assert!(result.is_err());
    }
}
