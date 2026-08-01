//! Proc macros for [`stano-launcher`](https://docs.rs/stano-launcher): `#[get]`/`#[post]`/
//! `#[put]`/`#[delete]`/`#[patch]` remove the boilerplate of writing `#[utoipa::path(...)]`
//! by inferring what's already visible in the handler's signature:
//!
//! - the HTTP method, from which macro is used
//! - `operation_id`, from the function name
//! - `request_body`, from a single `AppJson<T>` parameter
//! - the `200` entry of `responses(...)`, from an `AppJson<T>` or `Result<AppJson<T>, E>`
//!   return type
//! - `params(...)`, from `AppPath<T>`/`AppQuery<T>` parameters
//!
//! Everything else (`path`, `tag`/`tags`, `security`, extra `responses(...)` entries for
//! error cases) is still written explicitly and forwarded verbatim into the generated
//! `#[utoipa::path(...)]` attribute.
//!
//! ```ignore
//! #[get(path = "/health", responses((status = 200, body = String)))]
//! async fn health_handler() -> &'static str {
//!     "ok"
//! }
//!
//! #[post(path = "/widgets", tag = "widgets")]
//! async fn create_widget_handler(
//!     AppJson(req): AppJson<CreateWidgetRequest>,
//! ) -> AppJson<WidgetResponse> {
//!     // request_body and the 200 response are both inferred
//! }
//! ```
#![warn(missing_docs)]

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Delimiter, Ident, Literal, Span, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{FnArg, Item, ItemFn, PatType, ReturnType, Type, parse_macro_input};

