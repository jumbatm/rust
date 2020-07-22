#![allow(unused)] // FIXME: Remove
use quote::format_ident;
use quote::quote;

use std::collections::{HashMap, HashSet};

/// Implements #[derive(AsSessionError)], which allows for errors to be specified as a struct, independent
/// from the actual diagnostics emitting code.
/// ```
/// #[derive(AsSessionError)]
/// #[code = E0505]
/// #[error = "cannot move out of {name} because it is borrowed"]
/// pub struct MoveOutOfBorrowError {
///     pub name: Symbol,
///     pub ty: Ty,
///     #[label = "cannot move out of borrow"]
///     pub span: Span
///     #[label = "`{ty}` first borrowed here"]
///     pub other_span: Span
///     #[suggest(msg = "consider cloning here", code = "{name}.clone()")]
///     pub opt_sugg: Option<(Span, Applicability)>
/// }
/// ```
/// Then, later, to emit the error:
/// ```
/// sess.emit_err(MoveOutOfBorrowError {
///     expected,
///     actual,
///     span,
///     other_span,
///     opt_sugg: Some(suggestion, Applicability::MachineApplicable),
/// });
/// ```
pub fn as_session_error_derive(s: synstructure::Structure<'_>) -> proc_macro2::TokenStream {
    let ast = &s.ast();
    let attrs = &ast.attrs;
    let get_meta_value = |attrs: &[syn::Attribute], key: &str| {
        if let syn::Meta::NameValue(syn::MetaNameValue { lit: syn::Lit::Str(s), .. }) = attrs
            .iter()
            .find(|&attr| attr.path.segments.first().unwrap().ident == key)?
            .parse_meta()
            .expect("Could not parse attribute contents")
        {
            Some(s.value())
        } else {
            None
        }
    };

    let fields: &syn::Fields = if let syn::Data::Struct(syn::DataStruct { fields, .. }) = &ast.data
    {
        Some(fields)
    } else {
        None
    }
    .unwrap();

    // The name of the diagnostic builder we'll be working with.
    let diag = format_ident!("diag");

    let mut builder = SessionDeriveBuilder::new(&diag, fields);

    // FIXME: Is there a way to avoid needing a collect() here?
    let preamble: Vec<_> = attrs
        .iter()
        .map(|attr| {
            builder.generate_structure_code(attr, VariantInfo { ident: &ast.ident }).unwrap()
        })
        .collect();

    // FIXME: Could move all the logic into a single function. fn(Key, type) -> TokenStream. Then,
    // on this side, would just need to walk all the attributes on this struct. Using each here is
    // beneficial because it lets enums be used to dispatch slightly-differing messages.
    let body = s.each(|field_binding| {
        let field = field_binding.ast();
        let result = field.attrs.iter().map(|attr| {
            builder
                .generate_field_code(
                    &field_binding.binding,
                    attr,
                    FieldInfo {
                        vis: &field.vis,
                        binding: field_binding,
                        ident: field.ident.as_ref(),
                        ty: &field.ty,
                    },
                )
                .unwrap()
        });
        return quote! {
            #(#result);*
        };
    });

    // FIXME: Replace this with gen_impl.
    // FIXME: Want to be able to specify a Lint DiagnosticId too.
    s.gen_impl(quote! {
        gen impl<'a> rustc_errors::AsError<'a> for @Self {
            type Session = rustc_session::Session;
            fn as_error(self, sess: &'a Self::Session) -> rustc_errors::DiagnosticBuilder {
                let mut #diag = sess.struct_err("");
                #(#preamble)*;
                match self {
                    #body
                }
                #diag
            }
        }
    })
}

struct FieldInfo<'a> {
    vis: &'a syn::Visibility,
    ident: Option<&'a syn::Ident>,
    binding: &'a synstructure::BindingInfo<'a>,
    ty: &'a syn::Type,
}

struct VariantInfo<'a> {
    ident: &'a syn::Ident,
}

// Checks whether the type name of `ty` matches `name`.
//
// Given some struct at a::b::c::Foo, this will return true for c::Foo, b::c::Foo, or
// a::b::c::Foo. This reasonably allows qualified names to be used in the macro.
fn type_matches(ty: &syn::TypePath, name: &[&str]) -> bool {
    ty.path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .rev()
        .zip(name.iter().rev())
        .all(|(x, y)| &x.as_str() == y)
}

struct SessionDeriveBuilder<'a> {
    /// Store a map of field name to its corresponding field. This is built on construction of the
    /// derive builder.
    fields: HashMap<String, &'a syn::Field>,
    diag: &'a syn::Ident,
}

impl<'a> SessionDeriveBuilder<'a> {
    fn new(diag: &'a syn::Ident, fields: &'a syn::Fields) -> Self {
        let mut fields_map = HashMap::new();
        for field in fields.iter() {
            if let Some(ident) = &field.ident {
                fields_map.insert(ident.to_string(), field);
            }
        }
        Self { diag, fields: fields_map }
    }

