// check-fail
// Tests error conditions for specifying diagnostics using #[derive(AsSessionError)]

#![feature(rustc_private)]
#![crate_type = "lib"]

extern crate rustc_span;
use rustc_span::Span;
use rustc_span::symbol::Ident;

extern crate rustc_macros;
use rustc_macros::AsSessionError;

extern crate rustc_middle;
use rustc_middle::ty::Ty;

extern crate rustc_errors;
use rustc_errors::Applicability;

extern crate rustc_session;

#[derive(AsSessionError)]
#[error = "Hello, world!"]
#[code = "E0123"]
struct Hello {}

#[derive(AsSessionError)]
#[error = "Hello, world!"]
#[code = "E0123"]
#[code = "E0456"] //~ ERROR Diagnostic ID multiply provided
struct ErrorSpecifiedTwice {}

// FIXME: Uncomment when emitting lints is supported.
/*
#[derive(AsSessionError)]
#[error = "Hello, world!"]
#[lint = "clashing_extern_declarations"]
#[lint = "improper_ctypes"] // FIXME: ERROR Diagnostic ID multiply provided
struct LintSpecifiedTwice {}
*/

#[derive(AsSessionError)]
#[code = "E0123"]
struct ErrorWithField {
    name: String,
    #[error = "This error has a field, and references {name}"]
    span: Span
}

#[derive(AsSessionError)]
#[code = "E0123"]
struct ErrorWithNonexistentField {
    #[error = "This error has a field, and references {name}"]
    //~^ ERROR no field `name` on this type
    span: Span
}

#[derive(AsSessionError)]
#[code = "E0123"]
#[error = "Something something"]
struct LabelOnSpan {
    #[label = "See here"]
    sp: Span
}

#[derive(AsSessionError)]
#[code = "E0123"]
#[error = "Something something"]
struct LabelOnNonSpan {
    #[label = "See here"]
    //~^ ERROR The `#[label = ...]` attribute can only be applied to fields of type Span
    id: u32,
}

#[derive(AsSessionError)]
#[code = "E0123"]
#[error = "Something something else"]
struct OptionsInErrors {
    #[label = "Label message"]
    label: Option<Span>,
    #[suggestion(message = "suggestion message")]
    opt_sugg: Option<(Span, Applicability)>,
}

#[derive(AsSessionError)]
#[code = "E0456"]
struct MoveOutOfBorrowError<'tcx> {
    name: Ident,
    ty: Ty<'tcx>,
    #[error = "cannot move {ty} out of borrow"]
    #[label = "cannot move out of borrow"]
    span: Span,
    #[label = "`{ty}` first borrowed here"]
    other_span: Span,
    #[suggestion(message = "consider cloning here", code = "{name}.clone()")]
    opt_sugg: Option<(Span, Applicability)>,
}
