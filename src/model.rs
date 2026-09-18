use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::RangeInclusive;

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

    /// The start slots an activity of `duration` may **not** use because it would occupy part of
    /// this window — the occupancy rule, not the "start inside the range" one.
    ///
    /// `[start, start + duration)` touching `[self.start, self.end)`: blocking the single hour 9
    /// forbids a start at 8 only for a two-hour activity, and forbids 9 for every activity. Because
    /// the window is half-open, a blocked house is `[s, s + 1)`, never `[s, s + 1]`.
    pub fn blocked_starts(self, duration: i64) -> RangeInclusive<i64> {
        self.start.saturating_sub(duration).saturating_add(1)..=self.end.saturating_sub(1)
    }

    /// `self.blocked_starts(duration).contains(&start)`, for callers that only need to ask.
    pub fn occupies(self, start: i64, duration: i64) -> bool {
        self.blocked_starts(duration).contains(&start)
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
    unavailable_ranges: Vec<(i64, i64)>,
    blocked_windows: Vec<TimeWindow>,
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
            unavailable_ranges: Vec::new(),
            blocked_windows: Vec::new(),
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

    /// Marks `[start, end]` (inclusive) as a time range this resource cannot be booked in,
    /// e.g. a gym only usable 08–16 (AllowedTime: forbid the hours outside that window).
    pub fn with_unavailable_range(mut self, start: i64, end: i64) -> Self {
        self.unavailable_ranges.push((start, end));
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

    pub fn unavailable_ranges(&self) -> &[(i64, i64)] {
        &self.unavailable_ranges
    }

    /// Blocks a whole window instead of a start: no activity may **occupy** any part of it, however
    /// long it is. Unlike [`Self::with_unavailable_range`] this is duration-aware — see
    /// [`TimeWindow::blocked_starts`].
    pub fn with_blocked_window(mut self, window: TimeWindow) -> Self {
        self.blocked_windows.push(window);
        self
    }

    pub fn blocked_windows(&self) -> &[TimeWindow] {
        &self.blocked_windows
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
    unavailable_ranges: Vec<(i64, i64)>,
    blocked_windows: Vec<TimeWindow>,
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
            unavailable_ranges: Vec::new(),
            blocked_windows: Vec::new(),
        }
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Marks `[start, end]` (inclusive) as a time range this participant is unavailable in,
    /// e.g. a teacher not available on Tuesdays (Availability).
    ///
    /// This blocks **starts** inside the range. To forbid occupying the range at all — "the person
    /// is not bookable in these hours", whatever the activity's length — use
    /// [`Self::with_blocked_window`] instead.
    pub fn with_unavailable_range(mut self, start: i64, end: i64) -> Self {
        self.unavailable_ranges.push((start, end));
        self
    }

    /// Blocks a whole window instead of a start: no activity this participant attends may
    /// **occupy** any part of it, however long it is. That is what a timetable means by "this
    /// person is not available in the 3rd period" — see [`TimeWindow::blocked_starts`].
    pub fn with_blocked_window(mut self, window: TimeWindow) -> Self {
        self.blocked_windows.push(window);
        self
    }

    pub const fn id(&self) -> ParticipantId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn unavailable_ranges(&self) -> &[(i64, i64)] {
        &self.unavailable_ranges
    }

    pub fn blocked_windows(&self) -> &[TimeWindow] {
        &self.blocked_windows
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

/// How a conflict that names a [`Participant`] is reported (plan 26, §1).
///
/// The core detects a participant collision either way — this only decides the verdict. The
/// default keeps the booking-desk semantics: a double-booked person is reported, but accepting
/// it stays the caller's decision. A domain in which a person physically cannot be in two
/// places — one teacher, two lessons — opts into [`Self::Blocking`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParticipantConflictPolicy {
    /// Report the collision as `Advisory`; the caller decides what to do with it.
    #[default]
    Advisory,
    /// Report the collision as `Blocking`, exactly like a resource collision.
    Blocking,
}

impl ParticipantConflictPolicy {
    /// The severity a `Participant`-named conflict is reported with under this policy.
    pub const fn severity(self) -> ConflictSeverity {
        match self {
            Self::Advisory => ConflictSeverity::Advisory,
            Self::Blocking => ConflictSeverity::Blocking,
        }
    }
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

/// The full verdict on one [`Solution`], obtained without mutating it (plan 31, E0.1).
///
/// Unlike the single-move [`crate::MoveEvaluation`] this checks every activity of the solution at
/// once. A partial solution is made explicit: every modeled activity without an assignment is
/// reported as a blocking `Unassigned` conflict, and an assignment naming an activity the problem
/// no longer contains is reported as a blocking `ActivityDomain` conflict — never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolutionCheck {
    pub is_feasible: bool,
    pub hard_violations: Vec<Conflict>,
    pub warnings: Vec<Conflict>,
    /// The absolute score of the checked solution (not a delta).
    pub score: Score,
    pub score_components: Vec<ScoreComponent>,
    pub explanations: Vec<String>,
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

/// A recurring break within a day (e.g. lunch, recess). `window` is
/// day-relative, like [`SlotTemplate::offset`]. No activity may be placed so
/// that it overlaps a break; see [`ScheduleTemplate::allowed_starts_for`].
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
        let duration = duration as i64;
        self.days
            .iter()
            .flat_map(|day| {
                day.slots
                    .iter()
                    .filter(move |slot| slot.duration as i64 >= duration)
                    .filter_map(move |slot| {
                        let start = day.day_offset.saturating_add(slot.offset);
                        let end = start.saturating_add(duration);
                        let blocked = day.breaks.iter().any(|break_template| {
                            let break_start =
                                day.day_offset.saturating_add(break_template.window.start);
                            let break_end =
                                day.day_offset.saturating_add(break_template.window.end);
                            start < break_end && break_start < end
                        });
                        (!blocked).then_some(start)
                    })
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

/// A hard temporal relation between two activities' start times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityRelation {
    /// `first` and `second` start at exactly the same time (SameTime).
    SameStart,
    /// `first` and `second` must not overlap in time (DifferentTime).
    NoOverlap,
    /// `second` starts exactly when `first` ends, no gap allowed (Consecutive).
    Consecutive,
    /// `first` ends at least `min_gap` time units before `second` starts (Precedence).
    Precedence { min_gap: i64 },
    /// `second` starts exactly `offset` time units after `first` starts:
    /// `second.start = first.start + offset`. A negative `offset` places `second` before `first`.
    ///
    /// Generalises [`Self::SameStart`] (which is `FixedOffset { offset: 0 }`) to a fixed, non-zero
    /// time lag between two activities — the primitive a caller needs to tie copies of a recurring
    /// pattern to their shifted positions.
    FixedOffset { offset: i64 },
}

/// Ties two activities together with an [`ActivityRelation`]. Unlike [`ScoreRule`], this is a
/// hard constraint: the solver never produces a solution that violates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityRelationConstraint {
    pub first: ActivityId,
    pub second: ActivityId,
    pub relation: ActivityRelation,
}

impl ActivityRelationConstraint {
    pub const fn new(first: ActivityId, second: ActivityId, relation: ActivityRelation) -> Self {
        Self {
            first,
            second,
            relation,
        }
    }
}

/// Blueprint for expanding one template [`Activity`] into several instances that repeat every
/// `step` time units, each on the same relative position (plan 28, "Engine (`schedulr`, generisch)").
///
/// The caller names the instances: `instances` is a list of `(k, id)` pairs where `k` is the
/// instance index — **not** the position in the vector — and `id` the [`ActivityId`] the instance
/// must be reported under. Ordering is irrelevant; the instance index is what carries meaning.
///
/// The template itself is only a plan and is **not** added as an activity by
/// [`SchedulingProblem::with_recurring_activity`]; the caller supplies an instance for every index
/// it wants, including any index whose window coincides with the template's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    /// Distance between two consecutive instances, in time units. Negative steps shift the
    /// higher-indexed instances *before* the lower-indexed ones.
    pub step: i64,
    /// `(k, id)` pairs: instance `k` is placed under `id` with the template's window shifted by
    /// `k * step`.
    pub instances: Vec<(u32, ActivityId)>,
}

/// Hard cap on the total occupied duration one entity may accumulate inside a single period
/// bucket (e.g. one day): "sum of occupied duration per resource/participant and period bucket <=
/// limit".
///
/// Buckets come from the [`ScheduleTemplate`] — one bucket per [`DayTemplate`], repeated over the
/// cycle; without a schedule template the whole modeled horizon counts as a single bucket. The
/// limit is applied separately to each entity's assignments: a resource counts the activities that
/// require it (weighted by the requirement's units), a participant counts the activities it
/// attends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaximumDailyLoad {
    /// Resource or participant the limit applies to.
    pub entity: EntityRef,
    /// Maximum occupied duration per bucket.
    pub limit: u64,
}

impl MaximumDailyLoad {
    pub const fn new(entity: EntityRef, limit: u64) -> Self {
        Self { entity, limit }
    }

    pub const fn for_resource(resource: ResourceId, limit: u64) -> Self {
        Self::new(EntityRef::Resource(resource), limit)
    }

    pub const fn for_participant(participant: ParticipantId, limit: u64) -> Self {
        Self::new(EntityRef::Participant(participant), limit)
    }
}

/// One bucket a load rule is checked against: a half-open window plus the bucket it belongs to
/// (plan 25, C1). The bucket index repeats across cycles — the same day of the cycle is the same
/// bucket in every cycle, which is what makes "per day" mean "per day of the cycle".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BucketWindow {
    pub bucket: usize,
    pub window: TimeWindow,
}

