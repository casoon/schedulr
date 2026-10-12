#![doc = include_str!("../README.md")]

mod active_buckets;
mod batch;
mod bucket_rules;
mod construct;
mod explain;
mod model;
mod pairwise_distance;
mod repair;
mod state;

pub use batch::{CancellationToken, CompiledProblem, SolveOptions, SolveStrategy, compile};
pub use explain::{
    Analysis, AssignmentChange, Bottleneck, ChangeAspects, ChangeEvaluation, ChangeRequest,
    MoveEvaluation, SolutionComparison, Suggestion, compare,
};
pub use model::{
    AbortReason, AcademicPeriod, Activity, ActivityGoal, ActivityGoalKind, ActivityId,
    ActivityRelation, ActivityRelationConstraint, Assignment, BreakTemplate, BucketLoadPattern,
    BucketWindow, CompileError, ConcurrentFreeGoal, Conflict, ConflictSeverity,
    ConsecutiveRunLimit, DEFAULT_CAPACITY_DIMENSION, DayRule, DayRuleKind, DayTemplate, EntityRef,
    GroupMember, GroupMembership, IdleBounds, IsolatedBlockGoal, JointPresenceGoal,
    MaximumActiveBuckets, MaximumBucketStarts, MaximumDailyLoad, MaximumDistinctPerBucket,
    MinimumBreak, Participant, ParticipantChoiceGroup, ParticipantConflictPolicy, ParticipantGroup,
    ParticipantGroupId, ParticipantId, ParticipantPool, ParticipantPoolId, ParticipantRequirement,
    ProposedActivity, ProximityMode, Recurrence, Resource, ResourceCostGoal, ResourceId,
    ResourcePool, ResourcePoolId, ResourceRequirement, RoomChangeGoal, ScheduleTemplate,
    SchedulingProblem, Score, ScoreComponent, ScoreLevel, ScoreRule, ScoreRuleKind, SlotTemplate,
    SoftGoal, SoftGoalKind, SoftPenalty, Solution, SolutionCheck, SolveResult, SolveStatistics,
    SolveStatus, TimePreference, TimeWindow, bucket_load_blocks, bucket_load_pattern,
    bucket_windows, matches_bucket_load_pattern,
};
pub use repair::{RepairOptions, RepairOutcome, RepairResult};
pub use state::SchedulingState;
