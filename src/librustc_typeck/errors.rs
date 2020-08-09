//! Errors emitted by typeck.
use rustc_macros::SessionDiagnostic;
use rustc_span::Span;

#[derive(SessionDiagnostic)]
#[error = "E0124"]
pub struct FieldAlreadyDeclared {
    pub field_name: String,
    #[message = "field `{field_name}` is already declared"]
    #[label = "field already declared"]
    pub span: Span,
    #[label = "`{field_name}` first declared here"]
    pub prev_span: Span,
}
