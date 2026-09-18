#![doc = include_str!("../README.md")]

mod batch;
mod explain;
mod model;
mod repair;
mod state;

pub use batch::{CancellationToken, CompiledProblem, SolveOptions, compile};
pub use explain::{
    Analysis, AssignmentChange, Bottleneck, ChangeAspects, ChangeEvaluation, ChangeRequest,
    MoveEvaluation, SolutionComparison, Suggestion, compare,
};
pub use model::{
    AbortReason, AcademicPeriod, Activity, ActivityId, ActivityRelation,
    ActivityRelationConstraint, Assignment, BreakTemplate, BucketLoadPattern, BucketWindow,
    CompileError, Conflict, ConflictSeverity, DEFAULT_CAPACITY_DIMENSION, DayTemplate, EntityRef,
    GroupMember, GroupMembership, MaximumDailyLoad, MinimumBreak, Participant,
    ParticipantChoiceGroup, ParticipantConflictPolicy, ParticipantGroup, ParticipantGroupId,
    ParticipantId, ParticipantPool, ParticipantPoolId, ParticipantRequirement, ProposedActivity,
    Recurrence, Resource, ResourceId, ResourcePool, ResourcePoolId, ResourceRequirement,
    ScheduleTemplate, SchedulingProblem, Score, ScoreComponent, ScoreLevel, ScoreRule,
    ScoreRuleKind, SlotTemplate, SoftGoal, SoftGoalKind, Solution, SolutionCheck, SolveResult,
    SolveStatistics, SolveStatus, TimeWindow, bucket_load_blocks, bucket_load_pattern,
    bucket_windows, matches_bucket_load_pattern,
};
pub use repair::{RepairOptions, RepairOutcome, RepairResult};
pub use state::SchedulingState;
