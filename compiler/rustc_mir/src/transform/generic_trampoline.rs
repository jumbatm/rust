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

use rustc_data_structures::fx::FxIndexMap;
use rustc_index::bit_set::BitSet;
use rustc_middle::mir::traversal::postorder;
use rustc_middle::mir::traversal::reverse_postorder;
use rustc_middle::mir::{self, Body, HasLocalDecls, Location, Statement};
use rustc_middle::mir::{BasicBlock, BasicBlockData};
use rustc_middle::mir::{Terminator, TerminatorKind};
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
        liveness_results.visit_with(body, postorder(&body).map(|(bb, _)| bb), &mut annotator);
        let pinch_point = {
            // The first entry of `rpo` is the successor of every block.
            // NOTE: We're technically looking for the exit block -- a single block that every return
            // from the function eventually leads to (ie, a block which postdominates every other block
            // in the CFG). While MIR doesn't explicitly have an "exit block", we do have cleanup
            // blocks to run destructors, which is _hopefully_ close enough.
            // Take these results and collect them into the last point that's generic:
            let (rpo, exit_block) = {
                let mut rpo = reverse_postorder(body);
                let (block, bbd) = {
                    // NOTE: We don't want to consume `rpo`, so we can't use Iterator::last.
                    let mut last_elem = None;
                    while let Some(e) = rpo.next() {
                        last_elem = Some(e);
                    }
                    rpo.reset();
                    last_elem.expect("MIR body with no blocks")
                };
                (rpo, Location { block, statement_index: bbd.statements.len() + 1 })
            };
            debug!("Exit block is {:?}", &exit_block);
            let mut last_generic_point = None;
            let mut candidate_pinch_point = None;
            let locations = rpo.flat_map(|(bb, bb_data)| {
                (0..bb_data.statements.len() + 1)
                    .map(move |idx| Location { block: bb, statement_index: idx })
            });
            let dominators = body.dominators();
            for location in locations {
                // FIXME: There's an optimisation opportunity here. After finding a pinch point and
                // we've found the first successor that dominates the exit block, we can skip all
                // the way to that block's successor.
                if annotator.has_live_generic(&location) {
                    // We've found a new non-generic point.
                    debug!("Found a new generic point: {:?}", &location);
                    last_generic_point = Some(location);
                    // We need to look for the pinch point in this point's successors. Invalidate
                    // the previous candidate pinch point.
                    candidate_pinch_point = None;
                } else if last_generic_point.is_some() && candidate_pinch_point.is_none() {
                    // We've previously set the last non-generic point, and we're now searching for
                    // a pinch point. This `location` is a pinch point if it's after the last
                    // generic point (which we know is for certain, because we're traversing in
                    // rpo) and if it dominates the exit node (which we need to check now).
                    debug!(
                        "Checking candidate pinch point for {:?}: {:?}",
                        &last_generic_point, &location
                    );
                    if location.dominates(exit_block, &dominators) {
                        debug!("Yes, {:?} dominates {:?}", &location, &exit_block);
                        candidate_pinch_point = Some(location);
                    } else {
                        debug!("No, does not dominate");
                    }
                }
            }
            if let Some(pinch_point) = candidate_pinch_point {
                // The pinch point must be the first statement in the non-generic half of the
                // function.
                debug_assert!(!annotator.has_live_generic(&pinch_point));
            }
            candidate_pinch_point
        };
        debug!("Location of pinch point: {:#?}", pinch_point);
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