fn found_crate_path(found: FoundCrate) -> TokenStream2 {
    match found {
        FoundCrate::Itself => quote!(crate),
        FoundCrate::Name(name) => {
            let ident = Ident::new(&name, Span::call_site());
            quote!(::#ident)
        }
    }
}

fn stano_routes_path() -> TokenStream2 {
    // `RouteRegistration`/`inventory`/`collect_routes` live in `stano-axum`, so any crate
    // that only writes route handlers (no server bootstrap) never needs to depend on
    // `stano-launcher`. A direct dependency on `stano-axum` resolves to the crate itself.
    // Crates that instead depend on `stano-launcher` or the `stano-starter-rest` facade get
    // `stano-axum` re-exported nested inside, via `pub extern crate stano_axum`.
    if let Ok(found) = crate_name("stano-axum") {
        return found_crate_path(found);
    }

    if let Ok(found) = crate_name("stano-launcher") {
        let root = found_crate_path(found);
        return quote! { #root::stano_axum };
    }

    if let Ok(found) = crate_name("stano-starter-rest") {
        let root = found_crate_path(found);
        return quote! { #root::stano_axum };
    }

    panic!(
        "stano-route-macros: add `stano-axum`, `stano-launcher`, or `stano-starter-rest` as a dependency"
    );
}

macro_rules! method_macro {
    ($name:ident, $method:literal) => {
        #[doc = concat!(
            "Auto-registers a `",
            $method,
            "` handler into the platform's global route collection, inferring ",
            "`operation_id`, `request_body`, the `200` response, and `params(...)` from the ",
            "handler's signature. See the [crate-level docs](crate) for the full inference ",
            "rules."
        )]
        #[proc_macro_attribute]
        pub fn $name(attr: TokenStream, item: TokenStream) -> TokenStream {
            let input = parse_macro_input!(item as Item);
            route_method_impl($method, TokenStream2::from(attr), input).into()
        }
    };
}

method_macro!(get, "get");
method_macro!(post, "post");
method_macro!(put, "put");
method_macro!(delete, "delete");
method_macro!(patch, "patch");

fn route_method_impl(method: &str, attr: TokenStream2, input: Item) -> TokenStream2 {
    match input {
        Item::Fn(func) => route_method_fn(method, attr, func),
        other => {
            syn::Error::new_spanned(&other, format!("#[{method}] can only be used on functions"))
                .to_compile_error()
        }
    }
}

// ---------------------------------------------------------------------------------------
// Shallow top-level attribute-key splitting
// ---------------------------------------------------------------------------------------

/// One `key = value` or `key(...)` chunk of a `#[utoipa::path(...)]`-style attribute list,
/// split on top-level commas only. Nested commas inside a `(...)`/`[...]`/`{...}` group are
/// untouched (proc_macro2 already represents groups as atomic `TokenTree::Group` nodes); the
/// one extra case handled here is a bare, ungrouped generic like `Vec<Foo>` used directly as
/// a `request_body = ...` value, tracked via `<`/`>` depth so its (rare) internal commas
/// don't get mistaken for a key separator.
struct AttrKey {
    name: String,
    tokens: TokenStream2,
}

fn split_top_level_keys(attr: TokenStream2) -> Vec<AttrKey> {
    let mut keys = Vec::new();
    let mut current: Vec<TokenTree> = Vec::new();
    let mut angle_depth = 0i32;

    for tt in attr {
        if let TokenTree::Punct(p) = &tt {
            match p.as_char() {
                '<' => angle_depth += 1,
                '>' if angle_depth > 0 => angle_depth -= 1,
                ',' if angle_depth == 0 => {
                    if !current.is_empty() {
                        keys.push(build_attr_key(std::mem::take(&mut current)));
                    }
                    continue;
                }
                _ => {}
            }
        }
        current.push(tt);
    }
    if !current.is_empty() {
        keys.push(build_attr_key(current));
    }
    keys
}

fn build_attr_key(tokens: Vec<TokenTree>) -> AttrKey {
    let name = match tokens.first() {
        Some(TokenTree::Ident(ident)) => ident.to_string(),
        _ => String::new(),
    };
    AttrKey {
        name,
        tokens: tokens.into_iter().collect(),
    }
}

/// Extracts the single `(...)`/`[...]` group's inner tokens from a `key(...)` chunk, e.g.
/// turns `responses(a, b)`'s tokens into `a, b`. Returns `None` if `tokens` isn't a bare
/// `key` followed by exactly one group (i.e. it's a `key = value` chunk instead).
fn group_inner_tokens(tokens: &TokenStream2) -> Option<TokenStream2> {
    let mut iter = tokens.clone().into_iter();
    let _key = iter.next()?;
    match iter.next()? {
        TokenTree::Group(group) if iter.next().is_none() => Some(group.stream()),
        _ => None,
    }
}

fn entry_has_status_200(entry: &TokenTree) -> bool {
    let TokenTree::Group(group) = entry else {
        return false;
    };
    if group.delimiter() != Delimiter::Parenthesis {
        return false;
    }
    for key in split_top_level_keys(group.stream()) {
        if key.name != "status" {
            continue;
        }
        for tt in key.tokens {
            if let TokenTree::Literal(lit) = tt
                && let Some(rest) = lit.to_string().strip_prefix("200")
                && (rest.is_empty() || rest.chars().all(|c| c.is_ascii_alphabetic()))
            {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------------------
// Signature inspection
// ---------------------------------------------------------------------------------------

/// If `ty` is `wrapper<T>` (matched on the *last* path segment ident only, since call sites
/// may import `AppJson`/`AppPath`/`AppQuery` under any path/alias — this is a name-based
/// heuristic, not real type resolution, and will also match an unrelated local type that
/// happens to share the name), returns `T`.
fn extract_type_arg(ty: &Type, wrapper: &str) -> Option<Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != wrapper {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    match args.args.first()? {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    }
}

/// If `ty` is `Result<A, B>`, returns `A`.
fn extract_result_ok_arg(ty: &Type) -> Option<Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    }
}

const PRIMITIVE_PATH_PARAM_TYPES: &[&str] = &[
    "String", "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32",
    "u64", "u128", "usize", "f32", "f64",
];

/// Whether `ty` looks like a primitive or a typed-ID (`stano_common::id_type!`-generated,
/// e.g. `WidgetId`) — the shapes eligible for the single-placeholder `("name" = T, Path)`
/// convenience form. A single-segment, non-generic type path ending in `Id` is treated as a
/// typed ID by naming convention only (a proc macro can't resolve whether it's actually one);
/// this is a documented heuristic, not a hard guarantee.
fn is_primitive_path_param_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    if type_path.path.segments.len() != 1 {
        return false;
    }
    let segment = &type_path.path.segments[0];
    if !segment.arguments.is_empty() {
        return false;
    }
    let name = segment.ident.to_string();
    PRIMITIVE_PATH_PARAM_TYPES.contains(&name.as_str()) || name.ends_with("Id")
}

/// Extracts the `{name}`/`{*name}` placeholders from a `path = "..."` string literal, in
/// order. Purely textual — no matchit-level validation.
fn path_placeholders(path_lit: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = path_lit.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{'
            && let Some(len) = path_lit[i + 1..].find('}')
        {
            let name = &path_lit[i + 1..i + 1 + len];
            names.push(name.trim_start_matches('*').to_string());
            i += len + 2;
            continue;
        }
        i += 1;
    }
    names
}

fn find_path_literal(keys: &[AttrKey]) -> Option<String> {
    let path_key = keys.iter().find(|k| k.name == "path")?;
    for tt in path_key.tokens.clone() {
        if let TokenTree::Literal(lit) = tt {
            let lit_str: syn::LitStr = syn::parse_str(&lit.to_string()).ok()?;
            return Some(lit_str.value());
        }
    }
    None
}

/// Parameter types matched by `extract_type_arg`, gathered once per handler.
struct SignatureInference {
    app_json_types: Vec<Type>,
    app_path_type: Option<Type>,
    app_path_count: usize,
    app_query_type: Option<Type>,
    app_query_count: usize,
}

fn inspect_signature(func: &ItemFn) -> SignatureInference {
    let mut app_json_types = Vec::new();
    let mut app_path_type = None;
    let mut app_path_count = 0;
    let mut app_query_type = None;
    let mut app_query_count = 0;

    for input in &func.sig.inputs {
        let FnArg::Typed(PatType { ty, .. }) = input else {
            continue;
        };
        if let Some(inner) = extract_type_arg(ty, "AppJson") {
            app_json_types.push(inner);
        }
        if let Some(inner) = extract_type_arg(ty, "AppPath") {
            app_path_count += 1;
            app_path_type.get_or_insert(inner);
        }
        if let Some(inner) = extract_type_arg(ty, "AppQuery") {
            app_query_count += 1;
            app_query_type.get_or_insert(inner);
        }
    }

    SignatureInference {
        app_json_types,
        app_path_type,
        app_path_count,
        app_query_type,
        app_query_count,
    }
}

/// Returns the `AppJson<T>` inner `T` for a return type of `AppJson<T>` or
/// `Result<AppJson<T>, E>`.
fn inferred_response_body(ret: &ReturnType) -> Option<Type> {
    let ReturnType::Type(_, ty) = ret else {
        return None;
    };
    if let Some(inner) = extract_type_arg(ty, "AppJson") {
        return Some(inner);
    }
    let ok_ty = extract_result_ok_arg(ty)?;
    extract_type_arg(&ok_ty, "AppJson")
}

// ---------------------------------------------------------------------------------------
// Core expansion
// ---------------------------------------------------------------------------------------

fn route_method_fn(method: &str, attr: TokenStream2, func: ItemFn) -> TokenStream2 {
    let fn_name = &func.sig.ident;
    let fn_name_str = fn_name.to_string();
    let method_ident = Ident::new(method, Span::call_site());

    let keys = split_top_level_keys(attr);
    let sig = inspect_signature(&func);

    if sig.app_json_types.len() > 1 {
        return syn::Error::new_spanned(
            &func.sig,
            format!(
                "#[{method}]: found more than one AppJson<_> parameter; a handler can only \
                 take one request body"
            ),
        )
        .to_compile_error();
    }
    if sig.app_path_count > 1 {
        return syn::Error::new_spanned(
            &func.sig,
            format!(
                "#[{method}]: found more than one AppPath<_> parameter; combine multiple path \
                 segments into one #[derive(utoipa::IntoParams)] struct"
            ),
        )
        .to_compile_error();
    }
    if sig.app_query_count > 1 {
        return syn::Error::new_spanned(
            &func.sig,
            format!(
                "#[{method}]: found more than one AppQuery<_> parameter; combine multiple \
                 query fields into one #[derive(utoipa::IntoParams)] struct"
            ),
        )
        .to_compile_error();
    }

    // --- request_body -------------------------------------------------------------------
    let explicit_request_body = keys.iter().find(|k| k.name == "request_body");
    let inferred_request_body = sig.app_json_types.first();

    let request_body_tokens: Option<TokenStream2> =
        match (explicit_request_body, inferred_request_body) {
            (Some(explicit), Some(inferred)) => {
                let explicit_value: TokenStream2 =
                    explicit.tokens.clone().into_iter().skip(2).collect(); // skip `request_body` `=`
                let explicit_norm = normalize(&explicit_value);
                let inferred_norm = normalize(&quote!(#inferred));
                if explicit_norm != inferred_norm {
                    return syn::Error::new_spanned(
                        &func.sig,
                        format!(
                            "#[{method}]: request_body = {explicit_norm} conflicts with inferred \
                         AppJson<{inferred_norm}> parameter type; remove one"
                        ),
                    )
                    .to_compile_error();
                }
                Some(quote! { request_body = #inferred })
            }
            (Some(explicit), None) => Some(explicit.tokens.clone()),
            (None, Some(inferred)) => Some(quote! { request_body = #inferred }),
            (None, None) => None,
        };

    // --- responses -------------------------------------------------------------------
    let explicit_responses_key = keys.iter().find(|k| k.name == "responses");
    let explicit_responses_inner =
        explicit_responses_key.and_then(|k| group_inner_tokens(&k.tokens));
    let inferred_body = inferred_response_body(&func.sig.output);

    if inferred_body.is_some()
        && let Some(inner) = &explicit_responses_inner
        && split_top_level_keys(inner.clone())
            .iter()
            .flat_map(|k| k.tokens.clone())
            .any(|tt| entry_has_status_200(&tt))
    {
        return syn::Error::new_spanned(
            &func.sig,
            format!(
                "#[{method}] fn {fn_name_str}: inferred 200 response from the return type \
                 conflicts with an explicit `(status = 200, ...)` entry in `responses(...)`; \
                 the 200 case is always inferred from the return type — remove it from \
                 `responses(...)` and keep only error-case entries there."
            ),
        )
        .to_compile_error();
    }

    if inferred_body.is_none() && explicit_responses_inner.is_none() {
        return syn::Error::new_spanned(
            &func.sig,
            format!(
                "#[{method}] fn {fn_name_str}: cannot infer a 200 response (return type is not \
                 AppJson<_> or Result<AppJson<_>, _>) and no explicit responses(...) was \
                 provided; add responses((status = 200, body = ...)) or change the return type \
                 to AppJson<_>."
            ),
        )
        .to_compile_error();
    }

    let responses_tokens: Option<TokenStream2> = match (inferred_body, explicit_responses_inner) {
        (Some(body), Some(rest)) => Some(quote! { responses((status = 200, body = #body), #rest) }),
        (Some(body), None) => Some(quote! { responses((status = 200, body = #body)) }),
        (None, Some(rest)) => Some(quote! { responses(#rest) }),
        (None, None) => unreachable!("handled by the compile_error above"),
    };

    // --- params -------------------------------------------------------------------
    let explicit_params_key = keys.iter().find(|k| k.name == "params");
    let explicit_params_inner = explicit_params_key.and_then(|k| group_inner_tokens(&k.tokens));

    let path_lit = find_path_literal(&keys);
    let mut inferred_params: Vec<TokenStream2> = Vec::new();

    if let Some(path_ty) = &sig.app_path_type {
        let placeholders = path_lit
            .as_deref()
            .map(path_placeholders)
            .unwrap_or_default();
        if placeholders.len() == 1 && is_primitive_path_param_type(path_ty) {
            let name = Literal::string(&placeholders[0]);
            inferred_params.push(quote! { (#name = #path_ty, Path) });
        } else {
            inferred_params.push(quote! { #path_ty });
        }
    }
    if let Some(query_ty) = &sig.app_query_type {
        inferred_params.push(quote! { #query_ty });
    }

    let params_tokens: Option<TokenStream2> =
        match (inferred_params.is_empty(), explicit_params_inner) {
            (true, None) => None,
            (true, Some(rest)) => Some(quote! { params(#rest) }),
            (false, None) => Some(quote! { params(#(#inferred_params),*) }),
            (false, Some(rest)) => Some(quote! { params(#(#inferred_params),* , #rest) }),
        };

    // --- pass-through keys -------------------------------------------------------------------
    let pass_through: Vec<TokenStream2> = keys
        .iter()
        .filter(|k| !matches!(k.name.as_str(), "request_body" | "responses" | "params"))
        .map(|k| k.tokens.clone())
        .collect();

    // Resolved last: only the success path below needs the caller's `stano-axum` (or
    // `stano-launcher`/`stano-starter-rest`) dependency, so validation errors above never
    // trigger the "add a dependency" panic.
    let stano_routes = stano_routes_path();

    let operation_id = Literal::string(&fn_name_str);

    let mut path_args: Vec<TokenStream2> = vec![
        quote! { #method_ident },
        quote! { operation_id = #operation_id },
    ];
    path_args.extend(pass_through);
    path_args.extend(request_body_tokens);
    path_args.extend(responses_tokens);
    path_args.extend(params_tokens);

    quote! {
        #[utoipa::path(#(#path_args),*)]
        #func

        #stano_routes::routes::inventory::submit! {
            #stano_routes::routes::RouteRegistration(|| {
                #stano_routes::routes::utoipa_axum::routes!(#fn_name)
            })
        }
    }
}

/// Whitespace-insensitive comparison of two token streams, used to detect whether an
/// explicit `request_body = T` agrees with an inferred `AppJson<T>` parameter type.
fn normalize(tokens: &TokenStream2) -> String {
    tokens.to_string().replace(' ', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(src: &str) -> Item {
        syn::parse_str(src).unwrap()
    }

    // `stano_routes_path()` reads `CARGO_MANIFEST_DIR`/Cargo.toml and is called
    // unconditionally on the success path of `route_method_fn`, so every test that reaches it
    // points `CARGO_MANIFEST_DIR` at a fixture manifest via `ManifestDirGuard`. This lock keeps
    // those overrides from racing each other across threads.
    static MANIFEST_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_manifest_dir() -> std::sync::MutexGuard<'static, ()> {
        MANIFEST_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn expand(method: &str, attr: &str, src: &str) -> String {
        let manifest_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/direct_axum_consumer"
        );
        let _guard = ManifestDirGuard::set(manifest_dir);
        let attr = syn::parse_str::<TokenStream2>(attr).unwrap();
        let Item::Fn(func) = item(src) else {
            panic!("expected a fn item");
        };
        route_method_fn(method, attr, func).to_string()
    }

    fn norm(s: &str) -> String {
        s.replace(' ', "")
    }

    #[test]
    fn test_route_method_impl_rejects_non_fn_item() {
        let expanded =
            route_method_impl("get", TokenStream2::new(), item("struct NotAFn;")).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("#[get] can only be used on functions"));
    }

    #[test]
    fn test_get_forwards_pass_through_keys_verbatim() {
        let expanded = expand(
            "get",
            "path = \"/health\", tag = \"t\", security((\"bearerAuth\" = [])), responses((status = 200, body = String))",
            "async fn health() -> &'static str { \"ok\" }",
        );
        assert!(!expanded.contains("compile_error"));
        assert!(expanded.contains("path = \"/health\""));
        assert!(expanded.contains("tag = \"t\""));
        assert!(expanded.contains("security"));
    }

    #[test]
    fn test_operation_id_is_injected_from_fn_name() {
        let expanded = expand(
            "get",
            "path = \"/health\", responses((status = 200, body = String))",
            "async fn health_check() -> &'static str { \"ok\" }",
        );
        assert!(norm(&expanded).contains("operation_id=\"health_check\""));
    }

    #[test]
    fn test_request_body_inferred_from_single_app_json_param() {
        let expanded = expand(
            "post",
            "path = \"/widgets\"",
            "async fn create(AppJson(req): AppJson<Foo>) -> AppJson<Bar> { todo!() }",
        );
        assert!(norm(&expanded).contains("request_body=Foo"));
    }

    #[test]
    fn test_request_body_not_inferred_when_no_app_json_param() {
        let expanded = expand(
            "get",
            "path = \"/x\", responses((status = 200, body = String))",
            "async fn handler() -> &'static str { \"ok\" }",
        );
        assert!(!expanded.contains("request_body"));
    }

    #[test]
    fn test_request_body_multiple_app_json_params_is_compile_error() {
        let expanded = expand(
            "post",
            "path = \"/x\"",
            "async fn handler(a: AppJson<Foo>, b: AppJson<Bar>) -> AppJson<Baz> { todo!() }",
        );
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("more than one AppJson"));
    }

    #[test]
    fn test_request_body_explicit_and_inferred_agree_is_not_an_error() {
        let expanded = expand(
            "post",
            "path = \"/x\", request_body = Foo",
            "async fn handler(a: AppJson<Foo>) -> AppJson<Bar> { todo!() }",
        );
        assert!(!expanded.contains("compile_error"));
        assert!(norm(&expanded).contains("request_body=Foo"));
    }

    #[test]
    fn test_request_body_explicit_conflicts_with_inferred_is_compile_error() {
        let expanded = expand(
            "post",
            "path = \"/x\", request_body = Bar",
            "async fn handler(a: AppJson<Foo>) -> AppJson<Baz> { todo!() }",
        );
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("conflicts"));
    }

    #[test]
    fn test_response_inferred_from_app_json_return() {
        let expanded = expand(
            "get",
            "path = \"/x\"",
            "async fn handler() -> AppJson<Foo> { todo!() }",
        );
        assert!(norm(&expanded).contains("(status=200,body=Foo)"));
    }

    #[test]
    fn test_response_inferred_from_result_app_json_return() {
        let expanded = expand(
            "get",
            "path = \"/x\"",
            "async fn handler() -> Result<AppJson<Foo>, ApiError> { todo!() }",
        );
        let n = norm(&expanded);
        assert!(n.contains("(status=200,body=Foo)"));
        // ApiError legitimately appears in the preserved fn signature; make sure it never
        // ends up inside the generated responses(...) body.
        assert!(!n.contains("body=ApiError"));
    }

    #[test]
    fn test_response_no_inference_requires_explicit_responses() {
        let expanded = expand(
            "get",
            "path = \"/x\", responses((status = 200, body = String))",
            "async fn handler() -> &'static str { \"ok\" }",
        );
        assert!(!expanded.contains("compile_error"));
        assert!(norm(&expanded).contains("(status=200,body=String)"));
    }

    #[test]
    fn test_response_no_inference_and_no_explicit_responses_is_compile_error() {
        let expanded = expand(
            "get",
            "path = \"/x\"",
            "async fn handler() -> &'static str { \"ok\" }",
        );
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("cannot infer a 200 response"));
    }

    #[test]
    fn test_response_duplicate_200_is_compile_error() {
        let expanded = expand(
            "get",
            "path = \"/x\", responses((status = 200, body = Bar))",
            "async fn handler() -> AppJson<Foo> { todo!() }",
        );
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("conflicts with an explicit"));
    }

    #[test]
    fn test_response_explicit_error_appended_after_inferred_200() {
        let expanded = expand(
            "get",
            "path = \"/x\", responses((status = 404, body = String))",
            "async fn handler() -> Result<AppJson<Foo>, ApiError> { todo!() }",
        );
        let n = norm(&expanded);
        assert!(n.contains("(status=200,body=Foo)"));
        assert!(n.contains("(status=404,body=String)"));
    }

    #[test]
    fn test_params_inferred_from_app_path_single_primitive() {
        let expanded = expand(
            "get",
            "path = \"/widgets/{id}\", responses((status = 200, body = String))",
            "async fn handler(AppPath(id): AppPath<String>) -> &'static str { \"ok\" }",
        );
        assert!(norm(&expanded).contains("(\"id\"=String,Path)"));
    }

    #[test]
    fn test_params_inferred_from_app_path_typed_id_suffix_heuristic() {
        let expanded = expand(
            "get",
            "path = \"/widgets/{id}\", responses((status = 200, body = String))",
            "async fn handler(AppPath(id): AppPath<WidgetId>) -> &'static str { \"ok\" }",
        );
        assert!(norm(&expanded).contains("(\"id\"=WidgetId,Path)"));
    }

    #[test]
    fn test_params_inferred_from_app_path_struct_uses_into_params_form() {
        let expanded = expand(
            "get",
            "path = \"/widgets/{id}\", responses((status = 200, body = String))",
            "async fn handler(AppPath(p): AppPath<WidgetPathParams>) -> &'static str { \"ok\" }",
        );
        let n = norm(&expanded);
        assert!(n.contains("params(WidgetPathParams)"));
        assert!(!n.contains("Path)"));
    }

    #[test]
    fn test_params_inferred_from_app_path_multi_placeholder_uses_into_params_form() {
        let expanded = expand(
            "get",
            "path = \"/a/{x}/{y}\", responses((status = 200, body = String))",
            "async fn handler(AppPath(p): AppPath<String>) -> &'static str { \"ok\" }",
        );
        assert!(norm(&expanded).contains("params(String)"));
    }

    #[test]
    fn test_params_inferred_from_app_query() {
        let expanded = expand(
            "get",
            "path = \"/search\", responses((status = 200, body = String))",
            "async fn handler(AppQuery(q): AppQuery<SearchParams>) -> &'static str { \"ok\" }",
        );
        assert!(norm(&expanded).contains("params(SearchParams)"));
    }

    #[test]
    fn test_params_both_app_path_and_app_query_combine() {
        let expanded = expand(
            "get",
            "path = \"/widgets/{id}\", responses((status = 200, body = String))",
            "async fn handler(AppPath(id): AppPath<String>, AppQuery(q): AppQuery<SearchParams>) -> &'static str { \"ok\" }",
        );
        let n = norm(&expanded);
        assert!(n.contains("(\"id\"=String,Path)"));
        assert!(n.contains("SearchParams"));
    }

    #[test]
    fn test_params_multiple_app_path_is_compile_error() {
        let expanded = expand(
            "get",
            "path = \"/x/{a}/{b}\"",
            "async fn handler(a: AppPath<String>, b: AppPath<String>) -> &'static str { \"ok\" }",
        );
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("more than one AppPath"));
    }

    #[test]
    fn test_params_multiple_app_query_is_compile_error() {
        let expanded = expand(
            "get",
            "path = \"/x\"",
            "async fn handler(a: AppQuery<Foo>, b: AppQuery<Bar>) -> &'static str { \"ok\" }",
        );
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("more than one AppQuery"));
    }

    #[test]
    fn test_params_explicit_and_inferred_both_appear() {
        let expanded = expand(
            "get",
            "path = \"/widgets/{id}\", responses((status = 200, body = String)), params((\"legacy\" = String, Query))",
            "async fn handler(AppPath(id): AppPath<String>) -> &'static str { \"ok\" }",
        );
        let n = norm(&expanded);
        assert!(n.contains("(\"id\"=String,Path)"));
        assert!(n.contains("(\"legacy\"=String,Query)"));
    }

    #[test]
    fn test_route_method_fn_generates_inventory_registration() {
        let expanded = expand(
            "post",
            "path = \"/x\"",
            "async fn health(AppJson(r): AppJson<Foo>) -> AppJson<Bar> { todo!() }",
        );
        assert!(expanded.contains("RouteRegistration"));
        assert!(expanded.contains("inventory :: submit"));
        assert!(expanded.contains("routes ! (health)"));
    }

    #[test]
    fn test_route_method_fn_preserves_original_function_body() {
        let expanded = expand(
            "get",
            "path = \"/health\", responses((status = 200, body = String))",
            "async fn health() -> &'static str { \"ok\" }",
        );
        assert!(expanded.contains("\"ok\""));
    }

    #[test]
    fn test_get_emits_get_method_key() {
        let expanded = expand(
            "get",
            "path = \"/x\", responses((status = 200, body = String))",
            "async fn handler() -> &'static str { \"ok\" }",
        );
        assert!(expanded.contains("utoipa :: path (get ,"));
    }

    #[test]
    fn test_post_emits_post_method_key() {
        let expanded = expand(
            "post",
            "path = \"/x\", responses((status = 200, body = String))",
            "async fn handler() -> &'static str { \"ok\" }",
        );
        assert!(expanded.contains("utoipa :: path (post ,"));
    }

    #[test]
    fn test_stano_routes_path_resolves_to_direct_axum_dependency() {
        let manifest_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/direct_axum_consumer"
        );
        let _guard = ManifestDirGuard::set(manifest_dir);
        let path = stano_routes_path().to_string();
        assert!(path.contains("stano_axum"));
    }

    #[test]
    fn test_stano_routes_path_falls_back_to_stano_launcher() {
        let manifest_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/direct_launcher_consumer"
        );
        let _guard = ManifestDirGuard::set(manifest_dir);
        let path = stano_routes_path().to_string();
        assert!(path.contains("stano_launcher"));
        assert!(path.contains("stano_axum"));
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
    fn test_stano_routes_path_falls_back_to_stano_starter_rest() {
        let manifest_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/stano_starter_consumer"
        );
        let _guard = ManifestDirGuard::set(manifest_dir);
        let path = stano_routes_path().to_string();
        assert!(path.contains("stano_axum"));
    }

    #[test]
    fn test_stano_routes_path_panics_when_no_known_dependency_present() {
        let manifest_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/no_launcher_consumer"
        );
        let _guard = ManifestDirGuard::set(manifest_dir);
        let result = std::panic::catch_unwind(stano_routes_path);
        assert!(result.is_err());
    }
}
