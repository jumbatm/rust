//! Consider you have this generic function `foo`:
//!
//! ```rust,ignore(pseudocode)
//! fn foo<X, Y, Z>(x: X, y: Y, z: Z) {
//!    let x: String = something_with_x(x);
//!    let y: String = something_with_y(y);
//!    let z: String = something_with_z(z);
//!    // ... lots more code ...
//! }
//! ```
//!
//! If `foo` is monomorphised as-is, you'd have to copy the entire function body for every single
//! set of concrete use of `foo`.
//!
//! However, we can observe that `foo` is, only generic up until `x`, `y` and `z` are all moved.
//! After that point, the rest of the function body could really just be replaced by a non-generic
//! function `foo_impl`:
//!
//! ```rust,ignore(pseudocode)
//! fn foo<X, Y, Z>(x: X, y: Y, z: Z) {
//!    let x: String = something_with_x(x);
//!    let y: String = something_with_y(y);
//!    let z: String = something_with_z(z);
//!    foo_impl(x, y, z)
//! }
//!
//! fn foo_impl(x: String, y: String, z: String) {
//!    // ... lots more code ...
//! }
//! ```
//!
//! Now, when `foo` gets monomorphised, we only have to monomorphise the start of `foo`, and they
//! can all call the same `foo_impl`. However many statements we can move into `foo_impl` is the
//! number of statements we save from having to instantiate for every monomorphisation of `foo`.
//!
//! That's what this transform achieves: It first detects where (if at all) there's a "pinch point"
//! where the function becomes non-generic. It then splits the function at that point, putting
//! everything after the pinch point into a different Body and replacing it with a call to a
//! non-generic impl function.
//!
//! For now, for a generic function to be eligible for this optimisation, there must be some
//! program point P after which all operations are non-generic. The generated impl function is
//! always called at the end of the trampoline, and contains all statements from the original
//! function from P up until its exit: [P, exit). It's technically possible to generalise the impl
//! function doesn't have to go to exit (ie, [P, P+n]), but that makes the analysis much more
//! complex, and it's not clear that would give any benefit in real codebases.

use crate::dataflow::impls::MaybeLiveLocals;
use crate::dataflow::Analysis;
use crate::{
    dataflow::{AnalysisDomain, ResultsVisitor},
    transform::MirPass,
};

use rustc_index::bit_set::BitSet;
use rustc_middle::mir::{self, Body, HasLocalDecls, Location, Statement};
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::TypeFlags;

pub struct GenericTrampoliner;

impl MirPass<'tcx> for GenericTrampoliner {
    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
        // At every program point, we only want to consider every live local. Unlike a lot of other
        // use cases, we don't need to consider a local live if a reference to it is live, because
        // when we synthesise the impl function, we can just pass the live reference in instead.
        let _liveness_results = MaybeLiveLocals { drop_is_use: false }
            .into_engine(tcx, body)
            .iterate_to_fixpoint()
            .visit_with(body, body.basic_blocks().indices(), &mut AnnotateGenericStatements::new(body));
    }
}

/// A visitor which, based on liveness results, annotates each statement with whether or not, at a
/// particular program point P, there are any live generic values.
///
/// We need this information for a forward analysis we perform later on. Seeking a cursor over
/// liveness results would be slow, because seeking in the wrong direction is a O(n^2) operation,
/// so we cache the information we need instead.
struct AnnotateGenericStatements<'body, 'tcx> {
    body: &'body Body<'tcx>,
    cache: BitSet<Location>,
}

impl AnnotateGenericStatements<'body, 'tcx> {
    fn new(body: &'body Body<'tcx>) -> Self {
        // Count the number of statements in this body.
        // FIXME: Replace with map (BasicBlock -> StatementIndex). We could just store, for
        // each basic block, where in the basic block the last statement with a live generic
        // is.
        let num_statements = body
            .basic_blocks()
            .iter()
            .map(|data| data.statements.len() + /*terminator: */ 1)
            .fold(0, |acc, elem| acc + elem);

        Self {
            body,
            cache: BitSet::new_empty(num_statements),
        }
    }
    fn has_live_generic(&self, location: &Location) -> bool {
        self.cache.contains(location)
    }
    fn mark_has_live_generic(&mut self, location: &Location) {
        self.cache.insert(location)
    }

    fn check_for_pinch_point(
        &mut self,
        state: &<Self as ResultsVisitor<'mir, 'tcx>>::FlowState,
        location: rustc_middle::mir::Location,
    ) {
        // FIXME(jumbatm): This can be made faster -- can just check the diff between before and
        // after the statement effect to get locals which changed to become live.
        let mut live_local_types =
            self.body.local_decls().iter_enumerated().filter_map(|(local, local_decl)| {
                if state.contains(local) {
                    debug!("Local in scope: {:?}:{:?}", &local, &local_decl.ty);
                    Some(local_decl.ty)
                } else {
                    None
                }
            });

        if let Some(generic_ty) = live_local_types
            .find(|ty| ty.flags().intersects(TypeFlags::HAS_TY_PARAM | TypeFlags::NEEDS_SUBST))
        {
            // Found a generic ty!
            debug!("Found a live generic ty: {:?}", generic_ty);
        } else {
            // All live variables are fully concrete. This is a pinch point.
            debug!("This is a pinch point!");
            self.mark_has_live_generic(&location);
        }
    }
}

impl ResultsVisitor<'mir, 'tcx> for AnnotateGenericStatements<'body, 'tcx> {
    type FlowState = <MaybeLiveLocals as AnalysisDomain<'tcx>>::Domain;

    fn visit_statement_after_primary_effect(
        &mut self,
        state: &Self::FlowState,
        statement: &'mir Statement<'tcx>,
        location: rustc_middle::mir::Location,
    ) {
        trace!(
            "visit_statement_after_primary_effect {:?}: {:?} -> {:?}",
            location,
            statement,
            state
        );
        self.check_for_pinch_point(state, location)
    }
    fn visit_terminator_after_primary_effect(
        &mut self,
        state: &Self::FlowState,
        terminator: &'mir mir::Terminator<'tcx>,
        location: Location,
    ) {
        trace!(
            "visit_terminator_after_primary_effect {:?}: {:?} -> {:?}",
            location,
            terminator,
            state
        );
        self.check_for_pinch_point(state, location)
    }
}
