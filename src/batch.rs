use crate::model::{
    Activity, ActivityId, Assignment, CompileError, Conflict, ConflictSeverity,
    DEFAULT_CAPACITY_DIMENSION, EntityRef, Participant, ParticipantId, Resource, ResourceId,
    SchedulingProblem, Score, Solution, SolveResult, SolveStatus, TimeWindow,
};
use std::collections::{BTreeMap, HashMap};
use unifier::constraint::Assignment as UnifierAssignment;
use unifier::model::activity::Activity as UnifierActivity;
use unifier::propagation::{ConstraintId, ConstraintViolation, ValidatedGraph};
use unifier::solver::{BacktrackingSolver, SolveStatus as UnifierSolveStatus, SolverOptions};
use unifier::{ModelBuilder, VariableId};

#[derive(Debug)]
pub struct CompiledProblem {
    internal: InternalCompiled,
}

#[derive(Debug)]
pub(crate) struct InternalCompiled {
    graph: ValidatedGraph,
    activities: BTreeMap<ActivityId, Activity>,
    variables: BTreeMap<ActivityId, (VariableId, VariableId)>,
    activity_by_variable: HashMap<VariableId, ActivityId>,
    constraint_entities: HashMap<ConstraintId, EntityRef>,
    resource_names: BTreeMap<ResourceId, String>,
    participant_names: BTreeMap<ParticipantId, String>,
}

pub fn compile(problem: &SchedulingProblem) -> Result<CompiledProblem, CompileError> {
    build_internal(
        &problem.resources,
        &problem.participants,
        &problem.activities,
    )
    .map(|internal| CompiledProblem { internal })
}

impl CompiledProblem {
    pub fn solve(&self) -> SolveResult {
        let outcome =
            BacktrackingSolver::new().solve(&self.internal.graph, &SolverOptions::default());
        let status = match outcome.status {
            UnifierSolveStatus::Optimal | UnifierSolveStatus::Feasible => SolveStatus::Feasible,
            UnifierSolveStatus::Infeasible => SolveStatus::Infeasible,
            UnifierSolveStatus::Aborted(_) => SolveStatus::Aborted,
        };
        let solution = outcome.solution.map(|solution| {
            let mut assignments: Vec<Assignment> = self
                .internal
                .activities
                .values()
                .map(|activity| {
                    let (start, _) = self.internal.variables[&activity.id()];
                    let start = solution.assignment[&start];
                    Assignment::new(
                        activity.id(),
                        TimeWindow::new(
                            start,
                            start.saturating_add(duration_as_i64(activity.duration())),
                        ),
                    )
                })
                .collect();
            assignments.sort_by_key(|assignment| assignment.activity);
            Solution {
                assignments,
                score: Score {
                    hard: solution.score.hard,
                    soft: solution.score.soft,
                },
            }
        });
        SolveResult { status, solution }
    }

    /// Explains concrete hard violations for an infeasible fixed-assignment problem.
    ///
    /// Version 0.1 intentionally does not compute a general unsat core. It explains constraints
    /// for variables whose domains are already fixed, which covers validation of a concrete
    /// proposed schedule and the batch minimum definition of done.
    pub fn explain(&self, result: &SolveResult) -> Vec<Conflict> {
        if result.solution.is_some() {
            return Vec::new();
        }
        let proposed: Vec<(VariableId, i64)> = self
            .internal
            .graph
            .domains()
            .iter()
            .filter(|(_, domain)| domain.len() == 1)
            .map(|(&variable, domain)| (variable, domain.min().expect("singleton domain")))
            .collect();
        self.internal.conflicts_from_violations(
            self.internal
                .graph
                .check_incremental(&UnifierAssignment::new(), &proposed),
        )
    }
}

pub(crate) fn build_internal(
    resources: &[Resource],
    participants: &[Participant],
    activities: &[Activity],
) -> Result<InternalCompiled, CompileError> {
    let mut errors = validate_input(resources, participants, activities);
    if !errors.is_empty() {
        return Err(CompileError::new(errors));
    }

    let mut builder = ModelBuilder::new();
    let mut resource_map = BTreeMap::new();
    let mut participant_map = BTreeMap::new();
    let mut compiled_resources = Vec::new();
    let mut resource_entities = HashMap::new();

    for resource in resources {
        let compiled = builder.new_resource(resource.name(), resource.capacity());
        resource_map.insert(resource.id(), compiled.id());
        resource_entities.insert(compiled.id(), EntityRef::Resource(resource.id()));
        compiled_resources.push(compiled);
    }
    for participant in participants {
        let compiled = builder.new_resource(participant.name(), 1);
        participant_map.insert(participant.id(), compiled.id());
        resource_entities.insert(compiled.id(), EntityRef::Participant(participant.id()));
        compiled_resources.push(compiled);
    }

    let mut compiled_activities: Vec<UnifierActivity> = Vec::new();
    let mut variables = BTreeMap::new();
    let mut activity_by_variable = HashMap::new();
    for activity in activities {
        let duration = duration_as_i64(activity.duration());
        let window = activity.allowed_window();
        let latest_start = window.end.saturating_sub(duration);
        let interval = builder.new_interval(
            activity.name(),
            window.start..=latest_start,
            activity.duration(),
            window.start.saturating_add(duration)..=window.end,
        );
        variables.insert(activity.id(), (interval.start(), interval.end()));
        activity_by_variable.insert(interval.start(), activity.id());
        activity_by_variable.insert(interval.end(), activity.id());

        let mut compiled = builder.new_activity(activity.name(), interval);
        for requirement in activity.requirements() {
            compiled.require_resource(resource_map[&requirement.resource()], requirement.units());
        }
        for participant in activity.participants() {
            compiled.require_resource(participant_map[participant], 1);
        }
        compiled_activities.push(compiled);
    }

    let resource_constraints = builder
        .compile_scheduling_model(&compiled_activities, &compiled_resources, &[])
        .map_err(|model_errors| {
            errors.extend(model_errors.into_iter().map(|error| error.to_string()));
            CompileError::new(errors)
        })?;
    let graph = builder.build().map_err(|model_errors| {
        CompileError::new(model_errors.into_iter().map(|e| e.to_string()).collect())
    })?;

    let constraint_entities = resource_constraints
        .into_iter()
        .map(|(resource, constraint)| (constraint, resource_entities[&resource]))
        .collect();

    Ok(InternalCompiled {
        graph,
        activities: activities
            .iter()
            .cloned()
            .map(|activity| (activity.id(), activity))
            .collect(),
        variables,
        activity_by_variable,
        constraint_entities,
        resource_names: resources
            .iter()
            .map(|resource| (resource.id(), resource.name().to_string()))
            .collect(),
        participant_names: participants
            .iter()
            .map(|participant| (participant.id(), participant.name().to_string()))
            .collect(),
    })
}

