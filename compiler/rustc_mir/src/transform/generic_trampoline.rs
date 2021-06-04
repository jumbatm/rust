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

use crate::dataflow::fmt::DebugWithContext;
use crate::dataflow::impls::MaybeLiveLocals;
use crate::dataflow::Analysis;
use crate::dataflow::Forward;
use crate::dataflow::JoinSemiLattice;
use crate::{
    dataflow::{AnalysisDomain, ResultsVisitor},
    transform::MirPass,
};

use rustc_data_structures::fx::FxIndexMap;
use rustc_index::bit_set::BitSet;
use rustc_middle::mir::traversal::preorder;
use rustc_middle::mir::BasicBlock;
use rustc_middle::mir::{self, Body, HasLocalDecls, Location, Statement};
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::TypeFlags;

pub struct GenericTrampoliner;

impl MirPass<'tcx> for GenericTrampoliner {
    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
        // At every program point, we only want to consider every live local. Unlike a lot of other
        // use cases, we don't need to consider a local live if a reference to it is live, because
        // when we synthesise the impl function, we can just pass the live reference in instead.
        let liveness_results =
            MaybeLiveLocals { drop_is_use: false }.into_engine(tcx, body).iterate_to_fixpoint();
        let mut annotator = AnnotateGenericStatements::new(body);
        liveness_results.visit_with(body, body.basic_blocks().indices(), &mut annotator);
        // We can now run a forward analysis which propagates "may be generic" down the CFG.
        let may_be_generic_results =
            GenericMayBeInScope::new(annotator).into_engine(tcx, body).iterate_to_fixpoint();
        // Take these results and collect them into the last point that's generic:
        let mut collector = CollectLastNonGenericPoint::new();
        may_be_generic_results.visit_with(
            body,
            preorder(body).map(|(bb, _bb_data)| bb),
            &mut collector,
        );
        // And there we have it -- the location of the last statement that is generic. All
        // successors to this statement are non-generic, and can be split into the impl function.
        let last_generic_location = collector.into_last_generic_point();
        debug!("Location of last generic point: {:#?}", last_generic_location);
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
    // Maps (BB -> statement index)
    block_map: FxIndexMap<BasicBlock, BitSet<usize>>,
}

impl AnnotateGenericStatements<'body, 'tcx> {
    fn new(body: &'body Body<'tcx>) -> Self {
        // Count the number of statements in this body.
        // FIXME: Replace with map (BasicBlock -> StatementIndex). We could just store, for
        // each basic block, where in the basic block the last statement with a live generic
        // is.
        Self { body, block_map: FxIndexMap::default() }
    }

    fn has_live_generic(&self, location: &Location) -> bool {
        debug_assert!(self.block_map.contains_key(&location.block));
        self.block_map[&location.block].contains(location.statement_index)
    }

    fn mark_has_live_generic(&mut self, location: &Location) {
        debug_assert!(self.block_map.contains_key(&location.block));
        self.block_map[&location.block].insert(location.statement_index);
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
            self.mark_has_live_generic(&location);
        } else {
            // All live variables are fully concrete. This is a pinch point.
            debug!("This is a pinch point!");
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

    fn visit_block_end(
        &mut self,
        _state: &Self::FlowState,
        block_data: &'mir mir::BasicBlockData<'tcx>,
        block: BasicBlock,
    ) {
        self.block_map
            .insert(block, BitSet::new_empty(block_data.statements.len()+/*terminator:*/1));
    }
}

/// An analysis which detects if any locals dependent on a generic parameter *may* be in scope.
pub struct GenericMayBeInScope<'body, 'tcx> {
    annotations: AnnotateGenericStatements<'body, 'tcx>,
}

impl GenericMayBeInScope<'body, 'tcx> {
    fn new(annotations: AnnotateGenericStatements<'body, 'tcx>) -> Self {
        Self { annotations }
    }

    fn genericness(&self, location: &Location) -> Genericness {
        if self.annotations.has_live_generic(location) {
            Genericness::Maybe
        } else {
            Genericness::No
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Genericness {
    Maybe,
    No,
}

impl JoinSemiLattice for Genericness {
    fn join(&mut self, other: &Self) -> bool {
        if self != other {
            *self = Genericness::Maybe;
            true
        } else {
            false
        }
    }
}

impl<C> DebugWithContext<C> for Genericness {}

impl AnalysisDomain<'tcx> for GenericMayBeInScope<'body, 'tcx> {
    type Domain = Genericness;
    type Direction = Forward;

    const NAME: &'static str = "genericness";

    fn bottom_value(&self, _body: &mir::Body<'tcx>) -> Self::Domain {
        Genericness::No
    }

    fn initialize_start_block(&self, _body: &mir::Body<'tcx>, _state: &mut Self::Domain) {
        // Nothing
    }
}

impl Analysis<'tcx> for GenericMayBeInScope<'body, 'tcx> {
    fn apply_statement_effect(
        &self,
        state: &mut Self::Domain,
        _statement: &mir::Statement<'tcx>,
        location: Location,
    ) {
        *state = self.genericness(&location);
    }

    fn apply_before_terminator_effect(
        &self,
        state: &mut Self::Domain,
        _terminator: &mir::Terminator<'tcx>,
        _location: Location,
    ) {
        *state = Genericness::No;
    }

    fn apply_terminator_effect(
        &self,
        state: &mut Self::Domain,
        _terminator: &mir::Terminator<'tcx>,
        location: Location,
    ) {
        *state = self.genericness(&location);
    }

    fn apply_call_return_effect(
        &self,
        state: &mut Self::Domain,
        _block: BasicBlock,
        _func: &mir::Operand<'tcx>,
        _args: &[mir::Operand<'tcx>],
        _return_place: mir::Place<'tcx>,
    ) {
        *state = Genericness::No;
    }
}

struct CollectLastNonGenericPoint {
    // Points to the last location that is a generic statement. Any successors to the statement at that
    // location must be non-generic.
    // None at start, indicating the exit node.
    last_generic_point: Option<Location>,
}

impl CollectLastNonGenericPoint {
    fn new() -> Self {
        Self { last_generic_point: None }
    }
    fn check(
        &mut self,
        state: &<Self as ResultsVisitor<'mir, 'tcx>>::FlowState,
        location: Location,
    ) {
        if let Genericness::Maybe = state {
            self.last_generic_point = Some(location);
        }
    }
    fn into_last_generic_point(self) -> Option<Location> {
        self.last_generic_point
    }
}

impl ResultsVisitor<'mir, 'tcx> for CollectLastNonGenericPoint {
    type FlowState = <GenericMayBeInScope<'mir, 'tcx> as AnalysisDomain<'tcx>>::Domain;

    fn visit_statement_after_primary_effect(
        &mut self,
        state: &Self::FlowState,
        _statement: &'mir mir::Statement<'tcx>,
        location: Location,
    ) {
        self.check(state, location);
    }

    fn visit_terminator_after_primary_effect(
        &mut self,
        state: &Self::FlowState,
        _terminator: &'mir mir::Terminator<'tcx>,
        location: Location,
    ) {
        self.check(state, location);
    }
}
