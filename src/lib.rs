#![doc = include_str!("../README.md")]

mod batch;
mod explain;
mod model;
mod repair;
mod state;

pub use batch::{CompiledProblem, compile};
pub use explain::{
    Analysis, AssignmentChange, Bottleneck, MoveEvaluation, SolutionComparison, Suggestion, compare,
};
pub use model::{
    AcademicPeriod, Activity, ActivityId, Assignment, BreakTemplate, CompileError, Conflict,
    ConflictSeverity, DEFAULT_CAPACITY_DIMENSION, DayTemplate, EntityRef, GroupMember,
    GroupMembership, Participant, ParticipantGroup, ParticipantGroupId, ParticipantId,
    ParticipantPool, ParticipantPoolId, ParticipantRequirement, ProposedActivity, Resource,
    ResourceId, ResourcePool, ResourcePoolId, ResourceRequirement, ScheduleTemplate,
    SchedulingProblem, Score, ScoreComponent, ScoreLevel, ScoreRule, ScoreRuleKind, SlotTemplate,
    Solution, SolveResult, SolveStatistics, SolveStatus, TimeWindow,
};
pub use repair::RepairOptions;
pub use state::SchedulingState;
