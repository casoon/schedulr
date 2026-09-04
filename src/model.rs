use std::collections::BTreeMap;
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
}

impl Resource {
    pub fn new(id: ResourceId, name: impl Into<String>, capacity: u32) -> Self {
        let capacities = BTreeMap::from([(DEFAULT_CAPACITY_DIMENSION.to_string(), capacity)]);
        Self {
            id,
            name: name.into(),
            capacities,
            attributes: BTreeMap::new(),
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
}

/// Person or group whose simultaneous activities can be detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    id: ParticipantId,
    name: String,
    attributes: BTreeMap<String, String>,
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
    resource: ResourceId,
    dimension: String,
    units: u32,
    attributes: BTreeMap<String, String>,
}

impl ResourceRequirement {
    pub fn new(resource: ResourceId, units: u32) -> Self {
        Self {
            resource,
            dimension: DEFAULT_CAPACITY_DIMENSION.to_string(),
            units,
            attributes: BTreeMap::new(),
        }
    }

    pub fn for_dimension(resource: ResourceId, dimension: impl Into<String>, units: u32) -> Self {
        Self {
            resource,
            dimension: dimension.into(),
            units,
            attributes: BTreeMap::new(),
        }
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub const fn resource(&self) -> ResourceId {
        self.resource
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

/// Scheduling demand independent of any concrete solution assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    id: ActivityId,
    name: String,
    allowed_window: TimeWindow,
    duration: u64,
    participants: Vec<ParticipantId>,
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

    pub fn requirements(&self) -> &[ResourceRequirement] {
        &self.requirements
    }
}

/// Concrete placement of one activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub activity: ActivityId,
    pub window: TimeWindow,
}

impl Assignment {
    pub const fn new(activity: ActivityId, window: TimeWindow) -> Self {
        Self { activity, window }
    }
}

/// Public score independent of the underlying solver representation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score {
    pub hard: i64,
    pub soft: i64,
}

/// A solved set of activity placements and its aggregate score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub assignments: Vec<Assignment>,
    pub score: Score,
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
        }
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
}
