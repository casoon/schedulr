use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const DEFAULT_CAPACITY_DIMENSION: &str = "units";

macro_rules! identifier {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

identifier!(ResourceId, "r");
identifier!(ParticipantId, "p");
identifier!(ActivityId, "a");
identifier!(ParticipantGroupId, "pg");
identifier!(ResourcePoolId, "rp");
identifier!(ParticipantPoolId, "pp");

/// Half-open integer time interval `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeWindow {
    pub start: i64,
    pub end: i64,
}

impl TimeWindow {
    pub const fn new(start: i64, end: i64) -> Self {
        Self { start, end }
    }

    pub const fn is_valid(self) -> bool {
        self.start < self.end
    }

    pub fn duration(self) -> Option<u64> {
        u64::try_from(self.end.checked_sub(self.start)?).ok()
    }
}

/// Capacity-constrained entity consumed by activities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    id: ResourceId,
    name: String,
    capacities: BTreeMap<String, u32>,
    attributes: BTreeMap<String, String>,
    resource_type: String,
    features: BTreeSet<String>,
}

impl Resource {
    pub fn new(id: ResourceId, name: impl Into<String>, capacity: u32) -> Self {
        let capacities = BTreeMap::from([(DEFAULT_CAPACITY_DIMENSION.to_string(), capacity)]);
        Self {
            id,
            name: name.into(),
            capacities,
            attributes: BTreeMap::new(),
            resource_type: "resource".to_string(),
            features: BTreeSet::new(),
        }
    }

    pub fn with_capacity_dimension(mut self, dimension: impl Into<String>, capacity: u32) -> Self {
        self.capacities.insert(dimension.into(), capacity);
        self
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn with_type(mut self, resource_type: impl Into<String>) -> Self {
        self.resource_type = resource_type.into();
        self
    }

    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.features.insert(feature.into());
        self
    }

    pub const fn id(&self) -> ResourceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn capacity(&self) -> u32 {
        self.capacities[DEFAULT_CAPACITY_DIMENSION]
    }

    pub fn capacities(&self) -> &BTreeMap<String, u32> {
        &self.capacities
    }

    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    pub fn features(&self) -> &BTreeSet<String> {
        &self.features
    }
}

/// Named set of interchangeable resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePool {
    pub id: ResourcePoolId,
    pub name: String,
    pub resources: Vec<ResourceId>,
}

impl ResourcePool {
    pub fn new(
        id: ResourcePoolId,
        name: impl Into<String>,
        resources: impl IntoIterator<Item = ResourceId>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            resources: resources.into_iter().collect(),
        }
    }
}

/// Person or group whose simultaneous activities can be detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    id: ParticipantId,
    name: String,
    attributes: BTreeMap<String, String>,
}

/// Domain-neutral participant group. Memberships are stored separately so groups can overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantGroup {
    pub id: ParticipantGroupId,
    pub name: String,
}

/// Named set of interchangeable participants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantPool {
    pub id: ParticipantPoolId,
    pub name: String,
    pub participants: Vec<ParticipantId>,
}

impl ParticipantPool {
    pub fn new(
        id: ParticipantPoolId,
        name: impl Into<String>,
        participants: impl IntoIterator<Item = ParticipantId>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            participants: participants.into_iter().collect(),
        }
    }
}

