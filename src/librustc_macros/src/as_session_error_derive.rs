#![allow(unreachable_code)]
#![allow(unused)]
use quote::format_ident;
use quote::quote;

use proc_macro::Diagnostic;
use syn::spanned::Spanned;

use std::collections::{HashMap, HashSet};

/// Implements #[derive(AsSessionError)], which allows for errors to be specified as a struct, independent
/// from the actual diagnostics emitting code.
/// ```
/// # extern crate rustc_errors;
/// # use rustc_errors::Applicability;
/// # extern crate rustc_span;
/// # use rustc_span::{symbol::Ident, Span};
/// # extern crate rust_middle;
/// # use rustc_middle::ty::Ty;
/// #[derive(AsSessionError)]
/// #[code = "E0505"]
/// #[error = "cannot move out of {name} because it is borrowed"]
/// pub struct MoveOutOfBorrowError<'tcx> {
///     pub name: Ident,
///     pub ty: Ty<'tcx>,
///     #[label = "cannot move out of borrow"]
///     pub span: Span,
///     #[label = "`{ty}` first borrowed here"]
///     pub other_span: Span,
///     #[suggestion(message = "consider cloning here", code = "{name}.clone()")]
///     pub opt_sugg: Option<(Span, Applicability)>
/// }
/// ```
/// Then, later, to emit the error:
///
/// ```ignore (todo-make-this-not-ignore)
/// sess.emit_err(MoveOutOfBorrowError {
///     expected,
///     actual,
///     span,
///     other_span,
///     opt_sugg: Some(suggestion, Applicability::MachineApplicable),
/// });
/// ```
// FIXME: Make the marked example above not ignore anymore once that API is implemented.
pub fn as_session_error_derive(s: synstructure::Structure<'_>) -> proc_macro2::TokenStream {
    // Names for the diagnostic we build and the session we build it from.
    let diag = format_ident!("diag");
    let sess = format_ident!("sess");

    let mut builder = SessionDeriveBuilder::new(diag, sess, s);
    builder.build()
}

// FIXME: Remove unused fields.
#[allow(unused)]
struct FieldInfo<'a> {
    vis: &'a syn::Visibility,
    binding: &'a synstructure::BindingInfo<'a>,
    ty: &'a syn::Type,
}

#[allow(unused)]
struct VariantInfo<'a> {
    ident: &'a syn::Ident,
}

// Checks whether the type name of `ty` matches `name`.
//
// Given some struct at a::b::c::Foo, this will return true for c::Foo, b::c::Foo, or
// a::b::c::Foo. This reasonably allows qualified names to be used in the macro.
fn type_matches_path(ty: &syn::Type, name: &[&str]) -> bool {
    if let syn::Type::Path(ty) = ty {
        ty.path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .rev()
            .zip(name.iter().rev())
            .all(|(x, y)| &x.as_str() == y)
    } else {
        false
    }
}

/// The central struct for constructing the as_error method from an annotated struct.
struct SessionDeriveBuilder<'a> {
    structure: synstructure::Structure<'a>,
    state: SessionDeriveBuilderState<'a>,
}

#[allow(unused)]
enum DiagnosticId {
    Error(proc_macro2::TokenStream),
    Lint(proc_macro2::TokenStream),
}

#[derive(Debug)]
enum SessionDeriveBuilderErrorKind {
    SynError(syn::Error),
    IdNotProvided,
    IdMultiplyProvided,
}

#[derive(Debug)]
struct SessionDeriveBuilderError {
    kind: SessionDeriveBuilderErrorKind,
    span: proc_macro2::Span,
}

impl SessionDeriveBuilderError {
    // FIXME: Implement ToTokens?
    fn to_tokens(self) -> proc_macro2::TokenStream {
        let msg = match self.kind {
            SessionDeriveBuilderErrorKind::IdMultiplyProvided => "Diagnostic ID multiply provided",
            SessionDeriveBuilderErrorKind::IdNotProvided => {
                "Diagnostic ID not provided" // FIXME: Add help message.
            }
            SessionDeriveBuilderErrorKind::SynError(e) => {
                return e.to_compile_error();
            }
        };
        Diagnostic::spanned(self.span.unwrap(), proc_macro::Level::Error, msg).emit();
        return quote!();
    }
}

impl std::convert::From<syn::Error> for SessionDeriveBuilderError {
    fn from(e: syn::Error) -> Self {
        SessionDeriveBuilderError {
            span: e.span(),
            kind: SessionDeriveBuilderErrorKind::SynError(e),
        }
    }
}

