//! Procedural macros for RustUI 0.2: `#[derive(Observable)]` and `#[preview]`.

use std::fmt::Write as _;

use proc_macro::{Delimiter, TokenStream, TokenTree};

fn core_path_for_current_crate() -> &'static str {
    let pkg = option_env_or_runtime("CARGO_PKG_NAME");
    let crate_name = option_env_or_runtime("CARGO_CRATE_NAME");
    match (pkg.as_deref(), crate_name.as_deref()) {
        (Some("rustui-core"), Some("rustui_core")) => "crate",
        (
            Some(
                "rustui-core" | "rustui-layout" | "rustui-render" | "rustui-text"
                | "rustui-platform" | "rustui-widgets",
            ),
            _,
        ) => "::rustui_core",
        (Some("rustui"), Some("rustui")) => "crate::core",
        _ => "::rustui::core",
    }
}

fn option_env_or_runtime(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn compile_err(msg: &str) -> TokenStream {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    format!("compile_error!(\"{escaped}\");")
        .parse()
        .unwrap_or_default()
}

struct ParsedField {
    name: String,
    ty: String,
}

/// Generates a reactive model handle `<Name>Observable` with one `State<T>` signal per field
/// and implements `ObservableModel` so `cx.model(MyStruct { ... })` yields a `Copy` handle.
///
/// Each field `foo: T` exposes:
/// - `model.foo() -> T` (subscribes the current reactive scope to `foo` only)
/// - `model.foo_untracked() -> T`
/// - `model.set_foo(value: T)`
/// - `model.update_foo(|v| ...)`
/// - `model.foo_binding() -> Binding<T>`
/// - `model.foo_state() -> State<T>`
#[proc_macro_derive(Observable, attributes(observable))]
pub fn derive_observable(input: TokenStream) -> TokenStream {
    let trees: Vec<TokenTree> = input.into_iter().collect();
    let mut core_override: Option<String> = None;
    let mut i = 0usize;

    // Parse leading attributes and visibility up to `struct`.
    let mut vis_tokens = Vec::new();
    let mut found_struct = false;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                if let Some(TokenTree::Group(g)) = trees.get(i + 1)
                    && let Some(override_path) = parse_observable_attr(g.stream())
                {
                    core_override = Some(override_path);
                }
                i += 2;
            }
            TokenTree::Ident(id) if id.to_string() == "struct" => {
                found_struct = true;
                i += 1;
                break;
            }
            TokenTree::Ident(id) if matches!(id.to_string().as_str(), "enum" | "union") => {
                return compile_err("#[derive(Observable)] only supports structs");
            }
            other => {
                vis_tokens.push(other.to_string());
                i += 1;
            }
        }
    }

    if !found_struct {
        return compile_err("#[derive(Observable)] expected a struct definition");
    }

    let Some(TokenTree::Ident(struct_ident)) = trees.get(i) else {
        return compile_err("#[derive(Observable)] missing struct identifier");
    };
    let struct_name = struct_ident.to_string();
    i += 1;

    // Skip any generics / where clause until the `{ ... }` body group.
    let mut brace_group = None;
    while i < trees.len() {
        if let TokenTree::Group(g) = &trees[i]
            && g.delimiter() == Delimiter::Brace
        {
            brace_group = Some(g.clone());
            break;
        }
        if let TokenTree::Group(g) = &trees[i]
            && g.delimiter() == Delimiter::Parenthesis
        {
            return compile_err("#[derive(Observable)] does not support tuple structs");
        }
        i += 1;
    }

    let Some(body_group) = brace_group else {
        return compile_err("#[derive(Observable)] requires a braced struct with named fields");
    };

    let fields = match parse_struct_fields(body_group.stream()) {
        Ok(f) => f,
        Err(err_ts) => return err_ts,
    };

    let core = core_override
        .as_deref()
        .unwrap_or_else(|| core_path_for_current_crate());
    let vis = vis_tokens.join(" ");
    let handle_name = format!("{struct_name}Observable");

    let mut out = String::new();
    let _ = writeln!(out, "#[derive(Clone, Copy)]");
    let _ = writeln!(out, "{vis} struct {handle_name} {{");
    for f in &fields {
        let _ = writeln!(
            out,
            "    pub {name}: {core}::State<{ty}>,",
            name = f.name,
            ty = f.ty
        );
    }
    let _ = writeln!(out, "}}");

    // Manual Debug impl avoids requiring every field type to be tracked or formatted individually
    // and satisfies missing_debug_implementations across crates.
    let _ = writeln!(out, "impl ::std::fmt::Debug for {handle_name} {{");
    let _ = writeln!(
        out,
        "    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {{"
    );
    let _ = writeln!(
        out,
        "        f.debug_struct(\"{handle_name}\").finish_non_exhaustive()"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "impl {handle_name} {{");
    for f in &fields {
        let name = &f.name;
        let ty = &f.ty;
        let _ = writeln!(
            out,
            "    #[allow(dead_code)] pub fn {name}(&self) -> {ty} {{ self.{name}.get() }}"
        );
        let _ = writeln!(
            out,
            "    #[allow(dead_code)] pub fn {name}_untracked(&self) -> {ty} {{ self.{name}.get_untracked() }}"
        );
        let _ = writeln!(
            out,
            "    #[allow(dead_code)] pub fn set_{name}(&self, value: {ty}) {{ self.{name}.set(value); }}"
        );
        let _ = writeln!(
            out,
            "    #[allow(dead_code)] pub fn update_{name}(&self, f: impl FnOnce(&mut {ty})) {{ self.{name}.update(f); }}"
        );
        let _ = writeln!(
            out,
            "    #[allow(dead_code)] pub fn {name}_binding(&self) -> {core}::Binding<{ty}> {{ self.{name}.binding() }}"
        );
        let _ = writeln!(
            out,
            "    #[allow(dead_code)] pub fn {name}_state(&self) -> {core}::State<{ty}> {{ self.{name} }}"
        );
    }
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "impl {core}::ObservableModel for {struct_name} {{");
    let _ = writeln!(out, "    type Handle = {handle_name};");
    let _ = writeln!(
        out,
        "    fn into_model(self, rt: &{core}::Runtime, owner: Option<{core}::NodeId>) -> Self::Handle {{"
    );
    let _ = writeln!(out, "        {handle_name} {{");
    for f in &fields {
        let _ = writeln!(
            out,
            "            {name}: rt.create_state(self.{name}, owner),",
            name = f.name
        );
    }
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");

    out.parse()
        .unwrap_or_else(|e| compile_err(&format!("failed to parse generated Observable code: {e}")))
}

