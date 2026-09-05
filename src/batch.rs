use crate::model::{
    Activity, ActivityId, ActivityRelation, Assignment, CompileError, Conflict, ConflictSeverity,
    EntityRef, GroupMember, Participant, ParticipantGroupId, ParticipantId, ParticipantRequirement,
    Resource, ResourceId, ResourcePool, ResourceRequirement, SchedulingProblem, Score,
    ScoreComponent, ScoreLevel, ScoreRule, ScoreRuleKind, Solution, SolveResult, SolveStatistics,
    SolveStatus, TimeWindow,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use unifier::constraint::{
    Assignment as UnifierAssignment, Constraint, Explanation, PropagationResult,
};
use unifier::model::domain::{Domain, TrailedDomains};
use unifier::propagation::{ConstraintId, ConstraintViolation, ValidatedGraph};
use unifier::score::{Objective, ScoreCalculator, ScoreLevel as UnifierScoreLevel};
use unifier::solver::{
    BacktrackingSolver, BranchAndBoundSolver, SolveOutcome, SolveStatus as UnifierSolveStatus,
    SolverOptions,
};
use unifier::{ForbiddenValues, ModelBuilder, VariableId};

#[derive(Debug)]
pub struct CompiledProblem {
    pub(crate) internal: InternalCompiled,
    pub(crate) problem: SchedulingProblem,
}

#[derive(Debug)]
pub(crate) struct InternalCompiled {
    pub(crate) graph: ValidatedGraph,
    pub(crate) activities: BTreeMap<ActivityId, Activity>,
    variables: BTreeMap<ActivityId, (VariableId, VariableId)>,
    activity_by_variable: HashMap<VariableId, ActivityId>,
    selected_resources: BTreeMap<ActivityId, Vec<(ResourceId, VariableId)>>,
    fixed_resources: BTreeMap<ActivityId, Vec<ResourceId>>,
    selected_participants: BTreeMap<ActivityId, Vec<(ParticipantId, VariableId)>>,
    fixed_participants: BTreeMap<ActivityId, Vec<ParticipantId>>,
    selected_capacity_constraints: Vec<(ConstraintId, Arc<SelectedResourceCapacity>)>,
    constraint_entities: HashMap<ConstraintId, EntityRef>,
    resource_names: BTreeMap<ResourceId, String>,
    participant_names: BTreeMap<ParticipantId, String>,
}

pub fn compile(problem: &SchedulingProblem) -> Result<CompiledProblem, CompileError> {
    build_problem_internal(problem).map(|internal| CompiledProblem {
        internal,
        problem: problem.clone(),
    })
}

impl CompiledProblem {
    pub fn solve(&self) -> SolveResult {
        let options = SolverOptions::default();
        let outcome = if self.internal.graph.objectives().is_empty() {
            BacktrackingSolver::new().solve(&self.internal.graph, &options)
        } else {
            BranchAndBoundSolver::new().solve(&self.internal.graph, &options)
        };
        self.internal.solve_result(outcome)
    }

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
        let assignment = proposed.iter().copied().collect::<UnifierAssignment>();
        let mut violations = self
            .internal
            .graph
            .check_incremental(&UnifierAssignment::new(), &proposed);
        let mut reported = violations
            .iter()
            .map(|violation| violation.constraint_id)
            .collect::<BTreeSet<_>>();
        for (constraint_id, constraint) in &self.internal.selected_capacity_constraints {
            if reported.contains(constraint_id) {
                continue;
            }
            if let Some(explanation) = constraint.explain_pessimistic(&assignment) {
                violations.push(ConstraintViolation {
                    constraint_id: *constraint_id,
                    constraint_name: explanation.constraint_name.to_string(),
                    involved: explanation.involved,
                    message: explanation.message,
                });
                reported.insert(*constraint_id);
            }
        }
        self.internal.conflicts_from_violations(violations)
    }
}

pub(crate) fn build_internal(
    resources: &[Resource],
    participants: &[Participant],
    activities: &[Activity],
    resource_pools: &[ResourcePool],
) -> Result<InternalCompiled, CompileError> {
    let mut problem = SchedulingProblem::new(
        resources.to_vec(),
        participants.to_vec(),
        activities.to_vec(),
    );
    problem.resource_pools = resource_pools.to_vec();
    build_problem_internal(&problem)
}

