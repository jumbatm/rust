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
///     #[suggest = "consider cloning here", suggest_code = "{opt_sugg}.clone()"]
///     pub opt_sugg: Option<Span>
/// }
/// ```
/// Then, later, to emit the error:
/// ```
/// sess.emit_err(MoveOutOfBorrowError {
///     expected,
///     actual,
///     span,
///     other_span,
///     suggestion: Some(suggestion),
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

    // Grab the error code, which should have been supplied with #[code = ...].
    let error_code = get_meta_value(attrs, "code").unwrap();

    // Also grab the primary message format string, which should also have been specified in the
    // same way.
    let primary_message_fmt = (|| Some(build_format(fields, &get_meta_value(attrs, "error")?)))()
        .unwrap_or(quote! { "" });

    // The name of the diagnostic builder we'll be working with.
    let diag = format_ident!("diag");

    let body = s.each(|binding| {
        let binding_ast = binding.ast();
        if let syn::Type::Path(typath) = &binding_ast.ty {
            let mut result = Vec::new();
            if typath.path.segments.last().unwrap().ident == "Span" {
                if let Some(msg) = get_meta_value(&binding_ast.attrs, "error") {
                    let formatted_str = build_format(fields, &msg);
                    result.push(quote! {
                        // FIXME: Should error out if #[error = ] was specified more than once.
                        #diag.set_span(*#binding);
                        #diag.set_primary_message(#formatted_str);
                    });
                }
                if let Some(msg) = get_meta_value(&binding_ast.attrs, "label") {
                    let formatted_str = build_format(fields, &msg);
                    result.push(quote! {
                        diag.span_label(*#binding, #formatted_str);
                    });
                }
                return quote! {
                    #(#result);*
                };
            }
        }
        // Do nothing for this field.
        // FIXME: Should filter instead of generating dead code?
        quote! {}
    });

    // FIXME: Replace this with gen_impl.
    // FIXME: Want to be able to specify a Lint DiagnosticId too.
    s.gen_impl(quote! {
        gen impl<'a> rustc_errors::AsError<'a> for @Self {
            type Session = rustc_session::Session;
            fn as_error(self, sess: &'a Self::Session) -> rustc_errors::DiagnosticBuilder {
                let mut #diag = sess.struct_err_with_code(#primary_message_fmt, rustc_errors::DiagnosticId::Error(#error_code.to_string()));
                match self {
                    #body
                }
                #diag
            }
        }
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
fn build_format(fields: &syn::Fields, input: &String) -> proc_macro2::TokenStream {
    // Keep track of which fields have been referenced. First, initialise all to false:
    let mut field_referenced: HashMap<String, bool> = HashMap::new();
    fields.iter().for_each(|f| {
        field_referenced.insert(f.ident.as_ref().unwrap().to_string(), false);
    });

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
            let referenced_field = eat_argument();
            *field_referenced
                .get_mut(&referenced_field)
                .expect(&format!("`{}` is not a field in this struct", &referenced_field)) = true;
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