    fn generate_structure_code(
        &mut self,
        attr: &syn::Attribute,
        info: VariantInfo<'a>,
    ) -> syn::Result<proc_macro2::TokenStream> {
        let diag = self.diag;
        Ok(match attr.parse_meta()? {
            syn::Meta::NameValue(syn::MetaNameValue { lit: syn::Lit::Str(s), .. }) => {
                let formatted_str = self.build_format(&s.value());
                let name = attr.path.segments.last().unwrap().ident.to_string();
                let name = name.as_str();
                match name {
                    "error" => {
                        quote! {
                            #diag.set_primary_message(#formatted_str);
                        }
                    }
                    "code" => {
                        quote! {
                            #diag.code(rustc_errors::DiagnosticId::Error(#formatted_str));
                        }
                    }
                    _ => unimplemented!(),
                }
            }
            _ => todo!("unhandled meta kind"),
        })
    }

    /// In the strings in the attributes supplied to this macro, we want callers to be able to
    /// reference fields in the format string. Take this, for example:
    /// ```
    /// struct Point {
    ///     #[error = "Expected a point greater than ({x}, {y})"]
    ///     x: i32,
    ///     y: i32,
    /// }
    /// ```
    /// We want to automatically pick up that {x} refers `self.x` and {y} refers to `self.y`, then
    /// generate this call to format!:
    /// ```
    /// format!("Expected a point greater than ({x}, {y})", x = self.x, y = self.y)
    /// ```
    /// This function builds the entire call to format!.
    fn build_format(&self, input: &String) -> proc_macro2::TokenStream {
        // Keep track of which fields have been referenced. First, initialise all to false:
        let mut field_referenced: HashMap<String, bool> = HashMap::new();
        for field_name in self.fields.iter().map(|(k, v)| k) {
            field_referenced.insert(field_name.clone(), false);
        }

        // At this point, we can start parsing the format string.
        let mut it = input.chars().peekable();
        // Once the start of a format string has been found, process the format string and spit out
        // the referenced fields. Leaves `it` sitting on the closing brace of the format string, so the
        // next call to `it.next()` retrieves the next character.
        while let Some(c) = it.next() {
            if c == '{' && *it.peek().unwrap_or(&'\0') != '{' {
                #[must_use]
                let mut eat_argument = || -> String {
                    let mut result = String::new();
                    // Format specifiers look like
                    // format   := '{' [ argument ] [ ':' format_spec ] '}' .
                    // Therefore, we only need to eat until ':' or '}' to find the argument.
                    while let Some(c) = it.next() {
                        result.push(c);
                        let next = *it.peek().unwrap_or(&'\0');
                        if next == '}' {
                            break;
                        } else if next == ':' {
                            // Eat the ':' character.
                            assert_eq!(it.next().unwrap(), ':');
                            break;
                        }
                    }
                    // Eat until (and including) the matching '}'
                    while it
                        .next()
                        .expect("Fell off end of format string without finding closing brace")
                        != '}'
                    {
                        continue;
                    }
                    result
                };

                let referenced_field = eat_argument(); // FIXME: Inline eat_argument
                *field_referenced
                    .get_mut(&referenced_field)
                    .expect(&format!("`{}` is not a field in this struct", &referenced_field)) =
                    true;
            }
        }
        let args = field_referenced
            .into_iter()
            .filter_map(|(k, v)| if v { Some(k) } else { None })
            .map(|field: String| {
                let field_ident = format_ident!("{}", field);
                quote! {
                    #field_ident = &self.#field_ident
                }
            });
        quote! {
        format!(#input #(,#args)*)
        }
    }

    fn generate_field_code(
        &mut self,
        field_binding: &syn::Ident,
        attr: &syn::Attribute,
        info: FieldInfo<'_>,
    ) -> syn::Result<proc_macro2::TokenStream> {
        let diag = &self.diag;
        let name = attr.path.segments.last().unwrap().ident.to_string();
        let name = name.as_str();
        // At this point, we need to dispatch based on the attribute key + the
        // type.
        let meta = attr.parse_meta()?;
        let field_ty_path = if let syn::Type::Path(path) = info.ty { path } else { todo!() };
        Ok(match meta {
            syn::Meta::NameValue(syn::MetaNameValue { lit: syn::Lit::Str(s), .. }) => {
                let formatted_str = self.build_format(&s.value());
                match name {
                    "error" => {
                        if type_matches(field_ty_path, &["rustc_span", "Span"]) {
                            quote! {
                                #diag.set_span(*#field_binding);
                                #diag.set_primary_message(#formatted_str);
                            }
                        } else {
                            quote! {
                                #diag.set_primary_message(#formatted_str);
                            }
                        }
                    }
                    "label" => {
                        if type_matches(field_ty_path, &["rustc_span", "Span"]) {
                            quote! {
                                #diag.span_label(*#field_binding, #formatted_str);
                            }
                        } else {
                            todo!("Error: Label applied to non-Span field ({:?})", field_ty_path)
                        }
                    }
                    other => todo!("Unrecognised field: {}", other),
                }
            }
            _ => todo!("unhandled meta kind"),
        })
    }
}