/// The buckets load rules are checked against, derived from the schedule template: each day
/// becomes a bucket, repeated once per cycle; without a valid template the whole horizon is one
/// bucket (plan 25, C1).
///
/// Public on purpose: a caller reasoning about teaching blocks ("is this course taught as a double
/// period plus singles?") must use the **same** bucket notion as the solver. A second, parallel
/// notion of "day" is exactly the divergence this avoids.
pub fn bucket_windows(
    template: Option<&ScheduleTemplate>,
    min_value: i64,
    max_value: i64,
) -> Vec<BucketWindow> {
    if min_value >= max_value {
        return Vec::new();
    }
    let Some(template) = template.filter(|template| template.cycle_length > 0) else {
        return vec![BucketWindow {
            bucket: 0,
            window: TimeWindow::new(min_value, max_value),
        }];
    };
    let mut offsets: Vec<i64> = template.days.iter().map(|day| day.day_offset).collect();
    offsets.sort_unstable();
    offsets.dedup();
    let Some(&first_offset) = offsets.first() else {
        return vec![BucketWindow {
            bucket: 0,
            window: TimeWindow::new(min_value, max_value),
        }];
    };
    let cycle = template.cycle_length;
    let mut ranges = Vec::new();
    for k in (min_value.div_euclid(cycle) - 1)..=(max_value.div_euclid(cycle) + 1) {
        for (bucket, &offset) in offsets.iter().enumerate() {
            let end_offset = offsets
                .get(bucket + 1)
                .copied()
                .unwrap_or(first_offset + cycle);
            let base = k.saturating_mul(cycle);
            ranges.push(BucketWindow {
                bucket,
                window: TimeWindow::new(
                    base.saturating_add(offset),
                    base.saturating_add(end_offset),
                ),
            });
        }
    }
    ranges
}

