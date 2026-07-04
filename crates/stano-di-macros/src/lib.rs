use proc_macro::TokenStream;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::quote;
use syn::{parse_macro_input, Item, Type};

fn stano_di_path() -> TokenStream2 {
    let candidates: &[(&str, &[&str])] = &[
        ("stano-di", &[]),
        ("stano-starter", &[]),
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
            let extra_idents: Vec<_> = extra.iter()
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
        "stano-di-macros: add `stano-di`, `stano-starter`, `stano-starter-service`, \
         or `stano-starter-rest` as a dependency"
    );
}

#[proc_macro_attribute]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    match input {
        Item::Trait(trait_def) => component_trait(trait_def),
        other => syn::Error::new_spanned(
            &other,
            "#[component] can only be used on traits. Structs do not need this annotation — use register() or register_instance() in the container setup.",
        )
        .to_compile_error()
        .into(),
    }
}

fn component_trait(trait_def: syn::ItemTrait) -> TokenStream {
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
        .to_compile_error()
        .into();
    }

    let stano_di = stano_di_path();

    let expanded = quote! {
        #trait_def

        impl #stano_di::DynComponent for dyn #trait_name {}

        impl #stano_di::Injectable for dyn #trait_name {
            fn get_from(container: &#stano_di::Container) -> std::sync::Arc<Self> {
                container.get_trait::<dyn #trait_name>()
            }
        }
    };

    TokenStream::from(expanded)
}

#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    match input {
        Item::Struct(struct_def) => {
            let attr_type = if attr.is_empty() {
                None
            } else {
                match syn::parse::<Type>(attr) {
                    Ok(ty) => Some(ty),
                    Err(e) => {
                        return syn::Error::new_spanned(
                            &struct_def,
                            format!("Failed to parse service trait type: {}", e),
                        )
                        .to_compile_error()
                        .into();
                    }
                }
            };
            service_struct(struct_def, attr_type)
        }
        other => syn::Error::new_spanned(&other, "#[service] can only be used on structs")
            .to_compile_error()
            .into(),
    }
}

fn service_struct(struct_def: syn::ItemStruct, trait_type: Option<Type>) -> TokenStream {
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
                    .to_compile_error()
                    .into();
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
            .to_compile_error()
            .into();
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
                    fn factory(c: &#stano_di::Container) -> std::sync::Arc<Self> {
                        Self::build(c)
                    }
                    container.register_with_deps::<Self>(
                        factory,
                        stringify!(#struct_name),
                        Self::dependency_ids(),
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

    let expanded = quote! {
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
    };

    TokenStream::from(expanded)
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