impl ParticipantGroup {
    pub fn new(id: ParticipantGroupId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMember {
    Participant(ParticipantId),
    Group(ParticipantGroupId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupMembership {
    pub group: ParticipantGroupId,
    pub member: GroupMember,
}

impl GroupMembership {
    pub const fn participant(group: ParticipantGroupId, participant: ParticipantId) -> Self {
        Self {
            group,
            member: GroupMember::Participant(participant),
        }
    }

    pub const fn subgroup(group: ParticipantGroupId, subgroup: ParticipantGroupId) -> Self {
        Self {
            group,
            member: GroupMember::Group(subgroup),
        }
    }
}

impl Participant {
    pub fn new(id: ParticipantId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            attributes: BTreeMap::new(),
        }
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub const fn id(&self) -> ParticipantId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }
}

/// Exact capacity demand on a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRequirement {
    resource: Option<ResourceId>,
    resource_type: Option<String>,
    pool: Option<ResourcePoolId>,
    candidates: BTreeSet<ResourceId>,
    required_features: BTreeSet<String>,
    minimum_capacity: u32,
    dimension: String,
    units: u32,
    attributes: BTreeMap<String, String>,
}

impl ResourceRequirement {
    pub fn new(resource: ResourceId, units: u32) -> Self {
        Self {
            resource: Some(resource),
            resource_type: None,
            pool: None,
            candidates: BTreeSet::new(),
            required_features: BTreeSet::new(),
            minimum_capacity: units,
            dimension: DEFAULT_CAPACITY_DIMENSION.to_string(),
            units,
            attributes: BTreeMap::new(),
        }
    }

    pub fn for_dimension(resource: ResourceId, dimension: impl Into<String>, units: u32) -> Self {
        Self {
            resource: Some(resource),
            resource_type: None,
            pool: None,
            candidates: BTreeSet::new(),
            required_features: BTreeSet::new(),
            minimum_capacity: units,
            dimension: dimension.into(),
            units,
            attributes: BTreeMap::new(),
        }
    }

    /// Declares a requirement resolved by type/capacity/features instead of a fixed resource.
    pub fn matching(resource_type: impl Into<String>, units: u32) -> Self {
        Self {
            resource: None,
            resource_type: Some(resource_type.into()),
            pool: None,
            candidates: BTreeSet::new(),
            required_features: BTreeSet::new(),
            minimum_capacity: units,
            dimension: DEFAULT_CAPACITY_DIMENSION.to_string(),
            units,
            attributes: BTreeMap::new(),
        }
    }

    pub fn from_pool(pool: ResourcePoolId, units: u32) -> Self {
        let mut requirement = Self::matching("resource", units);
        requirement.resource_type = None;
        requirement.pool = Some(pool);
        requirement
    }

    pub fn with_candidate(mut self, resource: ResourceId) -> Self {
        self.candidates.insert(resource);
        self
    }

    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.required_features.insert(feature.into());
        self
    }

    pub fn with_minimum_capacity(mut self, capacity: u32) -> Self {
        self.minimum_capacity = capacity;
        self
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub const fn resource(&self) -> ResourceId {
        self.resource
            .expect("resource() is only available for exact requirements")
    }

    pub const fn exact_resource(&self) -> Option<ResourceId> {
        self.resource
    }

    pub fn resource_type(&self) -> Option<&str> {
        self.resource_type.as_deref()
    }

    pub const fn pool(&self) -> Option<ResourcePoolId> {
        self.pool
    }

    pub fn candidates(&self) -> &BTreeSet<ResourceId> {
        &self.candidates
    }

    pub fn required_features(&self) -> &BTreeSet<String> {
        &self.required_features
    }

    pub const fn minimum_capacity(&self) -> u32 {
        self.minimum_capacity
    }

    pub fn dimension(&self) -> &str {
        &self.dimension
    }

    pub const fn units(&self) -> u32 {
        self.units
    }

    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }
}

/// One participant chosen from an exact id, a named pool, or an explicit candidate set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantRequirement {
    participant: Option<ParticipantId>,
    pool: Option<ParticipantPoolId>,
    candidates: BTreeSet<ParticipantId>,
}

impl ParticipantRequirement {
    pub fn new(participant: ParticipantId) -> Self {
        Self {
            participant: Some(participant),
            pool: None,
            candidates: BTreeSet::new(),
        }
    }

    pub fn matching() -> Self {
        Self {
            participant: None,
            pool: None,
            candidates: BTreeSet::new(),
        }
    }

    pub fn from_pool(pool: ParticipantPoolId) -> Self {
        Self {
            participant: None,
            pool: Some(pool),
            candidates: BTreeSet::new(),
        }
    }

    pub fn with_candidate(mut self, participant: ParticipantId) -> Self {
        self.candidates.insert(participant);
        self
    }

    pub const fn exact_participant(&self) -> Option<ParticipantId> {
        self.participant
    }

    pub const fn pool(&self) -> Option<ParticipantPoolId> {
        self.pool
    }

    pub fn candidates(&self) -> &BTreeSet<ParticipantId> {
        &self.candidates
    }
}

/// Scheduling demand independent of any concrete solution assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    id: ActivityId,
    name: String,
    allowed_window: TimeWindow,
    duration: u64,
    participants: Vec<ParticipantId>,
    participant_groups: Vec<ParticipantGroupId>,
    participant_requirements: Vec<ParticipantRequirement>,
    requirements: Vec<ResourceRequirement>,
}