impl<'a> SessionDeriveBuilder<'a> {
    fn new(diag: syn::Ident, sess: syn::Ident, structure: synstructure::Structure<'a>) -> Self {
        // Build the mapping of field names to fields. This allows attributes to peek values from
        // other fields.
        let mut fields_map = HashMap::new();

        // Convenience bindings.
        let ast = structure.ast();
        let attrs = &ast.attrs;

        let fields: &syn::Fields =
            if let syn::Data::Struct(syn::DataStruct { fields, .. }) = &ast.data {
                fields
            } else {
                todo!("#[derive(AsSessionError)] can't yet be used on enums")
            };
        for field in fields.iter() {
            if let Some(ident) = &field.ident {
                fields_map.insert(ident.to_string(), field);
            }
        }

        Self {
            state: SessionDeriveBuilderState { diag, sess, fields: fields_map, kind: None },
            structure,
        }
    }
    fn build(mut self) -> proc_macro2::TokenStream {
        let SessionDeriveBuilder { structure, mut state } = self;

        let ast = structure.ast();
        let attrs = &ast.attrs;

        // FIXME: Is there a way to avoid needing a collect() here?
        let preamble: Vec<_> = attrs
            .iter()
            .map(|attr| {
                state
                    .generate_structure_code(attr, VariantInfo { ident: &ast.ident })
                    .unwrap_or_else(|v| v.to_tokens())
            })
            .collect();

        let body = structure.each(|field_binding| {
            let field = field_binding.ast();
            let result = field.attrs.iter().map(|attr| {
                state
                    .generate_field_code(
                        attr,
                        FieldInfo { vis: &field.vis, binding: field_binding, ty: &field.ty },
                    )
                    .unwrap_or_else(|v| v.to_tokens())
            });
            return quote! {
                #(#result);*
            };
        });

        // Finally, put it all together.
        let sess = &state.sess;
        let diag = &state.diag;
        let implementation = match state.kind {
            None => Err(SessionDeriveBuilderError {
                kind: SessionDeriveBuilderErrorKind::IdNotProvided,
                span: structure.ast().span(),
            }),
            Some(kind) => Ok(match kind {
                DiagnosticId::Lint(_lint) => todo!(),
                DiagnosticId::Error(code) => {
                    quote! {
                        let mut #diag = #sess.struct_err_with_code("", rustc_errors::DiagnosticId::Error(#code));
                        #(#preamble)*;
                        match self {
                            #body
                        }
                        #diag
                    }
                }
            }),
        };

        let implementation = match implementation {
            Ok(x) => x,
            Err(e) => e.to_tokens(),
        };

        structure.gen_impl(quote! {
            gen impl<'a> rustc_errors::AsError<'a> for @Self {
                type Session = rustc_session::Session;
                fn as_error(self, #sess: &'a Self::Session) -> rustc_errors::DiagnosticBuilder {
                    #implementation
                }
            }
        })
    }
}

/// Contains all persistent information required for building up the individual calls in the
/// as_error method. This is a separate struct to later be able to split self.state and the
/// self.structure up to avoid a double mut borrow of self when calling the generate_* inside the
/// closure passed to self.structure.each.
struct SessionDeriveBuilderState<'a> {
    /// Name of the session parameter that's passed in to the as_error method.
    sess: syn::Ident,

    /// Store a map of field name to its corresponding field. This is built on construction of the
    /// derive builder.
    fields: HashMap<String, &'a syn::Field>,

    /// The identifier to use for the generated DiagnosticBuilder instance.
    diag: syn::Ident,

    /// Whether this is a lint or an error. This dictates how the diag will be initialised.
    kind: Option<DiagnosticId>,
}

#[deny(unused_must_use)]
impl<'a> SessionDeriveBuilderState<'a> {
    fn generate_structure_code(
        &mut self,
        attr: &syn::Attribute,
        _info: VariantInfo<'a>, // FIXME: Remove this parameter?
    ) -> Result<proc_macro2::TokenStream, SessionDeriveBuilderError> {
        let diag = &self.diag;
        Ok(match attr.parse_meta()? {
            syn::Meta::NameValue(syn::MetaNameValue { lit: syn::Lit::Str(s), .. }) => {
                let formatted_str = self.build_format(&s.value(), attr.span());
                let name = attr.path.segments.last().unwrap().ident.to_string();
                let name = name.as_str();
                match name {
                    "error" => {
                        quote! {
                            #diag.set_primary_message(#formatted_str);
                        }
                    }
                    "code" => {
                        self.set_kind_once(DiagnosticId::Error(formatted_str), attr.span())?;
                        // This attribute is only allowed to be applied once, and the attribute
                        // will be set in the initialisation code.
                        quote! {}
                    }
                    "lint" => {
                        self.set_kind_once(DiagnosticId::Lint(formatted_str), attr.span())?;
                        // As with `code`, this attribute is only allowed once.
                        quote! {}
                    }
                    other => unimplemented!("Didn't recognise name: {}", other),
                }
            }
            _ => todo!("unhandled meta kind"),
        })
    }