/// All activities in the group choose the **same** participant for their candidate requirement
/// (plan 33.1).
///
/// This is the engine primitive behind "one teacher per course": a course expanded into several
/// activities (its terms, and one instance per cycle week) must not be split across different
/// teachers. The group is domain-neutral — it only names activities whose participant
/// requirements are compiled to satisfy the equality per candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantChoiceGroup {
    /// The activities whose participant selection must agree.
    pub activities: Vec<ActivityId>,
}

impl ParticipantChoiceGroup {
    pub fn new(activities: Vec<ActivityId>) -> Self {
        Self { activities }
    }
}

/// Allowed shapes of a group of activities' teaching blocks, expressed over the schedule's own
/// buckets (plan 25, C1).
///
/// `allowed` holds multisets of **consecutive block durations**, order irrelevant: `[2, 1, 1]`
/// means "one double block and two single blocks", regardless of which buckets they land in.
/// The buckets themselves come from the [`ScheduleTemplate`] (see `load_bucket_ranges`), so
/// schedulr never learns what a "day" or "week" is — the caller supplies the context.
/// An empty `allowed` list means "no pattern chosen", and the rule simply does not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketLoadPattern {
    /// The activities this pattern constrains, e.g. all occurrences of one course.
    pub activities: Vec<ActivityId>,
    /// Allowed block multisets, e.g. `[[2, 1, 1], [1, 1, 1, 1]]`.
    pub allowed: Vec<Vec<u64>>,
}