impl Activity {
    pub fn new(
        id: ActivityId,
        name: impl Into<String>,
        allowed_window: TimeWindow,
        duration: u64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            allowed_window,
            duration,
            participants: Vec::new(),
            participant_groups: Vec::new(),
            participant_requirements: Vec::new(),
            requirements: Vec::new(),
        }
    }

    pub fn with_participant(mut self, participant: ParticipantId) -> Self {
        self.participants.push(participant);
        self
    }

    pub fn with_requirement(mut self, requirement: ResourceRequirement) -> Self {
        self.requirements.push(requirement);
        self
    }

    pub fn with_participant_group(mut self, group: ParticipantGroupId) -> Self {
        self.participant_groups.push(group);
        self
    }

    pub fn with_participant_requirement(mut self, requirement: ParticipantRequirement) -> Self {
        self.participant_requirements.push(requirement);
        self
    }

    pub const fn id(&self) -> ActivityId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn allowed_window(&self) -> TimeWindow {
        self.allowed_window
    }

    pub const fn duration(&self) -> u64 {
        self.duration
    }

    pub fn participants(&self) -> &[ParticipantId] {
        &self.participants
    }

    pub fn participant_groups(&self) -> &[ParticipantGroupId] {
        &self.participant_groups
    }

    pub fn participant_requirements(&self) -> &[ParticipantRequirement] {
        &self.participant_requirements
    }

    pub fn requirements(&self) -> &[ResourceRequirement] {
        &self.requirements
    }
}

/// Concrete placement of one activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub activity: ActivityId,
    pub window: TimeWindow,
    pub resources: Vec<ResourceId>,
    pub participants: Vec<ParticipantId>,
}

impl Assignment {
    pub const fn new(activity: ActivityId, window: TimeWindow) -> Self {
        Self {
            activity,
            window,
            resources: Vec::new(),
            participants: Vec::new(),
        }
    }

    pub fn with_resource(mut self, resource: ResourceId) -> Self {
        self.resources.push(resource);
        self
    }

    pub fn with_participant(mut self, participant: ParticipantId) -> Self {
        self.participants.push(participant);
        self
    }
}

/// Public score independent of the underlying solver representation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score {
    pub hard: i64,
    pub strong: i64,
    pub medium: i64,
    pub weak: i64,
    pub soft: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScoreLevel {
    Strong,
    Medium,
    Weak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreComponent {
    pub category: String,
    pub level: ScoreLevel,
    pub value: i64,
    pub activity: Option<ActivityId>,
}

/// A solved set of activity placements and its aggregate score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub assignments: Vec<Assignment>,
    pub score: Score,
    pub score_components: Vec<ScoreComponent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSeverity {
    Blocking,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityRef {
    Resource(ResourceId),
    Participant(ParticipantId),
    Activity(ActivityId),
}

/// Structured scheduling conflict suitable for direct display or app-side localization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub severity: ConflictSeverity,
    pub constraint_name: String,
    pub involved: Vec<ActivityId>,
    pub entity: Option<EntityRef>,
    pub message: String,
}

/// Exact single-activity change checked against a [`crate::SchedulingState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedActivity {
    name: String,
    window: TimeWindow,
    participants: Vec<ParticipantId>,
    requirements: Vec<ResourceRequirement>,
    excluding: Option<ActivityId>,
}

/// One reusable slot inside a periodic schedule template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotTemplate {
    pub name: String,
    pub offset: i64,
    pub duration: u64,
}