    #[must_use]
    fn set_kind_once(
        &mut self,
        kind: DiagnosticId,
        span: proc_macro2::Span,
    ) -> Result<(), SessionDeriveBuilderError> {
        if self.kind.is_none() {
            self.kind = Some(kind);
            Ok(())
        } else {
            Err(SessionDeriveBuilderError {
                kind: SessionDeriveBuilderErrorKind::IdMultiplyProvided,
                span,
            })
        }
    }

    fn generate_field_code(
        &mut self,
        attr: &syn::Attribute,
        info: FieldInfo<'_>,
    ) -> Result<proc_macro2::TokenStream, SessionDeriveBuilderError> {
        let diag = &self.diag;
        let field_binding = &info.binding.binding;
        let name = attr.path.segments.last().unwrap().ident.to_string();
        let name = name.as_str();

        let option_ty = option_inner_ty(&info.ty);

        let generated_code = self.generate_non_option_field_code(
            attr,
            FieldInfo { vis: info.vis, binding: info.binding, ty: option_ty.unwrap_or(&info.ty) },
        )?;
        Ok(if option_ty.is_none() {
            quote! { #generated_code }
        } else {
            quote! {
                if let Some(#field_binding) = #field_binding {
                    #generated_code
                }
            }
        })
    }

    fn generate_non_option_field_code(
        &mut self,
        attr: &syn::Attribute,
        info: FieldInfo<'_>,
    ) -> Result<proc_macro2::TokenStream, SessionDeriveBuilderError> {
        let diag = &self.diag;
        let field_binding = &info.binding.binding;
        let name = attr.path.segments.last().unwrap().ident.to_string();
        let name = name.as_str();
        // At this point, we need to dispatch based on the attribute key + the
        // type.
        let meta = attr.parse_meta()?;
        Ok(match meta {
            syn::Meta::NameValue(syn::MetaNameValue { lit: syn::Lit::Str(s), .. }) => {
                let formatted_str = self.build_format(&s.value(), attr.span());
                match name {
                    "error" => {
                        if type_matches_path(&info.ty, &["rustc_span", "Span"]) {
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
                        if type_matches_path(&info.ty, &["rustc_span", "Span"]) {
                            quote! {
                                #diag.span_label(*#field_binding, #formatted_str);
                            }
                        } else {
                            Diagnostic::spanned(attr.span().unwrap(), proc_macro::Level::Error, "The `#[label = ...]` attribute can only be applied to fields of type Span").emit();
                            quote!()
                        }
                    }
                    other => todo!("Unrecognised field: {}", other),
                }
            }
            syn::Meta::List(list) => {
                match list.path.segments.iter().last().unwrap().ident.to_string().as_str() {
                    suggestion_kind @ "suggestion"
                    | suggestion_kind @ "suggestion_short"
                    | suggestion_kind @ "suggestion_hidden"
                    | suggestion_kind @ "suggestion_verbose" => {
                        // For suggest, we need to ensure we are running on a (Span,
                        // Applicability) pair.
                        let (span, applicability) = (|| {
                            if let syn::Type::Tuple(tup) = &info.ty {
                                let mut span_idx = None;
                                let mut applicability_idx = None;
                                for (idx, elem) in tup.elems.iter().enumerate() {
                                    if type_matches_path(elem, &["rustc_span", "Span"]) {
                                        if span_idx.is_none() {
                                            span_idx = Some(syn::Index::from(idx));
                                        } else {
                                            todo!("Error: field contains more than one span");
                                        }
                                    } else if type_matches_path(
                                        elem,
                                        &["rustc_errors", "Applicability"],
                                    ) {
                                        if applicability_idx.is_none() {
                                            applicability_idx = Some(syn::Index::from(idx));
                                        } else {
                                            todo!(
                                                "Error: field contains more than one Applicability"
                                            );
                                        }
                                    }
                                }
                                if let (Some(span_idx), Some(applicability_idx)) =
                                    (span_idx, applicability_idx)
                                {
                                    let binding = &info.binding.binding;
                                    let span = quote!(#binding.#span_idx);
                                    let applicability = quote!(#binding.#applicability_idx);
                                    (span, applicability)
                                } else {
                                    todo!("Error: Wrong types for suggestion")
                                }
                            } else {
                                // FIXME: This "wrong types for suggestion" message  (and the one
                                // above) should be replaced with some kind of Err return
                                unimplemented!("Error: Wrong types for suggestion")
                            }
                        };
                        // Now read the key-value pairs.
                        let mut msg = None;
                        let mut code = None;

                        for arg in list.nested.iter() {
                            if let syn::NestedMeta::Meta(syn::Meta::NameValue(arg_name_value)) = arg
                            {
                                if let syn::MetaNameValue { lit: syn::Lit::Str(s), .. } =
                                    arg_name_value
                                {
                                    let name = arg_name_value
                                        .path
                                        .segments
                                        .last()
                                        .unwrap()
                                        .ident
                                        .to_string();
                                    let name = name.as_str();
                                    let formatted_str = self.build_format(&s.value(), arg.span());
                                    match name {
                                        "message" => {
                                            msg = Some(formatted_str);
                                        }
                                        "code" => {
                                            code = Some(formatted_str);
                                        }
                                        _ => unimplemented!(
                                            "Expected `message` or `code`, got {}",
                                            name
                                        ),
                                    }
                                }
                            }
                        }
                        let msg = msg.map_or_else(
                            || unimplemented!("Error: missing suggestion message"),
                            |m| quote!(#m.as_str()),
                        );

                        let code = code.unwrap_or_else(|| quote! { String::new() });
                        // Now build it out:
                        let binding = &info.binding.binding;
                        let suggestion_method = format_ident!("span_{}", suggestion_kind);
                        quote! {
                            #diag.#suggestion_method(#span, #msg, #code, #applicability);
                        }
                    }
                    other => unimplemented!("Didn't recognise {} as a valid list name", other),
                }
            }
            _ => panic!("unhandled meta kind"),
        })
    }

    /// In the strings in the attributes supplied to this macro, we want callers to be able to
    /// reference fields in the format string. Take this, for example:
    /// ```ignore (not-usage-example)
    /// struct Point {
    ///     #[error = "Expected a point greater than ({x}, {y})"]
    ///     x: i32,
    ///     y: i32,
    /// }
    /// ```
    /// We want to automatically pick up that {x} refers `self.x` and {y} refers to `self.y`, then
    /// generate this call to format!:
    /// ```ignore (not-usage-example)
    /// format!("Expected a point greater than ({x}, {y})", x = self.x, y = self.y)
    /// ```
    /// This function builds the entire call to format!.
    fn build_format(&self, input: &String, span: proc_macro2::Span) -> proc_macro2::TokenStream {
        let mut referenced_fields: HashSet<String> = HashSet::new();

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
                referenced_fields.insert(referenced_field);
            }
        }
        // At this point, `referenced_fields` contains a set of the unique fields that were
        // referenced in the format string. Generate the corresponding "x = self.x" format
        // string parameters:
        let args = referenced_fields.into_iter().map(|field: String| {
            let field_ident = format_ident!("{}", field);
            let value = if self.fields.contains_key(&field) {
                quote! {
                    &self.#field_ident
                }
            } else {
                // This field doesn't exist. Emit a diagnostic.
                Diagnostic::spanned(
                    span.unwrap(),
                    proc_macro::Level::Error,
                    format!("no field `{}` on this type", field),
                )
                .emit();
                quote! {
                    "{#field}"
                }
            };
            quote! {
                #field_ident = #value
            }
        });
        quote! {
            format!(#input #(,#args)*)
        }
    }
}

/// /// If `ty` is an Option, returns Some(inner type). Else, returns None.
fn option_inner_ty(ty: &syn::Type) -> Option<&syn::Type> {
    if type_matches_path(ty, &["std", "option", "Option"]) {
        if let syn::Type::Path(ty_path) = ty {
            let path = &ty_path.path;
            let ty = path.segments.iter().last().unwrap();
            if let syn::PathArguments::AngleBracketed(bracketed) = &ty.arguments {
                if bracketed.args.len() == 1 {
                    if let syn::GenericArgument::Type(ty) = bracketed.args.iter().next().unwrap() {
                        return Some(ty);
                    }
                }
            }
        }
    }
    None
}
