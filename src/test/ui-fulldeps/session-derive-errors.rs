// check-fail
// Tests error conditions for specifying diagnostics using #[derive(SessionDiagnostic)]

#![feature(rustc_private)]
#![crate_type = "lib"]

extern crate rustc_span;
use rustc_span::Span;
use rustc_span::symbol::Ident;

extern crate rustc_macros;
use rustc_macros::SessionDiagnostic;

extern crate rustc_middle;
use rustc_middle::ty::Ty;

extern crate rustc_errors;
use rustc_errors::Applicability;

extern crate rustc_session;

#[derive(SessionDiagnostic)]
#[error = "Hello, world!"]
#[code = "E0123"]
struct Hello {}

#[derive(SessionDiagnostic)]
#[error = "Hello, world!"]
#[code = "E0123"]
#[code = "E0456"] //~ ERROR `code` specified multiple times
struct ErrorSpecifiedTwice {}

#[derive(SessionDiagnostic)]
struct ErrorCodeNotProvided {} //~ ERROR `code` not specified

// FIXME: Uncomment when emitting lints is supported.
/*
#[derive(SessionDiagnostic)]
#[error = "Hello, world!"]
#[lint = "clashing_extern_declarations"]
#[lint = "improper_ctypes"] // FIXME: ERROR error code specified multiple times
struct LintSpecifiedTwice {}
*/

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct ErrorWithField {
    name: String,
    #[error = "This error has a field, and references {name}"]
    span: Span
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct ErrorWithNonexistentField {
    #[error = "This error has a field, and references {name}"]
    //~^ ERROR `name` doesn't refer to a field on this type
    span: Span
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
#[error = "Something something"]
struct LabelOnSpan {
    #[label = "See here"]
    sp: Span
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
#[error = "Something something"]
struct LabelOnNonSpan {
    #[label = "See here"]
    //~^ ERROR The `#[label = ...]` attribute can only be applied to fields of type Span
    id: u32,
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct Suggest {
    #[suggestion(message = "This is a suggestion", code = "This is the suggested code")]
    suggestion: (Span, Applicability),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithoutCode {
    #[suggestion(message = "This is a suggestion")]
    suggestion: (Span, Applicability),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithBadKey {
    #[suggestion(nonsense = "This is nonsense")]
    //~^ ERROR `nonsense` is not a valid key for `#[suggestion(...)]`
    suggestion: (Span, Applicability),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithShorthandMsg {
    #[suggestion(msg = "This is a suggestion")]
    //~^ ERROR `msg` is not a valid key for `#[suggestion(...)]`
    suggestion: (Span, Applicability),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithoutMsg {
    #[suggestion(code = "This is suggested code")]
    //~^ ERROR missing suggestion message
    suggestion: (Span, Applicability),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithTypesSwapped{
    #[suggestion(message = "This is a message", code = "This is suggested code")]
    suggestion: (Applicability, Span),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithWrongTypeApplicabilityOnly{
    #[suggestion(message = "This is a message", code = "This is suggested code")]
    //~^ ERROR wrong types for suggestion
    suggestion: Applicability,
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithWrongTypeSpanOnly{
    #[suggestion(message = "This is a message", code = "This is suggested code")]
    //~^ ERROR wrong types for suggestion
    suggestion: Span,
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithDuplicateSpanAndApplicability {
    #[suggestion(message = "This is a message", code = "This is suggested code")]
    //~^ ERROR type of field annotated with `#[suggestion(...)]` contains more than one Span
    suggestion: (Span, Span, Applicability),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct SuggestWithDuplicateApplicabilityAndSpan {
    #[suggestion(message = "This is a message", code = "This is suggested code")]
    //~^ ERROR type of field annotated with `#[suggestion(...)]` contains more than one
    suggestion: (Applicability, Applicability, Span),
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
struct WrongKindOfAnnotation {
    #[label("wrong kind of annotation for label")]
    //~^ ERROR invalid annotation list `#[label(...)]`
    z: Span,
}

#[derive(SessionDiagnostic)]
#[code = "E0123"]
#[error = "Something something else"]
struct OptionsInErrors {
    #[label = "Label message"]
    label: Option<Span>,
    #[suggestion(message = "suggestion message")]
    opt_sugg: Option<(Span, Applicability)>,
}

#[derive(SessionDiagnostic)]
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