impl SlotTemplate {
    pub fn new(name: impl Into<String>, offset: i64, duration: u64) -> Self {
        Self {
            name: name.into(),
            offset,
            duration,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakTemplate {
    pub name: String,
    pub window: TimeWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayTemplate {
    pub day_offset: i64,
    pub slots: Vec<SlotTemplate>,
    pub breaks: Vec<BreakTemplate>,
}

impl DayTemplate {
    pub fn new(day_offset: i64) -> Self {
        Self {
            day_offset,
            slots: Vec::new(),
            breaks: Vec::new(),
        }
    }

    pub fn with_slot(mut self, slot: SlotTemplate) -> Self {
        self.slots.push(slot);
        self
    }

    pub fn with_break(mut self, break_template: BreakTemplate) -> Self {
        self.breaks.push(break_template);
        self
    }
}

/// Periodic slot model (one week, A/B weeks, or block cycle) plus absolute exceptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleTemplate {
    pub cycle_length: i64,
    pub days: Vec<DayTemplate>,
    pub unavailable_ranges: Vec<(i64, i64)>,
}

impl ScheduleTemplate {
    pub fn new(cycle_length: i64) -> Self {
        Self {
            cycle_length,
            days: Vec::new(),
            unavailable_ranges: Vec::new(),
        }
    }

    pub fn with_day(mut self, day: DayTemplate) -> Self {
        self.days.push(day);
        self
    }

    pub fn with_unavailable_range(mut self, start: i64, end: i64) -> Self {
        self.unavailable_ranges.push((start, end));
        self
    }

    pub fn allowed_starts(&self) -> BTreeSet<i64> {
        self.allowed_starts_for(0)
    }

    pub fn allowed_starts_for(&self, duration: u64) -> BTreeSet<i64> {
        self.days
            .iter()
            .flat_map(|day| {
                day.slots
                    .iter()
                    .filter(move |slot| slot.duration >= duration)
                    .map(move |slot| day.day_offset.saturating_add(slot.offset))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcademicPeriod {
    pub window: TimeWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoreRuleKind {
    PreferWindow(TimeWindow),
    KeepStart(i64),
}

/// Named, inspectable scoring rule. Components are attached to each produced solution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreRule {
    pub category: String,
    pub level: ScoreLevel,
    pub activity: ActivityId,
    pub weight: i64,
    pub kind: ScoreRuleKind,
}

impl ScoreRule {
    pub fn prefer_window(
        category: impl Into<String>,
        level: ScoreLevel,
        activity: ActivityId,
        window: TimeWindow,
        weight: i64,
    ) -> Self {
        Self {
            category: category.into(),
            level,
            activity,
            weight,
            kind: ScoreRuleKind::PreferWindow(window),
        }
    }
}

impl ProposedActivity {
    pub fn new(name: impl Into<String>, window: TimeWindow) -> Self {
        Self {
            name: name.into(),
            window,
            participants: Vec::new(),
            requirements: Vec::new(),
            excluding: None,
        }
    }

    pub fn with_participant(mut self, participant: ParticipantId) -> Self {
        self.participants.push(participant);
        self
    }

    pub fn with_requirement(mut self, requirement: ResourceRequirement) -> Self {
        self.requirements.push(requirement);
        self
    }

    pub const fn excluding(mut self, activity: ActivityId) -> Self {
        self.excluding = Some(activity);
        self
    }

    pub fn add_participant(&mut self, participant: ParticipantId) {
        self.participants.push(participant);
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn window(&self) -> TimeWindow {
        self.window
    }

    pub fn participants(&self) -> &[ParticipantId] {
        &self.participants
    }

    pub fn requirements(&self) -> &[ResourceRequirement] {
        &self.requirements
    }

    pub const fn excluded_activity(&self) -> Option<ActivityId> {
        self.excluding
    }
}

/// In-memory batch scheduling input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulingProblem {
    pub resources: Vec<Resource>,
    pub participants: Vec<Participant>,
    pub activities: Vec<Activity>,
    pub resource_pools: Vec<ResourcePool>,
    pub participant_pools: Vec<ParticipantPool>,
    pub participant_groups: Vec<ParticipantGroup>,
    pub group_memberships: Vec<GroupMembership>,
    pub academic_period: Option<AcademicPeriod>,
    pub schedule_template: Option<ScheduleTemplate>,
    pub score_rules: Vec<ScoreRule>,
}

impl SchedulingProblem {
    pub fn new(
        resources: Vec<Resource>,
        participants: Vec<Participant>,
        activities: Vec<Activity>,
    ) -> Self {
        Self {
            resources,
            participants,
            activities,
            resource_pools: Vec::new(),
            participant_pools: Vec::new(),
            participant_groups: Vec::new(),
            group_memberships: Vec::new(),
            academic_period: None,
            schedule_template: None,
            score_rules: Vec::new(),
        }
    }

    pub fn with_resource_pool(mut self, pool: ResourcePool) -> Self {
        self.resource_pools.push(pool);
        self
    }

    pub fn with_participant_pool(mut self, pool: ParticipantPool) -> Self {
        self.participant_pools.push(pool);
        self
    }

    pub fn with_participant_group(mut self, group: ParticipantGroup) -> Self {
        self.participant_groups.push(group);
        self
    }

    pub fn with_group_membership(mut self, membership: GroupMembership) -> Self {
        self.group_memberships.push(membership);
        self
    }

    pub fn with_calendar(
        mut self,
        academic_period: AcademicPeriod,
        schedule_template: ScheduleTemplate,
    ) -> Self {
        self.academic_period = Some(academic_period);
        self.schedule_template = Some(schedule_template);
        self
    }

    pub fn with_score_rule(mut self, rule: ScoreRule) -> Self {
        self.score_rules.push(rule);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    messages: Vec<String>,
}

impl CompileError {
    pub(crate) fn new(messages: Vec<String>) -> Self {
        Self { messages }
    }

    pub fn messages(&self) -> &[String] {
        &self.messages
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.messages.join("; "))
    }
}

impl std::error::Error for CompileError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveStatus {
    Feasible,
    Infeasible,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolveResult {
    pub status: SolveStatus,
    pub solution: Option<Solution>,
    pub statistics: SolveStatistics,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SolveStatistics {
    pub nodes_expanded: u64,
    pub elapsed_millis: u128,
    pub optimal: bool,
}
