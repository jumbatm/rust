use quote::format_ident;
use quote::quote;


use std::collections::HashMap;

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
    // Names for the diagnostic we build and the session we build it from.
    let diag = format_ident!("diag");
    let sess = format_ident!("sess");

    // Convenience bindings.
    let ast = s.ast();
    let attrs = &ast.attrs;
    let fields: &syn::Fields = if let syn::Data::Struct(syn::DataStruct { fields, .. }) = &ast.data
    {
       fields
    } else {
        todo!()
    };

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
                    attr,
                    FieldInfo {
                        vis: &field.vis,
                        binding: field_binding,
                        ty: &field.ty,
                    },
                )
                .unwrap()
        });
        return quote! {
            #(#result);*
        };
    });
    // Finally, put it all together.
    let implementation = match builder.kind {
        None => Err(SessionDeriveBuilderError::IdNotProvided),
        Some(kind) => Ok(match kind {
            DiagnosticId::Lint(_tokens) => todo!(),
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
    }
    .unwrap_or_else(|_| todo!("Proper error handling"));

    s.gen_impl(quote! {
        gen impl<'a> rustc_errors::AsError<'a> for @Self {
            type Session = rustc_session::Session;
            fn as_error(self, #sess: &'a Self::Session) -> rustc_errors::DiagnosticBuilder {
                #implementation
            }
        }
    })
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

    /// The identifier to use for the generated DiagnosticBuilder instance.
    diag: &'a syn::Ident,

    /// Whether this is a lint or an error. This dictates how the diag will be initialised.
    kind: Option<DiagnosticId>,
}

#[allow(unused)]
enum DiagnosticId {
    Error(proc_macro2::TokenStream),
    Lint(proc_macro2::TokenStream),
}

#[derive(Debug)]
enum SessionDeriveBuilderError {
    SynError(syn::Error),
    IdNotProvided,
    IdMultiplyProvided,
}

impl std::convert::From<syn::Error> for SessionDeriveBuilderError {
    fn from(e: syn::Error) -> Self {
        SessionDeriveBuilderError::SynError(e)
    }
}

#[deny(unused_must_use)]
impl<'a> SessionDeriveBuilder<'a> {
    fn new(diag: &'a syn::Ident, fields: &'a syn::Fields) -> Self {
        // Build the mapping of field names to fields. This allows attributes to peek values from
        // other fields.
        let mut fields_map = HashMap::new();
        for field in fields.iter() {
            if let Some(ident) = &field.ident {
                fields_map.insert(ident.to_string(), field);
            }
        }

        Self { diag, fields: fields_map, kind: None, }
    }

    fn generate_structure_code(
        &mut self,
        attr: &syn::Attribute,
        _info: VariantInfo<'a>, // FIXME: Remove this parameter?
    ) -> Result<proc_macro2::TokenStream, SessionDeriveBuilderError> {
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
                        self.set_kind_once(DiagnosticId::Error(formatted_str))?;
                        // This attribute is only allowed to be applied once, and the attribute
                        // will be set in the initialisation code.
                        quote! {}
                    }
                    "lint" => todo!(),
                    _ => unimplemented!(),
                }
            }
            _ => todo!("unhandled meta kind"),
        })
    }

    #[must_use]
    fn set_kind_once(&mut self, kind: DiagnosticId) -> Result<(), SessionDeriveBuilderError> {
        if self.kind.is_none() {
            self.kind = Some(kind);
            Ok(())
        } else {
            Err(SessionDeriveBuilderError::IdMultiplyProvided)
        }
    }

    fn generate_field_code(
        &mut self,
        attr: &syn::Attribute,
        info: FieldInfo<'_>,
    ) -> Result<proc_macro2::TokenStream, SessionDeriveBuilderError> {
        let diag = self.diag;
        let field_binding = &info.binding.binding;
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
        for field_name in self.fields.iter().map(|(k, _)| k) {
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
}