fn parse_observable_attr(attr_stream: TokenStream) -> Option<String> {
    let items: Vec<TokenTree> = attr_stream.into_iter().collect();
    if let (Some(TokenTree::Ident(attr_name)), Some(TokenTree::Group(inner))) =
        (items.first(), items.get(1))
        && attr_name.to_string() == "observable"
    {
        let inner_items: Vec<TokenTree> = inner.stream().into_iter().collect();
        if let (Some(TokenTree::Ident(k)), Some(TokenTree::Punct(eq))) =
            (inner_items.first(), inner_items.get(1))
            && matches!(k.to_string().as_str(), "core" | "crate")
            && eq.as_char() == '='
        {
            let rest: String = inner_items[2..]
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join("");
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    None
}

fn parse_struct_fields(stream: TokenStream) -> Result<Vec<ParsedField>, TokenStream> {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut fields = Vec::new();
    let mut i = 0usize;

    while i < tokens.len() {
        // Skip field attributes.
        while matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == '#') {
            i += 2;
        }
        if i >= tokens.len() {
            break;
        }
        // Skip visibility (`pub`, `pub(crate)`, `pub(super)`).
        if matches!(tokens.get(i), Some(TokenTree::Ident(id)) if id.to_string() == "pub") {
            i += 1;
            if matches!(tokens.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
            {
                i += 1;
            }
        }
        let Some(TokenTree::Ident(field_ident)) = tokens.get(i) else {
            return Err(compile_err(
                "#[derive(Observable)] expected field identifier",
            ));
        };
        let field_name = field_ident.to_string();
        i += 1;

        let Some(TokenTree::Punct(colon)) = tokens.get(i) else {
            return Err(compile_err(
                "#[derive(Observable)] expected `:` after field name",
            ));
        };
        if colon.as_char() != ':' {
            return Err(compile_err(
                "#[derive(Observable)] expected `:` after field name",
            ));
        }
        i += 1;

        // Collect tokens for field type until top-level `,` (tracking `<...>` nesting).
        let mut angle_depth = 0usize;
        let mut ty_tokens = Vec::new();
        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Punct(p) if p.as_char() == '<' => {
                    angle_depth += 1;
                    ty_tokens.push(tokens[i].to_string());
                }
                TokenTree::Punct(p) if p.as_char() == '>' => {
                    angle_depth = angle_depth.saturating_sub(1);
                    ty_tokens.push(tokens[i].to_string());
                }
                TokenTree::Punct(p) if p.as_char() == ',' && angle_depth == 0 => {
                    i += 1;
                    break;
                }
                other => {
                    ty_tokens.push(other.to_string());
                }
            }
            i += 1;
        }
        if ty_tokens.is_empty() {
            return Err(compile_err(&format!(
                "#[derive(Observable)] missing type for field `{field_name}`"
            )));
        }
        fields.push(ParsedField {
            name: field_name,
            ty: ty_tokens.join(" "),
        });
    }

    Ok(fields)
}

/// Marks a RustUI view helper function as a live preview entry point for Anvil IDE and `rustui dev`.
///
/// Preserves the original function and generates `<fn_name>_preview_spec()` returning
/// `(&'static str, fn() -> AnyView)`.
#[proc_macro_attribute]
pub fn preview(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_str = attr.to_string();
    let trimmed_attr = attr_str.trim().trim_matches('"');

    let item_str = item.to_string();
    let tokens: Vec<TokenTree> = item.clone().into_iter().collect();
    let mut fn_name = None;
    for idx in 0..tokens.len() {
        if let TokenTree::Ident(id) = &tokens[idx]
            && id.to_string() == "fn"
            && let Some(TokenTree::Ident(name)) = tokens.get(idx + 1)
        {
            fn_name = Some(name.to_string());
            break;
        }
    }

    let Some(name) = fn_name else {
        return compile_err("#[preview] must be attached to a `fn` returning `impl View`");
    };
    let label = if trimmed_attr.is_empty() {
        name.clone()
    } else {
        trimmed_attr.to_owned()
    };
    let escaped_label = label.replace('\\', "\\\\").replace('"', "\\\"");
    let core = core_path_for_current_crate();
    let spec_fn = format!("{name}_preview_spec");

    let extra = format!(
        "\n#[allow(dead_code)]\npub fn {spec_fn}() -> (&'static str, fn() -> {core}::AnyView) {{\n    fn __build() -> {core}::AnyView {{ {core}::AnyView::new({name}()) }}\n    (\"{escaped_label}\", __build)\n}}\n"
    );

    format!("{item_str}\n{extra}")
        .parse()
        .unwrap_or_else(|e| compile_err(&format!("failed to emit #[preview] helper: {e}")))
}
