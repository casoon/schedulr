#![doc = include_str!("../README.md")]

mod batch;
mod explain;
mod model;
mod repair;
mod state;

pub use batch::{CancellationToken, CompiledProblem, SolveOptions, compile};
pub use explain::{
    Analysis, AssignmentChange, Bottleneck, ChangeEvaluation, ChangeRequest, MoveEvaluation,
    SolutionComparison, Suggestion, compare,
};
pub use model::{
    AbortReason, AcademicPeriod, Activity, ActivityId, ActivityRelation,
    ActivityRelationConstraint, Assignment, BreakTemplate, BucketWindow, CompileError, Conflict,
    ConflictSeverity, DEFAULT_CAPACITY_DIMENSION, DayTemplate, EntityRef, GroupMember,
    GroupMembership, MaximumDailyLoad, MinimumBreak, Participant, ParticipantGroup,
    ParticipantGroupId, ParticipantId, ParticipantPool, ParticipantPoolId, ParticipantRequirement,
    ProposedActivity, Resource, ResourceId, ResourcePool, ResourcePoolId, ResourceRequirement,
    ScheduleTemplate, SchedulingProblem, Score, ScoreComponent, ScoreLevel, ScoreRule,
    ScoreRuleKind, SlotTemplate, Solution, SolveResult, SolveStatistics, SolveStatus, TimeWindow,
    bucket_windows,
};
pub use repair::RepairOptions;
pub use state::SchedulingState;