fn build_problem_internal(problem: &SchedulingProblem) -> Result<InternalCompiled, CompileError> {
    let mut errors = validate_input(problem);
    let expanded_activities = expand_group_participants(problem, &mut errors);
    if !errors.is_empty() {
        return Err(CompileError::new(errors));
    }

    let mut builder = ModelBuilder::new();
    let mut resource_map = BTreeMap::new();
    let mut participant_map = BTreeMap::new();
    let mut compiled_resources = Vec::new();
    let mut resource_entities = HashMap::new();

    for resource in &problem.resources {
        let compiled = builder.new_resource(resource.name(), resource.capacity());
        resource_map.insert(resource.id(), compiled.id());
        resource_entities.insert(compiled.id(), EntityRef::Resource(resource.id()));
        compiled_resources.push(compiled);
    }
    for participant in &problem.participants {
        let compiled = builder.new_resource(participant.name(), 1);
        participant_map.insert(participant.id(), compiled.id());
        resource_entities.insert(compiled.id(), EntityRef::Participant(participant.id()));
        compiled_resources.push(compiled);
    }

    let mut compiled_activities = Vec::new();
    let mut variables = BTreeMap::new();
    let mut activity_by_variable = HashMap::new();
    let mut flexible_requirements = Vec::new();
    let mut flexible_participant_requirements = Vec::new();
    let mut fixed_resources: BTreeMap<ActivityId, Vec<ResourceId>> = BTreeMap::new();
    let mut fixed_participants: BTreeMap<ActivityId, Vec<ParticipantId>> = BTreeMap::new();

    for activity in &expanded_activities {
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

        if let (Some(period), Some(template)) =
            (problem.academic_period, problem.schedule_template.as_ref())
        {
            builder.add_periodic_calendar(
                interval.start(),
                template.cycle_length,
                template.allowed_starts_for(activity.duration()),
                template.unavailable_ranges.iter().copied(),
            );
            if window.start < period.window.start || window.end > period.window.end {
                errors.push(format!(
                    "activity {} lies outside the academic period",
                    activity.id()
                ));
            }
        }

        let interval_start = interval.start();
        let mut compiled = builder.new_activity(activity.name(), interval);
        for requirement in activity.requirements() {
            let candidates = resolve_requirement(problem, requirement);
            if candidates.len() == 1 && requirement.exact_resource().is_some() {
                let resource = candidates[0];
                compiled.require_resource(resource_map[&resource], requirement.units());
                apply_resource_calendar(&mut builder, problem, interval_start, resource);
                fixed_resources
                    .entry(activity.id())
                    .or_default()
                    .push(resource);
            } else {
                flexible_requirements.push((activity.id(), requirement.clone(), candidates));
            }
        }
        for &participant in activity.participants() {
            compiled.require_resource(participant_map[&participant], 1);
            apply_participant_calendar(&mut builder, problem, interval_start, participant);
        }
        for requirement in activity.participant_requirements() {
            let candidates = resolve_participant_requirement(problem, requirement);
            if candidates.len() == 1 && requirement.exact_participant().is_some() {
                let participant = candidates[0];
                compiled.require_resource(participant_map[&participant], 1);
                apply_participant_calendar(&mut builder, problem, interval_start, participant);
                fixed_participants
                    .entry(activity.id())
                    .or_default()
                    .push(participant);
            } else {
                flexible_participant_requirements.push((activity.id(), candidates));
            }
        }
        compiled_activities.push(compiled);
    }

    if !errors.is_empty() {
        return Err(CompileError::new(errors));
    }

    let mut selected_resources: BTreeMap<ActivityId, Vec<(ResourceId, VariableId)>> =
        BTreeMap::new();
    let mut selected_participants: BTreeMap<ActivityId, Vec<(ParticipantId, VariableId)>> =
        BTreeMap::new();
    let compiled_by_activity: BTreeMap<_, _> = expanded_activities
        .iter()
        .zip(&compiled_activities)
        .map(|(domain, compiled)| (domain.id(), compiled))
        .collect();
    for relation in &problem.relations {
        let first_interval = compiled_by_activity[&relation.first].interval();
        let second_interval = compiled_by_activity[&relation.second].interval();
        match relation.relation {
            ActivityRelation::SameStart => {
                builder.add_equal(first_interval.start(), second_interval.start(), 0);
            }
            ActivityRelation::Consecutive => {
                builder.add_equal(first_interval.end(), second_interval.start(), 0);
            }
            ActivityRelation::Precedence { min_gap } => {
                builder.add_precedence(first_interval, second_interval, min_gap);
            }
            ActivityRelation::NoOverlap => {
                let first_duration = duration_of(&expanded_activities, relation.first);
                let second_duration = duration_of(&expanded_activities, relation.second);
                builder.add_no_overlap(
                    &[first_interval.clone(), second_interval.clone()],
                    &[first_duration, second_duration],
                );
            }
        }
    }
    let mut flexible_tasks: BTreeMap<ResourceId, Vec<SelectedTask>> = BTreeMap::new();
    for (activity_id, requirement, candidates) in flexible_requirements {
        let presences = candidates
            .iter()
            .map(|resource| builder.new_presence_var(format!("{activity_id}_on_{resource}")))
            .collect::<Vec<_>>();
        builder.add_exactly_one(presences.clone(), 1);
        for (&resource, &presence) in candidates.iter().zip(&presences) {
            selected_resources
                .entry(activity_id)
                .or_default()
                .push((resource, presence));
            let interval = compiled_by_activity[&activity_id].interval();
            apply_optional_resource_calendar(
                &mut builder,
                problem,
                interval.start(),
                resource,
                presence,
            );
            flexible_tasks
                .entry(resource)
                .or_default()
                .push(SelectedTask {
                    start: interval.start(),
                    duration: duration_as_i64(
                        expanded_activities
                            .iter()
                            .find(|activity| activity.id() == activity_id)
                            .expect("compiled activity exists")
                            .duration(),
                    ),
                    demand: requirement.units(),
                    presence: Some(presence),
                });
        }
    }
    let mut flexible_participant_tasks: BTreeMap<ParticipantId, Vec<SelectedTask>> =
        BTreeMap::new();
    for (activity_id, candidates) in flexible_participant_requirements {
        let presences = candidates
            .iter()
            .map(|participant| {
                builder.new_presence_var(format!("{activity_id}_with_{participant}"))
            })
            .collect::<Vec<_>>();
        builder.add_exactly_one(presences.clone(), 1);
        for (&participant, &presence) in candidates.iter().zip(&presences) {
            selected_participants
                .entry(activity_id)
                .or_default()
                .push((participant, presence));
            let interval = compiled_by_activity[&activity_id].interval();
            apply_optional_participant_calendar(
                &mut builder,
                problem,
                interval.start(),
                participant,
                presence,
            );
            flexible_participant_tasks
                .entry(participant)
                .or_default()
                .push(SelectedTask {
                    start: interval.start(),
                    duration: duration_as_i64(
                        expanded_activities
                            .iter()
                            .find(|activity| activity.id() == activity_id)
                            .expect("compiled activity exists")
                            .duration(),
                    ),
                    demand: 1,
                    presence: Some(presence),
                });
        }
    }

    let resource_constraints = builder
        .compile_scheduling_model(&compiled_activities, &compiled_resources, &[])
        .map_err(|model_errors| {
            errors.extend(model_errors.into_iter().map(|error| error.to_string()));
            CompileError::new(errors.clone())
        })?;
    let mut constraint_entities: HashMap<ConstraintId, EntityRef> = resource_constraints
        .into_iter()
        .map(|(resource, constraint)| (constraint, resource_entities[&resource]))
        .collect();

    let mut selected_capacity_constraints = Vec::new();
    for (resource, mut tasks) in flexible_tasks {
        for activity in &expanded_activities {
            if fixed_resources
                .get(&activity.id())
                .is_some_and(|resources| resources.contains(&resource))
            {
                tasks.push(SelectedTask {
                    start: variables[&activity.id()].0,
                    duration: duration_as_i64(activity.duration()),
                    demand: activity
                        .requirements()
                        .iter()
                        .find(|requirement| requirement.exact_resource() == Some(resource))
                        .map_or(1, ResourceRequirement::units),
                    presence: None,
                });
            }
        }
        let capacity = problem
            .resources
            .iter()
            .find(|candidate| candidate.id() == resource)
            .expect("validated resource")
            .capacity();
        let selected_constraint = Arc::new(SelectedResourceCapacity::new(tasks, capacity));
        let constraint = builder.add_constraint(selected_constraint.clone());
        constraint_entities.insert(constraint, EntityRef::Resource(resource));
        selected_capacity_constraints.push((constraint, selected_constraint));
    }

    for (participant, mut tasks) in flexible_participant_tasks {
        for activity in &expanded_activities {
            let fixed = activity.participants().contains(&participant)
                || fixed_participants
                    .get(&activity.id())
                    .is_some_and(|participants| participants.contains(&participant));
            if fixed {
                tasks.push(SelectedTask {
                    start: variables[&activity.id()].0,
                    duration: duration_as_i64(activity.duration()),
                    demand: 1,
                    presence: None,
                });
            }
        }
        let selected_constraint = Arc::new(SelectedResourceCapacity::new(tasks, 1));
        let constraint = builder.add_constraint(selected_constraint.clone());
        constraint_entities.insert(constraint, EntityRef::Participant(participant));
        selected_capacity_constraints.push((constraint, selected_constraint));
    }

    for rule in &problem.score_rules {
        let Some(&(start, _)) = variables.get(&rule.activity) else {
            continue;
        };
        builder.add_scored_objective(
            &rule.category,
            score_level(rule.level),
            Arc::new(RuleObjective::new(start, rule.clone())),
        );
    }

    let graph = builder.build().map_err(|model_errors| {
        CompileError::new(model_errors.into_iter().map(|e| e.to_string()).collect())
    })?;

    Ok(InternalCompiled {
        graph,
        activities: expanded_activities
            .into_iter()
            .map(|activity| (activity.id(), activity))
            .collect(),
        variables,
        activity_by_variable,
        selected_resources,
        fixed_resources,
        selected_participants,
        fixed_participants,
        selected_capacity_constraints,
        constraint_entities,
        resource_names: problem
            .resources
            .iter()
            .map(|resource| (resource.id(), resource.name().to_string()))
            .collect(),
        participant_names: problem
            .participants
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

    pub(crate) fn assignment_map(&self, solution: &Solution) -> UnifierAssignment {
        let mut assignment = UnifierAssignment::new();
        for item in &solution.assignments {
            if let Some(&(start, end)) = self.variables.get(&item.activity) {
                assignment.insert(start, item.window.start);
                assignment.insert(end, item.window.end);
            }
            if let Some(candidates) = self.selected_resources.get(&item.activity) {
                for &(resource, presence) in candidates {
                    assignment.insert(presence, i64::from(item.resources.contains(&resource)));
                }
            }
            if let Some(candidates) = self.selected_participants.get(&item.activity) {
                for &(participant, presence) in candidates {
                    assignment.insert(
                        presence,
                        i64::from(item.participants.contains(&participant)),
                    );
                }
            }
        }
        assignment
    }

    pub(crate) fn score_for(&self, assignment: &UnifierAssignment) -> Score {
        score_from_unifier(ScoreCalculator.calculate_score(&self.graph, assignment))
    }

    pub(crate) fn score_components(&self, assignment: &UnifierAssignment) -> Vec<ScoreComponent> {
        self.graph
            .objectives()
            .iter()
            .map(|objective| ScoreComponent {
                category: objective.category().to_string(),
                level: public_score_level(objective.level()),
                value: objective.evaluate(assignment),
                activity: objective
                    .scope()
                    .iter()
                    .find_map(|variable| self.activity_by_variable.get(variable).copied()),
            })
            .collect()
    }

    pub(crate) fn solve_result(&self, outcome: SolveOutcome) -> SolveResult {
        let statistics = SolveStatistics {
            nodes_expanded: outcome.statistics.nodes_expanded,
            elapsed_millis: outcome.statistics.elapsed.as_millis(),
            optimal: matches!(outcome.status, UnifierSolveStatus::Optimal),
        };
        let status = match outcome.status {
            UnifierSolveStatus::Optimal | UnifierSolveStatus::Feasible => SolveStatus::Feasible,
            UnifierSolveStatus::Infeasible => SolveStatus::Infeasible,
            UnifierSolveStatus::Aborted(_) => SolveStatus::Aborted,
        };
        let solution = outcome
            .solution
            .map(|solution| self.public_solution(solution));
        SolveResult {
            status,
            solution,
            statistics,
        }
    }

    fn public_solution(&self, solution: unifier::Solution) -> Solution {
        let mut assignments = self
            .activities
            .values()
            .map(|activity| {
                let (start, _) = self.variables[&activity.id()];
                let start_value = solution.assignment[&start];
                let mut assignment = Assignment::new(
                    activity.id(),
                    TimeWindow::new(
                        start_value,
                        start_value.saturating_add(duration_as_i64(activity.duration())),
                    ),
                );
                if let Some(resources) = self.fixed_resources.get(&activity.id()) {
                    assignment.resources.extend(resources);
                }
                if let Some(candidates) = self.selected_resources.get(&activity.id()) {
                    assignment.resources.extend(candidates.iter().filter_map(
                        |&(resource, presence)| {
                            (solution.assignment.get(&presence) == Some(&1)).then_some(resource)
                        },
                    ));
                }
                assignment.resources.sort_unstable();
                assignment.resources.dedup();
                assignment
                    .participants
                    .extend(activity.participants().iter().copied());
                if let Some(participants) = self.fixed_participants.get(&activity.id()) {
                    assignment.participants.extend(participants);
                }
                if let Some(candidates) = self.selected_participants.get(&activity.id()) {
                    assignment.participants.extend(candidates.iter().filter_map(
                        |&(participant, presence)| {
                            (solution.assignment.get(&presence) == Some(&1)).then_some(participant)
                        },
                    ));
                }
                assignment.participants.sort_unstable();
                assignment.participants.dedup();
                assignment
            })
            .collect::<Vec<_>>();
        assignments.sort_by_key(|assignment| assignment.activity);
        Solution {
            assignments,
            score: score_from_unifier(solution.score),
            score_components: self.score_components(&solution.assignment),
        }
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

fn validate_input(problem: &SchedulingProblem) -> Vec<String> {
    let resource_ids: BTreeMap<_, _> = problem
        .resources
        .iter()
        .map(|resource| (resource.id(), resource))
        .collect();
    let participant_ids: BTreeSet<_> = problem.participants.iter().map(Participant::id).collect();
    let group_ids: BTreeSet<_> = problem
        .participant_groups
        .iter()
        .map(|group| group.id)
        .collect();
    let pool_ids: BTreeMap<_, _> = problem
        .resource_pools
        .iter()
        .map(|pool| (pool.id, pool))
        .collect();
    let participant_pool_ids: BTreeMap<_, _> = problem
        .participant_pools
        .iter()
        .map(|pool| (pool.id, pool))
        .collect();
    let mut errors = Vec::new();
    let mut seen_resources = BTreeSet::new();
    for resource in &problem.resources {
        if !seen_resources.insert(resource.id()) {
            errors.push(format!("duplicate resource id {}", resource.id()));
        }
        if resource.capacity() == 0 {
            errors.push(format!("resource {} has zero capacity", resource.id()));
        }
    }
    let mut seen_participants = BTreeSet::new();
    for participant in &problem.participants {
        if !seen_participants.insert(participant.id()) {
            errors.push(format!("duplicate participant id {}", participant.id()));
        }
    }
    for membership in &problem.group_memberships {
        if !group_ids.contains(&membership.group) {
            errors.push(format!("unknown participant group {}", membership.group));
        }
        match membership.member {
            GroupMember::Participant(id) if !participant_ids.contains(&id) => {
                errors.push(format!("unknown participant {id} in group membership"));
            }
            GroupMember::Group(id) if !group_ids.contains(&id) => {
                errors.push(format!("unknown subgroup {id}"));
            }
            _ => {}
        }
    }
    for pool in &problem.resource_pools {
        for resource in &pool.resources {
            if !resource_ids.contains_key(resource) {
                errors.push(format!(
                    "pool {} references unknown resource {resource}",
                    pool.id
                ));
            }
        }
    }
    for pool in &problem.participant_pools {
        for participant in &pool.participants {
            if !participant_ids.contains(participant) {
                errors.push(format!(
                    "participant pool {} references unknown participant {participant}",
                    pool.id
                ));
            }
        }
    }
    let mut seen_activities = BTreeSet::new();
    for activity in &problem.activities {
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
        for group in activity.participant_groups() {
            if !group_ids.contains(group) {
                errors.push(format!(
                    "activity {} references unknown group {group}",
                    activity.id()
                ));
            }
        }
        for requirement in activity.requirements() {
            if requirement.units() == 0 {
                errors.push(format!(
                    "activity {} has a zero-unit resource requirement",
                    activity.id()
                ));
            }
            if let Some(resource) = requirement.exact_resource()
                && !resource_ids.contains_key(&resource)
            {
                errors.push(format!(
                    "activity {} requires unknown resource {resource}",
                    activity.id()
                ));
            }
            if let Some(pool) = requirement.pool()
                && !pool_ids.contains_key(&pool)
            {
                errors.push(format!(
                    "activity {} references unknown pool {pool}",
                    activity.id()
                ));
            }
            if resolve_requirement(problem, requirement).is_empty() {
                errors.push(format!(
                    "activity {} has no matching resource for a requirement",
                    activity.id()
                ));
            }
        }
        for participant in activity.participants() {
            if !participant_ids.contains(participant) {
                errors.push(format!(
                    "activity {} references unknown participant {participant}",
                    activity.id()
                ));
            }
        }
        for requirement in activity.participant_requirements() {
            if let Some(participant) = requirement.exact_participant()
                && !participant_ids.contains(&participant)
            {
                errors.push(format!(
                    "activity {} requires unknown participant {participant}",
                    activity.id()
                ));
            }
            if let Some(pool) = requirement.pool()
                && !participant_pool_ids.contains_key(&pool)
            {
                errors.push(format!(
                    "activity {} references unknown participant pool {pool}",
                    activity.id()
                ));
            }
            if resolve_participant_requirement(problem, requirement).is_empty() {
                errors.push(format!(
                    "activity {} has no matching participant for a requirement",
                    activity.id()
                ));
            }
        }
    }
    if let Some(template) = &problem.schedule_template {
        if template.cycle_length <= 0 {
            errors.push("schedule template cycle length must be positive".to_string());
        }
        if template.allowed_starts().is_empty() {
            errors.push("schedule template must contain at least one slot".to_string());
        }
    }
    for rule in &problem.score_rules {
        if !seen_activities.contains(&rule.activity) {
            errors.push(format!(
                "score rule references unknown activity {}",
                rule.activity
            ));
        }
        if rule.weight <= 0 {
            errors.push(format!(
                "score rule '{}' must have a positive weight",
                rule.category
            ));
        }
    }
    for relation in &problem.relations {
        if !seen_activities.contains(&relation.first) {
            errors.push(format!(
                "activity relation references unknown activity {}",
                relation.first
            ));
        }
        if !seen_activities.contains(&relation.second) {
            errors.push(format!(
                "activity relation references unknown activity {}",
                relation.second
            ));
        }
        if relation.first == relation.second {
            errors.push(format!(
                "activity relation cannot relate activity {} to itself",
                relation.first
            ));
        }
    }
    errors
}

fn resolve_requirement(
    problem: &SchedulingProblem,
    requirement: &ResourceRequirement,
) -> Vec<ResourceId> {
    if let Some(resource) = requirement.exact_resource() {
        return vec![resource];
    }
    let pool_members = requirement.pool().and_then(|pool_id| {
        problem
            .resource_pools
            .iter()
            .find(|pool| pool.id == pool_id)
            .map(|pool| pool.resources.iter().copied().collect::<BTreeSet<_>>())
    });
    problem
        .resources
        .iter()
        .filter(|resource| {
            requirement
                .resource_type()
                .is_none_or(|kind| resource.resource_type() == kind)
                && resource.capacity() >= requirement.minimum_capacity()
                && requirement
                    .required_features()
                    .is_subset(resource.features())
                && (requirement.candidates().is_empty()
                    || requirement.candidates().contains(&resource.id()))
        })
        .filter(|resource| {
            pool_members
                .as_ref()
                .is_none_or(|members| members.contains(&resource.id()))
        })
        .map(Resource::id)
        .collect()
}

fn resolve_participant_requirement(
    problem: &SchedulingProblem,
    requirement: &ParticipantRequirement,
) -> Vec<ParticipantId> {
    if let Some(participant) = requirement.exact_participant() {
        return vec![participant];
    }
    let pool_members = requirement.pool().and_then(|pool_id| {
        problem
            .participant_pools
            .iter()
            .find(|pool| pool.id == pool_id)
            .map(|pool| pool.participants.iter().copied().collect::<BTreeSet<_>>())
    });
    problem
        .participants
        .iter()
        .filter(|participant| {
            requirement.candidates().is_empty()
                || requirement.candidates().contains(&participant.id())
        })
        .filter(|participant| {
            pool_members
                .as_ref()
                .is_none_or(|members| members.contains(&participant.id()))
        })
        .map(Participant::id)
        .collect()
}

/// Restricts `start` to avoid `resource`'s unavailable ranges (AllowedTime), unconditionally —
/// for use where `resource` is guaranteed to be booked for the activity.
fn apply_resource_calendar(
    builder: &mut ModelBuilder,
    problem: &SchedulingProblem,
    start: VariableId,
    resource: ResourceId,
) {
    if let Some(found) = problem
        .resources
        .iter()
        .find(|candidate| candidate.id() == resource)
    {
        let ranges = found.unavailable_ranges();
        if !ranges.is_empty() {
            builder.add_calendar(start, ranges);
        }
    }
}

/// Restricts `start` to avoid `participant`'s unavailable ranges (Availability), unconditionally —
/// for use where `participant` is guaranteed to attend the activity.
fn apply_participant_calendar(
    builder: &mut ModelBuilder,
    problem: &SchedulingProblem,
    start: VariableId,
    participant: ParticipantId,
) {
    if let Some(found) = problem
        .participants
        .iter()
        .find(|candidate| candidate.id() == participant)
    {
        let ranges = found.unavailable_ranges();
        if !ranges.is_empty() {
            builder.add_calendar(start, ranges);
        }
    }
}

/// Restricts `start` to avoid `resource`'s unavailable ranges (AllowedTime), but only if
/// `resource` is the one actually selected for the activity (`presence`).
fn apply_optional_resource_calendar(
    builder: &mut ModelBuilder,
    problem: &SchedulingProblem,
    start: VariableId,
    resource: ResourceId,
    presence: VariableId,
) {
    if let Some(found) = problem
        .resources
        .iter()
        .find(|candidate| candidate.id() == resource)
    {
        let ranges = found.unavailable_ranges();
        if !ranges.is_empty() {
            let forbidden = ranges
                .iter()
                .flat_map(|&(range_start, range_end)| range_start..=range_end);
            builder.add_optional(Arc::new(ForbiddenValues::new(start, forbidden)), presence);
        }
    }
}

/// Restricts `start` to avoid `participant`'s unavailable ranges (Availability), but only if
/// `participant` is the one actually selected for the activity (`presence`).
fn apply_optional_participant_calendar(
    builder: &mut ModelBuilder,
    problem: &SchedulingProblem,
    start: VariableId,
    participant: ParticipantId,
    presence: VariableId,
) {
    if let Some(found) = problem
        .participants
        .iter()
        .find(|candidate| candidate.id() == participant)
    {
        let ranges = found.unavailable_ranges();
        if !ranges.is_empty() {
            let forbidden = ranges
                .iter()
                .flat_map(|&(range_start, range_end)| range_start..=range_end);
            builder.add_optional(Arc::new(ForbiddenValues::new(start, forbidden)), presence);
        }
    }
}

fn expand_group_participants(
    problem: &SchedulingProblem,
    errors: &mut Vec<String>,
) -> Vec<Activity> {
    let memberships: BTreeMap<ParticipantGroupId, Vec<GroupMember>> = problem
        .participant_groups
        .iter()
        .map(|group| {
            (
                group.id,
                problem
                    .group_memberships
                    .iter()
                    .filter(|membership| membership.group == group.id)
                    .map(|membership| membership.member)
                    .collect(),
            )
        })
        .collect();
    problem
        .activities
        .iter()
        .map(|activity| {
            let mut expanded = activity.clone();
            let mut participants = activity
                .participants()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            for group in activity.participant_groups() {
                expand_group(
                    *group,
                    &memberships,
                    &mut BTreeSet::new(),
                    &mut participants,
                    errors,
                );
            }
            for participant in participants {
                if !expanded.participants().contains(&participant) {
                    expanded = expanded.with_participant(participant);
                }
            }
            expanded
        })
        .collect()
}

fn expand_group(
    group: ParticipantGroupId,
    memberships: &BTreeMap<ParticipantGroupId, Vec<GroupMember>>,
    visiting: &mut BTreeSet<ParticipantGroupId>,
    participants: &mut BTreeSet<ParticipantId>,
    errors: &mut Vec<String>,
) {
    if !visiting.insert(group) {
        errors.push(format!("participant group cycle contains {group}"));
        return;
    }
    for member in memberships.get(&group).into_iter().flatten() {
        match member {
            GroupMember::Participant(participant) => {
                participants.insert(*participant);
            }
            GroupMember::Group(subgroup) => {
                expand_group(*subgroup, memberships, visiting, participants, errors);
            }
        }
    }
    visiting.remove(&group);
}

#[derive(Debug, Clone)]
struct RuleObjective {
    variable: VariableId,
    rule: ScoreRule,
    scope: [VariableId; 1],
}

impl RuleObjective {
    fn new(variable: VariableId, rule: ScoreRule) -> Self {
        Self {
            variable,
            rule,
            scope: [variable],
        }
    }

    fn contribution(&self, value: i64) -> i64 {
        match self.rule.kind {
            ScoreRuleKind::PreferWindow(window) => {
                if window.start <= value && value < window.end {
                    0
                } else {
                    self.rule.weight.saturating_neg()
                }
            }
            ScoreRuleKind::KeepStart(start) => {
                if value == start {
                    0
                } else {
                    self.rule.weight.saturating_neg()
                }
            }
        }
    }
}

impl Objective for RuleObjective {
    fn name(&self) -> &str {
        "ScheduleScoreRule"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        assignment
            .get(&self.variable)
            .map_or(0, |&value| self.contribution(value))
    }

    fn optimistic_bound(&self, domains: &HashMap<VariableId, Domain>) -> i64 {
        domains.get(&self.variable).map_or(0, |domain| {
            let can_avoid_penalty = match self.rule.kind {
                ScoreRuleKind::PreferWindow(window) => domain.min().is_some_and(|min| {
                    domain
                        .max()
                        .is_some_and(|max| min < window.end && max >= window.start)
                }),
                ScoreRuleKind::KeepStart(start) => domain.contains(start),
            };
            if can_avoid_penalty {
                0
            } else {
                self.rule.weight.saturating_neg()
            }
        })
    }
}

#[derive(Debug, Clone)]
struct SelectedTask {
    start: VariableId,
    duration: i64,
    demand: u32,
    presence: Option<VariableId>,
}

#[derive(Debug, Clone)]
struct SelectedResourceCapacity {
    tasks: Vec<SelectedTask>,
    capacity: u32,
    scope: Vec<VariableId>,
}

impl SelectedResourceCapacity {
    fn new(tasks: Vec<SelectedTask>, capacity: u32) -> Self {
        let mut scope = Vec::new();
        for task in &tasks {
            scope.push(task.start);
            if let Some(presence) = task.presence {
                scope.push(presence);
            }
        }
        scope.sort_unstable();
        scope.dedup();
        Self {
            tasks,
            capacity,
            scope,
        }
    }

    fn overloaded(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        let mut events = self
            .tasks
            .iter()
            .filter_map(|task| {
                if task
                    .presence
                    .is_some_and(|presence| assignment.get(&presence) != Some(&1))
                {
                    return None;
                }
                let start = *assignment.get(&task.start)?;
                Some((
                    (start, i64::from(task.demand)),
                    (start.saturating_add(task.duration), -i64::from(task.demand)),
                ))
            })
            .flat_map(|(start, end)| [start, end])
            .collect::<Vec<_>>();
        events.sort_unstable();
        let mut load = 0i64;
        events.into_iter().any(|(_, delta)| {
            load = load.saturating_add(delta);
            load > i64::from(self.capacity)
        })
    }

    fn explain_pessimistic(&self, assignment: &UnifierAssignment) -> Option<Explanation> {
        let mut pessimistic = assignment.clone();
        for task in &self.tasks {
            if let Some(presence) = task.presence {
                pessimistic.entry(presence).or_insert(1);
            }
        }
        self.overloaded(&pessimistic).then(|| Explanation {
            constraint_name: "AlternativeResourceCapacity",
            involved: self.scope.clone(),
            message: format!(
                "capacity {} is insufficient for the unresolved alternatives",
                self.capacity
            ),
        })
    }
}

impl Constraint for SelectedResourceCapacity {
    fn name(&self) -> &str {
        "AlternativeResourceCapacity"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn is_satisfied(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        !self.overloaded(assignment)
    }

    fn explain(&self, assignment: &UnifierAssignment) -> Option<Explanation> {
        self.overloaded(assignment).then(|| Explanation {
            constraint_name: "AlternativeResourceCapacity",
            involved: self.scope.clone(),
            message: format!("selected resource capacity {} is exceeded", self.capacity),
        })
    }

    fn propagate(&self, _domains: &mut TrailedDomains) -> PropagationResult {
        PropagationResult::Success { changed: false }
    }
}

fn score_level(level: ScoreLevel) -> UnifierScoreLevel {
    match level {
        ScoreLevel::Strong => UnifierScoreLevel::Strong,
        ScoreLevel::Medium => UnifierScoreLevel::Medium,
        ScoreLevel::Weak => UnifierScoreLevel::Weak,
    }
}

fn public_score_level(level: UnifierScoreLevel) -> ScoreLevel {
    match level {
        UnifierScoreLevel::Strong => ScoreLevel::Strong,
        UnifierScoreLevel::Medium => ScoreLevel::Medium,
        UnifierScoreLevel::Weak => ScoreLevel::Weak,
    }
}

fn score_from_unifier(score: unifier::HardSoftScore) -> Score {
    Score {
        hard: score.hard,
        strong: score.strong,
        medium: score.medium,
        weak: score.weak,
        soft: score.soft,
    }
}

fn duration_as_i64(duration: u64) -> i64 {
    i64::try_from(duration).unwrap_or(i64::MAX)
}

fn duration_of(activities: &[Activity], id: ActivityId) -> u64 {
    activities
        .iter()
        .find(|activity| activity.id() == id)
        .map_or(0, Activity::duration)
}