impl InternalCompiled {
    pub(crate) fn variables_for(&self, activity: ActivityId) -> (VariableId, VariableId) {
        self.variables[&activity]
    }

    pub(crate) fn check_incremental(
        &self,
        committed: &UnifierAssignment,
        proposed: &[(VariableId, i64)],
    ) -> Vec<Conflict> {
        self.conflicts_from_violations(self.graph.check_incremental(committed, proposed))
    }

    fn conflicts_from_violations(&self, violations: Vec<ConstraintViolation>) -> Vec<Conflict> {
        violations
            .into_iter()
            .map(|violation| {
                let entity = self
                    .constraint_entities
                    .get(&violation.constraint_id)
                    .copied();
                let (severity, entity_name) = match entity {
                    Some(EntityRef::Participant(id)) => (
                        ConflictSeverity::Advisory,
                        self.participant_names.get(&id).map(String::as_str),
                    ),
                    Some(EntityRef::Resource(id)) => (
                        ConflictSeverity::Blocking,
                        self.resource_names.get(&id).map(String::as_str),
                    ),
                    _ => (ConflictSeverity::Blocking, None),
                };
                let mut involved: Vec<ActivityId> = violation
                    .involved
                    .iter()
                    .filter_map(|variable| self.activity_by_variable.get(variable).copied())
                    .collect();
                involved.sort_unstable();
                involved.dedup();
                let message = entity_name.map_or(violation.message.clone(), |name| {
                    format!("{name}: {}", violation.message)
                });
                Conflict {
                    severity,
                    constraint_name: violation.constraint_name,
                    involved,
                    entity,
                    message,
                }
            })
            .collect()
    }
}

fn validate_input(
    resources: &[Resource],
    participants: &[Participant],
    activities: &[Activity],
) -> Vec<String> {
    let resource_ids: BTreeMap<_, _> = resources
        .iter()
        .map(|resource| (resource.id(), resource))
        .collect();
    let participant_ids: BTreeMap<_, _> = participants
        .iter()
        .map(|participant| (participant.id(), participant))
        .collect();
    let mut errors = Vec::new();
    let mut seen_resources = std::collections::BTreeSet::new();
    for resource in resources {
        if !seen_resources.insert(resource.id()) {
            errors.push(format!("duplicate resource id {}", resource.id()));
        }
        if resource.capacity() == 0 {
            errors.push(format!("resource {} has zero capacity", resource.id()));
        }
    }
    let mut seen_participants = std::collections::BTreeSet::new();
    for participant in participants {
        if !seen_participants.insert(participant.id()) {
            errors.push(format!("duplicate participant id {}", participant.id()));
        }
    }
    let mut seen_activities = std::collections::BTreeSet::new();
    for activity in activities {
        if !seen_activities.insert(activity.id()) {
            errors.push(format!("duplicate activity id {}", activity.id()));
        }
        let window_duration = activity.allowed_window().duration();
        if !activity.allowed_window().is_valid()
            || window_duration.is_none_or(|duration| duration < activity.duration())
            || activity.duration() == 0
        {
            errors.push(format!(
                "activity {} has an invalid time window",
                activity.id()
            ));
        }
        for requirement in activity.requirements() {
            if requirement.units() == 0 {
                errors.push(format!(
                    "activity {} has a zero-unit resource requirement",
                    activity.id()
                ));
            }
            if requirement.dimension() != DEFAULT_CAPACITY_DIMENSION {
                errors.push(format!(
                    "activity {} uses unsupported capacity dimension '{}'",
                    activity.id(),
                    requirement.dimension()
                ));
            }
            if !resource_ids.contains_key(&requirement.resource()) {
                errors.push(format!(
                    "activity {} requires unknown resource {}",
                    activity.id(),
                    requirement.resource()
                ));
            }
        }
        for participant in activity.participants() {
            if !participant_ids.contains_key(participant) {
                errors.push(format!(
                    "activity {} references unknown participant {}",
                    activity.id(),
                    participant
                ));
            }
        }
    }
    errors
}

fn duration_as_i64(duration: u64) -> i64 {
    i64::try_from(duration).unwrap_or(i64::MAX)
}