impl BucketLoadPattern {
    pub fn new(activities: Vec<ActivityId>, allowed: Vec<Vec<u64>>) -> Self {
        Self {
            activities,
            allowed,
        }
    }

    /// Whether the rule applies at all — a pattern without activities or without an allowed
    /// shape must never turn into a violation.
    pub fn is_active(&self) -> bool {
        !self.activities.is_empty() && !self.allowed.is_empty()
    }
}

/// The consecutive block durations this group occupies **per bucket**, in bucket order
/// (plan 25, C1). Two touching or overlapping assignments form one block, which is what makes
/// "a double period" observable at all; every block is reported as an absolute end, so callers
/// can compare block lengths without re-deriving the arithmetic.
pub fn bucket_load_blocks(
    assignments: &[(ActivityId, TimeWindow)],
    buckets: &[TimeWindow],
) -> Vec<Vec<i64>> {
    let mut blocks_per_bucket = Vec::with_capacity(buckets.len());
    for bucket in buckets {
        let mut segments: Vec<(i64, i64)> = assignments
            .iter()
            .filter_map(|(_, window)| {
                let start = window.start.max(bucket.start);
                let end = window.end.min(bucket.end);
                (start < end).then_some((start, end))
            })
            .collect();
        segments.sort_unstable();

        let mut blocks: Vec<(i64, i64)> = Vec::new();
        for (start, end) in segments {
            match blocks.last_mut() {
                // Touching counts as one block: a double period occupies two adjacent slots.
                Some(block) if start <= block.1 => block.1 = block.1.max(end),
                _ => blocks.push((start, end)),
            }
        }
        blocks_per_bucket.push(blocks.into_iter().map(|(start, end)| end - start).collect());
    }
    blocks_per_bucket
}

/// The group's block durations across all buckets, largest first — the comparable form of a
/// teaching pattern. `[2, 1, 1]` and `[1, 2, 1]` produce the same result (plan 25, C1).
pub fn bucket_load_pattern(
    assignments: &[(ActivityId, TimeWindow)],
    buckets: &[TimeWindow],
) -> Vec<i64> {
    let mut durations: Vec<i64> = bucket_load_blocks(assignments, buckets)
        .into_iter()
        .flatten()
        .collect();
    durations.sort_unstable_by(|left, right| right.cmp(left));
    durations
}

/// Whether the group's teaching blocks match one of the allowed patterns (order irrelevant,
/// plan 25, C1). A pattern that is not active is never violated.
pub fn matches_bucket_load_pattern(
    pattern: &BucketLoadPattern,
    assignments: &[(ActivityId, TimeWindow)],
    buckets: &[TimeWindow],
) -> bool {
    if !pattern.is_active() {
        return true;
    }
    let observed = bucket_load_pattern(assignments, buckets);
    pattern.allowed.iter().any(|allowed| {
        let mut allowed: Vec<i64> = allowed.iter().map(|block| *block as i64).collect();
        allowed.sort_unstable_by(|left, right| right.cmp(left));
        allowed == observed
    })
}

/// Minimum distance between any two of an entity's assignments, e.g. a person-specific minimum
/// break between two of their activities.
///
/// Kept deliberately separate from a fixed [`BreakTemplate`] (which blocks a calendar window): this
/// constrains the relative placement of an entity's assignments. The distance is measured between
/// the assignments' start values, so to require a real free gap the caller includes the earlier
/// activity's duration in `min_distance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinimumBreak {
    /// Resource or participant the distance applies to.
    pub entity: EntityRef,
    /// Minimum distance between any two of the entity's assignment start values.
    pub min_distance: i64,
}

impl MinimumBreak {
    pub const fn new(entity: EntityRef, min_distance: i64) -> Self {
        Self {
            entity,
            min_distance,
        }
    }

    pub const fn for_resource(resource: ResourceId, min_distance: i64) -> Self {
        Self::new(EntityRef::Resource(resource), min_distance)
    }

    pub const fn for_participant(participant: ParticipantId, min_distance: i64) -> Self {
        Self::new(EntityRef::Participant(participant), min_distance)
    }
}

