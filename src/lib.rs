//! Domain-neutral scheduling and synchronous booking checks.
//!
//! The public API deliberately exposes no solver graph or variable identifiers.

mod batch;
mod model;
mod state;

pub use batch::{CompiledProblem, compile};
pub use model::{
    Activity, ActivityId, Assignment, CompileError, Conflict, ConflictSeverity,
    DEFAULT_CAPACITY_DIMENSION, EntityRef, Participant, ParticipantId, ProposedActivity, Resource,
    ResourceId, ResourceRequirement, SchedulingProblem, Score, Solution, SolveResult, SolveStatus,
    TimeWindow,
};
pub use state::SchedulingState;