/// A problem-level soft preference, contrasted with the activity-scoped [`ScoreRule`].
///
/// Every goal contributes to its [`ScoreLevel`] tier with its `weight`, and its contribution is
/// reported in [`Solution::score_components`] under `category` (with `activity == None`, since the
/// goal is not owned by a single activity). This lets a caller display exactly the goals it
/// configured and how much each one currently costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftGoalKind {
    /// Minimise free time (unoccupied spans) between one participant's assignments inside each
    /// period bucket.
    MinimizeParticipantIdle,
    /// Minimise free time between the aggregate occupancy of one participant group inside each
    /// period bucket. Occupancy is the union of the assignments of all members of the group.
    MinimizeGroupIdle,
    /// Prefer a stable resource assignment: occurrences that share an activity name should use as
    /// few distinct resources as possible.
    RoomStability,
    /// Spread occurrences that share an activity name across the period buckets instead of
    /// clustering several of them in the same bucket.
    SpreadActivityOverDays,
}

/// One problem-level soft goal.
///
/// Unlike [`ScoreRule`] this is not tied to a single activity: it applies to the whole
/// [`SchedulingProblem`] and is added through [`SchedulingProblem::with_soft_goal`]. The penalty a
/// goal accumulates always reduces the soft score, so a larger `weight` makes the goal dominate
/// other goals at the same [`ScoreLevel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftGoal {
    /// Stable category shown in [`Solution::score_components`].
    pub category: String,
    /// Lexicographic level the contribution is scored at.
    pub level: ScoreLevel,
    /// Multiplier applied to the goal's accumulated penalty (non-negative by convention).
    pub weight: i64,
    /// Which global preference this goal expresses.
    pub kind: SoftGoalKind,
}

impl SoftGoal {
    pub fn new(
        category: impl Into<String>,
        level: ScoreLevel,
        weight: i64,
        kind: SoftGoalKind,
    ) -> Self {
        Self {
            category: category.into(),
            level,
            weight,
            kind,
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
    pub relations: Vec<ActivityRelationConstraint>,
    pub maximum_daily_loads: Vec<MaximumDailyLoad>,
    pub minimum_breaks: Vec<MinimumBreak>,
    /// Allowed block shapes for groups of activities (plan 25, C1).
    pub bucket_load_patterns: Vec<BucketLoadPattern>,
    /// Groups whose activities must select the same participant (plan 33.1).
    pub participant_choice_groups: Vec<ParticipantChoiceGroup>,
    pub soft_goals: Vec<SoftGoal>,
    /// How a collision between two activities of the same participant is reported (plan 26, §1).
    pub participant_conflict_policy: ParticipantConflictPolicy,
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
            relations: Vec::new(),
            maximum_daily_loads: Vec::new(),
            minimum_breaks: Vec::new(),
            bucket_load_patterns: Vec::new(),
            participant_choice_groups: Vec::new(),
            soft_goals: Vec::new(),
            participant_conflict_policy: ParticipantConflictPolicy::default(),
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

    /// Attaches the schedule template **without** turning on the periodic calendar, which
    /// [`Self::with_calendar`] does together with it.
    ///
    /// The template supplies the bucket structure every load rule is measured against
    /// (plan 25, C1) — without it the whole horizon is a single bucket. Restricting starts to the
    /// template's slots is a separate decision and stays with the caller that also has an
    /// academic period.
    pub fn with_schedule_template(mut self, schedule_template: ScheduleTemplate) -> Self {
        self.schedule_template = Some(schedule_template);
        self
    }

    pub fn with_score_rule(mut self, rule: ScoreRule) -> Self {
        self.score_rules.push(rule);
        self
    }

    pub fn with_relation(mut self, relation: ActivityRelationConstraint) -> Self {
        self.relations.push(relation);
        self
    }

    /// Expands one template [`Activity`] into the instances named by `recurrence`, shifting the
    /// template's window by `k * step` for instance index `k` (plan 28).
    ///
    /// The template is a **blueprint only** and is not added to the problem — every activity the
    /// caller wants must appear in [`Recurrence::instances`] under the id it should be reported
    /// with. That keeps the caller in full control of naming and lets an index `k = 0` coincide
    /// with the template's own window without producing a duplicate.
    ///
    /// Each instance keeps the template's duration and its whole requirement set (resource and
    /// participant requirements, participant groups); only the time window moves. Shifting uses
    /// saturating arithmetic, so an unusually large `step` cannot wrap.
    ///
    /// The instances are tied together by [`ActivityRelation::FixedOffset`] relations in a **star**
    /// around the first entry of `instances`: every other instance is linked directly to that
    /// anchor with the exact total offset `(k - k_anchor) * step`. A chain (`k` to `k+1`) would need
    /// only `step` per link but lets the individual offsets add up, so a single wrong link — or a
    /// caller tweaking one relation — shifts everything downstream; in a star each instance's
    /// position is pinned by one independent equation, and one link is enough for the solver's
    /// bounds propagation to move the whole group. The anchor is the *first list entry*, not
    /// necessarily index `0`, so a caller may order the vector however it likes.
    pub fn with_recurring_activity(mut self, template: Activity, recurrence: Recurrence) -> Self {
        let mut anchor: Option<(i64, ActivityId)> = None;
        for &(k, id) in &recurrence.instances {
            let shift = i64::from(k).saturating_mul(recurrence.step);
            let window = template.allowed_window;
            self.activities.push(Activity {
                id,
                name: template.name.clone(),
                allowed_window: TimeWindow::new(
                    window.start.saturating_add(shift),
                    window.end.saturating_add(shift),
                ),
                duration: template.duration,
                participants: template.participants.clone(),
                participant_groups: template.participant_groups.clone(),
                participant_requirements: template.participant_requirements.clone(),
                requirements: template.requirements.clone(),
            });
            match anchor {
                None => anchor = Some((i64::from(k), id)),
                Some((anchor_k, anchor_id)) => {
                    self.relations.push(ActivityRelationConstraint::new(
                        anchor_id,
                        id,
                        ActivityRelation::FixedOffset {
                            offset: i64::from(k)
                                .saturating_sub(anchor_k)
                                .saturating_mul(recurrence.step),
                        },
                    ));
                }
            }
        }
        self
    }

    /// Adds a per-entity cap on the occupied duration within each period bucket.
    pub fn with_maximum_daily_load(mut self, load: MaximumDailyLoad) -> Self {
        self.maximum_daily_loads.push(load);
        self
    }

    /// Adds a per-entity minimum distance between any two of its assignments.
    pub fn with_minimum_break(mut self, minimum_break: MinimumBreak) -> Self {
        self.minimum_breaks.push(minimum_break);
        self
    }

    /// Adds an allowed teaching-shape rule for a group of activities (plan 25, C1).
    ///
    /// The rule is compiled as a hard constraint: the consecutive blocks the group occupies must
    /// form one of the pattern's allowed multisets. An inactive pattern (no activities or no
    /// allowed shape) is ignored rather than rejected — see [`BucketLoadPattern::is_active`].
    pub fn with_bucket_load_pattern(mut self, pattern: BucketLoadPattern) -> Self {
        self.bucket_load_patterns.push(pattern);
        self
    }

    /// Adds a group whose activities must all select the same participant (plan 33.1).
    pub fn with_participant_choice_group(mut self, group: ParticipantChoiceGroup) -> Self {
        self.participant_choice_groups.push(group);
        self
    }

    /// Adds a problem-level soft goal scored at its own level and reported under its category.
    pub fn with_soft_goal(mut self, goal: SoftGoal) -> Self {
        self.soft_goals.push(goal);
        self
    }

    /// Decides whether two overlapping activities of the same participant block an arrangement or
    /// are only reported (plan 26, §1). Defaults to [`ParticipantConflictPolicy::Advisory`], which
    /// is the previous behaviour, so callers that do not care keep it unchanged.
    pub fn with_participant_conflict_policy(mut self, policy: ParticipantConflictPolicy) -> Self {
        self.participant_conflict_policy = policy;
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
    Aborted(AbortReason),
}

/// Why a solve run stopped without reaching a conclusive result — mirrors
/// `unifier::solver::AbortReason` (except for [`AbortReason::RejectedSolution`], which this
/// crate decides on its own), kept as a distinct type here so callers never need to depend on
/// `unifier` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// The caller's [`crate::CancellationToken`] was cancelled.
    Cancelled,
    /// The configured time limit elapsed.
    Timeout,
    /// The configured search node limit was reached.
    NodeLimit,
    /// A local-search-style solver reached a local optimum with no improving move
    /// available. This does not prove infeasibility or optimality.
    LocalOptimum,
    /// The search returned an assignment violating hard constraints, so it was withheld instead
    /// of being handed on as a schedule.
    ///
    /// This says something about the search, not about the problem: it is no evidence that a
    /// valid schedule does or does not exist. It is reported rather than swallowed because a
    /// schedule that quietly breaks the rules it was given is worse than no schedule at all.
    RejectedSolution,
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
