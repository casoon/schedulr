use crate::model::{
    AbortReason, Activity, ActivityId, ActivityRelation, Assignment, CompileError, Conflict,
    ConflictSeverity, EntityRef, GroupMember, Participant, ParticipantGroupId, ParticipantId,
    ParticipantRequirement, Resource, ResourceId, ResourcePool, ResourceRequirement,
    SchedulingProblem, Score, ScoreComponent, ScoreLevel, ScoreRule, ScoreRuleKind, SoftGoal,
    SoftGoalKind, Solution, SolutionCheck, SolveResult, SolveStatistics, SolveStatus, TimeWindow,
    bucket_windows,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;
use unifier::constraint::{
    Assignment as UnifierAssignment, BucketRange, BucketedTask, Constraint, Explanation,
    PropagationResult,
};
use unifier::model::domain::{Domain, TrailedDomains};
use unifier::propagation::{ConstraintId, ConstraintViolation, ValidatedGraph};
use unifier::score::{Objective, ScoreCalculator, ScoreLevel as UnifierScoreLevel};
pub use unifier::solver::CancellationToken;
use unifier::solver::{
    AbortReason as UnifierAbortReason, BacktrackingSolver, BranchAndBoundSolver, ParallelSolver,
    SearchStatistics, SharedIncumbent, SolveOutcome, SolveStatus as UnifierSolveStatus,
    SolverOptions,
};
use unifier::{ForbiddenValues, ModelBuilder, VariableId};

/// Parameters controlling one [`CompiledProblem::solve_with`] run: how long to search and
/// how to interrupt it early. `solve()` is a convenience default of this with no
/// cancellation and a 10 second time limit — the same default `unifier`'s own
/// `SolverOptions` uses.
#[derive(Debug, Clone)]
pub struct SolveOptions {
    /// Maximum duration to search. `None` means no time limit.
    pub time_limit: Option<Duration>,
    /// Handle a caller can use to interrupt the search from another thread/task.
    pub cancellation_token: Option<CancellationToken>,
    /// How to spend the budget. See [`SolveStrategy`].
    pub strategy: SolveStrategy,
}

impl Default for SolveOptions {
    fn default() -> Self {
        Self {
            time_limit: Some(Duration::from_secs(10)),
            cancellation_token: None,
            strategy: SolveStrategy::default(),
        }
    }
}

/// How [`CompiledProblem::solve_with`] spends its budget.
///
/// All of these end at the same guard: a schedule that violates hard constraints is never
/// returned, whichever strategy produced it (see `InternalCompiled::solve_result`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SolveStrategy {
    /// Construct a feasible schedule, then improve it with Branch & Bound.
    ///
    /// Two phases so that finding *a* schedule and finding a *good* one stay separate questions:
    /// the optimizer starts bounded by a schedule that already exists instead of searching for
    /// its first complete assignment on the clock.
    ConstructThenOptimize,
    /// Construct a feasible schedule, then hand it to the four-worker portfolio (Backtracking,
    /// Local Search, LNS, Branch & Bound over a shared incumbent) for the remaining budget.
    ///
    /// The default, because a tree search alone plateaus where a repair-based one does not: on
    /// the reference instances this both reached schedules [`Self::ConstructThenOptimize`] never
    /// found in the same budget and roughly tripled the quality of the ones it did find. It
    /// costs four threads and four copies of the model.
    ///
    /// Which of several equally good schedules comes back depends on how the workers happen to
    /// be scheduled, so two runs over the same problem can return different arrangements of the
    /// same quality. Pick [`Self::ConstructThenOptimize`] where a run has to be repeatable.
    #[default]
    ConstructThenPortfolio,
    /// One Branch & Bound run over the whole budget, with no construction phase.
    ///
    /// The shape the search had before construction was split out, kept so a benchmark can say
    /// what the split is worth rather than assuming it.
    OptimizeOnly,
}

/// Divides the run's time limit into the share `CompiledProblem::search` gives its
/// construction phase before the optimizer takes over.
///
/// Construction stops at the first hard-feasible assignment, so it only spends its whole share
/// on problems where it finds none — and there the remainder is not lost either, it goes to an
/// unseeded optimizer run, which is exactly what a single-phase search would have done.
const CONSTRUCTION_BUDGET_DIVISOR: u32 = 2;

fn solver_options(time_limit: Option<Duration>, options: &SolveOptions) -> SolverOptions {
    SolverOptions {
        time_limit,
        cancellation_token: options.cancellation_token.clone(),
        ..SolverOptions::default()
    }
}

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
    /// Resolved from [`SchedulingProblem::participant_conflict_policy`] once, at compile time.
    participant_conflict_severity: ConflictSeverity,
    resource_names: BTreeMap<ResourceId, String>,
    participant_names: BTreeMap<ParticipantId, String>,
}

pub fn compile(problem: &SchedulingProblem) -> Result<CompiledProblem, CompileError> {
    compile_with_stability(problem, None)
}

/// Compiles `problem`, optionally attaching the baseline-stability preference and frozen domains
/// [`crate::repair`] needs (plan 31, E0.2). Crate-internal: stability never becomes part of the
/// public [`SchedulingProblem`] vocabulary.
pub(crate) fn compile_with_stability(
    problem: &SchedulingProblem,
    stability: Option<&StabilityBaseline>,
) -> Result<CompiledProblem, CompileError> {
    build_problem_internal(problem, stability).map(|internal| CompiledProblem {
        internal,
        problem: problem.clone(),
    })
}

/// Baseline values a stable repair holds on to (plan 31, E0.2).
///
/// `assignment_change_penalty` is the `Strong` penalty for a presence variable deviating from its
/// baseline value; `frozen` activities have their time **and** resource/participant selections
/// fixed to the baseline domain values.
#[derive(Debug)]
pub(crate) struct StabilityBaseline {
    pub(crate) assignments: BTreeMap<ActivityId, StabilityAssignment>,
    pub(crate) assignment_change_penalty: i64,
    pub(crate) frozen: BTreeSet<ActivityId>,
}

#[derive(Debug)]
pub(crate) struct StabilityAssignment {
    pub(crate) start: i64,
    pub(crate) resources: BTreeSet<ResourceId>,
    pub(crate) participants: BTreeSet<ParticipantId>,
}

impl CompiledProblem {
    /// Solves with a 10 second time limit and no cancellation. Use
    /// [`CompiledProblem::solve_with`] to control the time limit or to pass a
    /// [`CancellationToken`] the caller can cancel from elsewhere.
    pub fn solve(&self) -> SolveResult {
        self.solve_with(&SolveOptions::default())
    }

    /// Solves under the given [`SolveOptions`] (time limit, cancellation), constructing a
    /// feasible schedule first and optimizing it with whatever budget is left (see
    /// `CompiledProblem::search`).
    pub fn solve_with(&self, options: &SolveOptions) -> SolveResult {
        self.internal.solve_result(self.search(options))
    }

    /// Searches in two phases — construct, then optimize — and returns whichever produced the
    /// better schedule.
    ///
    /// Branch & Bound records a solution only once it reaches a *complete* assignment, so on a
    /// problem whose first complete assignment sits deep in the tree it can spend the entire
    /// budget and return nothing at all. Splitting the run decouples the two questions: phase
    /// one asks "does any assignment satisfy the hard constraints" and stops at the first one
    /// it finds; phase two spends the remaining budget improving that assignment's objectives,
    /// bounded by it from its first node onwards instead of searching unbounded.
    ///
    /// The schedule from phase one is never lost: if the optimizer runs out of time before it
    /// can so much as adopt the incumbent, its empty outcome is dropped in favour of phase
    /// one's. A run therefore returns a feasible schedule whenever one was reachable at all.
    fn search(&self, options: &SolveOptions) -> SolveOutcome {
        let graph = &self.internal.graph;
        if graph.objectives().is_empty() {
            // Nothing to optimize, so construction is already the whole search.
            return BacktrackingSolver::new()
                .solve(graph, &solver_options(options.time_limit, options));
        }
        if options.strategy == SolveStrategy::OptimizeOnly {
            return BranchAndBoundSolver::new()
                .solve(graph, &solver_options(options.time_limit, options));
        }

        let construction_limit = options
            .time_limit
            .map(|limit| limit / CONSTRUCTION_BUDGET_DIVISOR);
        let construction =
            BacktrackingSolver::new().solve(graph, &solver_options(construction_limit, options));

        // Objectives cannot make unsatisfiable hard constraints satisfiable, so an exhausted
        // construction search proves infeasibility for the whole run, not just for phase one.
        if construction.status == UnifierSolveStatus::Infeasible {
            return construction;
        }

        let construction_statistics = construction.statistics;
        let mut optimizer_options = solver_options(
            options
                .time_limit
                .map(|limit| limit.saturating_sub(construction_statistics.elapsed)),
            options,
        );
        if let Some(solution) = &construction.solution {
            let incumbent = SharedIncumbent::new();
            incumbent.offer(&solution.assignment, solution.score);
            optimizer_options.shared_incumbent = Some(incumbent);
        }

        let optimized = match options.strategy {
            SolveStrategy::ConstructThenPortfolio => {
                ParallelSolver::new().solve(graph, &optimizer_options)
            }
            _ => BranchAndBoundSolver::new().solve(graph, &optimizer_options),
        };
        let optimizer_statistics = optimized.statistics;
        let mut outcome = if optimized.solution.is_some() {
            optimized
        } else {
            // Die Konstruktion ist der Rückfall — aber die Auskunft *wie weit* steckt im
            // Optimierer: er hat auf ihrem Ergebnis weitergearbeitet und weiß als Einziger,
            // wie nah der Lauf am Ende kam. Ohne diese Zeile fiel sie hier lautlos heraus,
            // und der Aufrufer bekam „kein Plan" statt „so weit kam ich" (plan/51, B4).
            let reached = optimized.best_effort;
            let mut fallback = construction;
            if fallback.solution.is_none() && fallback.best_effort.is_none() {
                fallback.best_effort = reached;
            }
            fallback
        };
        outcome.statistics = SearchStatistics {
            nodes_expanded: construction_statistics.nodes_expanded
                + optimizer_statistics.nodes_expanded,
            elapsed: construction_statistics.elapsed + optimizer_statistics.elapsed,
        };
        outcome
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

    /// Checks a complete or partial [`Solution`] against every constraint without changing it
    /// (plan 31, E0.1). Equivalent to the union of the no-op probes
    /// [`CompiledProblem::evaluate_move`] would run per assignment, but in one call.
    ///
    /// Unknown activities become a blocking `ActivityDomain` conflict each, activities the
    /// solution omits a blocking `Unassigned` conflict each — a partial solution is reported,
    /// never rejected by a panic.
    pub fn check(&self, solution: &Solution) -> SolutionCheck {
        self.internal.check(solution)
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
    build_problem_internal(&problem, None)
}

fn build_problem_internal(
    problem: &SchedulingProblem,
    stability: Option<&StabilityBaseline>,
) -> Result<InternalCompiled, CompileError> {
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
                apply_resource_calendar(&mut builder, problem, interval_start, resource, duration);
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
            apply_participant_calendar(
                &mut builder,
                problem,
                interval_start,
                participant,
                duration,
            );
        }
        for requirement in activity.participant_requirements() {
            let candidates = resolve_participant_requirement(problem, requirement);
            if candidates.len() == 1 && requirement.exact_participant().is_some() {
                let participant = candidates[0];
                compiled.require_resource(participant_map[&participant], 1);
                apply_participant_calendar(
                    &mut builder,
                    problem,
                    interval_start,
                    participant,
                    duration,
                );
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

    // ParticipantChoiceGroup (plan 33.1): every activity of a group must pick the same
    // participant, so their candidate sets are intersected. An empty intersection can never be
    // satisfied and is refused here, with an explanation, instead of surfacing as an opaque
    // infeasibility later.
    let candidate_sets: BTreeMap<ActivityId, BTreeSet<ParticipantId>> = expanded_activities
        .iter()
        .map(|activity| {
            let candidates = activity
                .participant_requirements()
                .iter()
                .filter(|requirement| requirement.exact_participant().is_none())
                .flat_map(|requirement| resolve_participant_requirement(problem, requirement))
                .collect::<BTreeSet<_>>();
            (activity.id(), candidates)
        })
        .collect();
    for group in &problem.participant_choice_groups {
        let mut intersection: Option<BTreeSet<ParticipantId>> = None;
        for activity in &group.activities {
            let Some(candidates) = candidate_sets.get(activity) else {
                errors.push(format!(
                    "participant choice group references unknown activity {activity}"
                ));
                continue;
            };
            if candidates.is_empty() {
                errors.push(format!(
                    "activity {activity} in a participant choice group has no flexible participant \
                     requirement to agree on"
                ));
                continue;
            }
            intersection = Some(match intersection {
                None => candidates.clone(),
                Some(current) => current.intersection(candidates).copied().collect(),
            });
        }
        if let Some(intersection) = intersection
            && intersection.is_empty()
        {
            errors.push(format!(
                "participant choice group [{}] has no common candidate: the activities cannot \
                 agree on one participant",
                group
                    .activities
                    .iter()
                    .map(|activity| activity.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
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
            ActivityRelation::FixedOffset { offset } => {
                // `add_equal(v1, v2, k)` means `v1 = v2 + k`; the relation wants
                // `second.start = first.start + offset`, so `second` is `v1`.
                builder.add_equal(second_interval.start(), first_interval.start(), offset);
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
                duration_as_i64(duration_of(&expanded_activities, activity_id)),
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
                duration_as_i64(duration_of(&expanded_activities, activity_id)),
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

    // Each candidate of a participant choice group gets one equality constraint over its
    // presence variables, forcing every activity in the group to select it together (plan 33.1).
    for group in &problem.participant_choice_groups {
        let candidate_sets: Vec<BTreeSet<ParticipantId>> = group
            .activities
            .iter()
            .map(|activity| {
                selected_participants
                    .get(activity)
                    .into_iter()
                    .flatten()
                    .map(|(participant, _)| *participant)
                    .collect()
            })
            .collect();
        let mut intersection = candidate_sets.first().cloned().unwrap_or_default();
        for set in candidate_sets.iter().skip(1) {
            intersection = intersection.intersection(set).copied().collect();
        }
        for candidate in intersection {
            let presences: Vec<VariableId> = group
                .activities
                .iter()
                .filter_map(|activity| {
                    selected_participants.get(activity).and_then(|candidates| {
                        candidates
                            .iter()
                            .find(|(participant, _)| *participant == candidate)
                            .map(|(_, presence)| *presence)
                    })
                })
                .collect();
            if presences.len() == group.activities.len() {
                let mut scope = presences.clone();
                scope.extend(
                    group
                        .activities
                        .iter()
                        .filter_map(|activity| variables.get(activity).map(|(start, _)| *start)),
                );
                builder.add_constraint(Arc::new(ParticipantChoiceGroupConstraint::new(
                    presences, scope,
                )));
            }
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

    // MaximumDailyLoad: cap the occupied duration each entity accumulates per period bucket.
    if !problem.maximum_daily_loads.is_empty() {
        let ranges = load_bucket_ranges(problem, &expanded_activities);
        for rule in &problem.maximum_daily_loads {
            let tasks = load_tasks_for(
                problem,
                &expanded_activities,
                &variables,
                &fixed_resources,
                &fixed_participants,
                &selected_resources,
                &selected_participants,
                rule.entity,
            );
            if tasks.is_empty() {
                continue;
            }
            builder.add_maximum_bucket_load(
                tasks,
                ranges.clone(),
                i64::try_from(rule.limit).unwrap_or(i64::MAX),
            );
        }
    }

    // BucketLoadPattern: a group of activities must occupy one of the allowed teaching shapes per
    // bucket (plan 25, C1). Unlike the load caps, this constrains the *shape* of the occupied
    // intervals — consecutive blocks, so two adjacent periods count as one double block.
    if !problem.bucket_load_patterns.is_empty() {
        let ranges = load_bucket_ranges(problem, &expanded_activities);
        for pattern in &problem.bucket_load_patterns {
            if !pattern.is_active() {
                continue;
            }
            let tasks = pattern_tasks_for(&expanded_activities, &variables, &pattern.activities);
            if tasks.is_empty() {
                continue;
            }
            let allowed: Vec<Vec<i64>> = pattern
                .allowed
                .iter()
                .map(|blocks| {
                    blocks
                        .iter()
                        .map(|block| i64::try_from(*block).unwrap_or(i64::MAX))
                        .collect()
                })
                .collect();
            builder.add_bucket_block_pattern(tasks, ranges.clone(), allowed);
        }
    }

    // MinimumBreak: minimum distance between any two of an entity's deterministically assigned
    // activities. Multi-candidate pools are not paired (a distance may only apply when the entity
    // is selected for *both* activities, a condition the available primitives cannot express);
    // requirements that force a single candidate do take part.
    for rule in &problem.minimum_breaks {
        if rule.min_distance <= 0 {
            continue;
        }
        let activity_ids = entity_activity_ids(
            &expanded_activities,
            &fixed_resources,
            &fixed_participants,
            &selected_resources,
            &selected_participants,
            rule.entity,
        );
        for (index, &first) in activity_ids.iter().enumerate() {
            for &second in &activity_ids[index + 1..] {
                builder.add_minimum_distance(
                    variables[&first].0,
                    variables[&second].0,
                    rule.min_distance,
                );
            }
        }
    }

    if let Some(stability) = stability {
        apply_stability(
            &mut builder,
            stability,
            &expanded_activities,
            &variables,
            &selected_resources,
            &selected_participants,
        );
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

    // Problem-level soft goals: like score rules these are scored objectives, but they span the
    // whole problem instead of one activity.
    for goal in &problem.soft_goals {
        let objective = soft_goal_objective(
            problem,
            goal,
            &expanded_activities,
            &variables,
            &fixed_resources,
            &fixed_participants,
            &selected_resources,
            &selected_participants,
        );
        builder.add_scored_objective(&goal.category, score_level(goal.level), objective);
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
        participant_conflict_severity: problem.participant_conflict_policy.severity(),
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

    pub(crate) fn knows_activity(&self, activity: ActivityId) -> bool {
        self.variables.contains_key(&activity)
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

    pub(crate) fn check(&self, solution: &Solution) -> SolutionCheck {
        let mut hard_violations = Vec::new();
        let mut warnings = Vec::new();

        for assignment in &solution.assignments {
            if !self.knows_activity(assignment.activity) {
                hard_violations.push(Conflict {
                    severity: ConflictSeverity::Blocking,
                    constraint_name: "ActivityDomain".to_string(),
                    involved: vec![assignment.activity],
                    entity: Some(EntityRef::Activity(assignment.activity)),
                    message: "solution refers to an activity that is not part of the problem"
                        .to_string(),
                });
            }
        }

        let assigned = solution
            .assignments
            .iter()
            .map(|assignment| assignment.activity)
            .collect::<BTreeSet<_>>();
        hard_violations.extend(
            self.variables
                .keys()
                .filter(|activity| !assigned.contains(activity))
                .map(|&activity| Conflict {
                    severity: ConflictSeverity::Blocking,
                    constraint_name: "Unassigned".to_string(),
                    involved: vec![activity],
                    entity: Some(EntityRef::Activity(activity)),
                    message: format!("activity {activity} is not assigned"),
                }),
        );

        let assignment = self.assignment_map(solution);
        let proposed = assignment
            .iter()
            .map(|(&variable, &value)| (variable, value));
        for conflict in self.conflicts_from_violations(
            self.graph
                .check_incremental(&UnifierAssignment::new(), &proposed.collect::<Vec<_>>()),
        ) {
            match conflict.severity {
                ConflictSeverity::Blocking => hard_violations.push(conflict),
                ConflictSeverity::Advisory => warnings.push(conflict),
            }
        }

        SolutionCheck {
            is_feasible: hard_violations.is_empty(),
            explanations: hard_violations
                .iter()
                .chain(&warnings)
                .map(|conflict| conflict.message.clone())
                .collect(),
            hard_violations,
            warnings,
            score: self.score_for(&assignment),
            score_components: self.score_components(&assignment),
        }
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
                activity: if is_problem_level_objective(objective.as_ref()) {
                    None
                } else {
                    objective
                        .scope()
                        .iter()
                        .find_map(|variable| self.activity_by_variable.get(variable).copied())
                },
            })
            .collect()
    }

    /// Turns a search outcome into a [`SolveResult`] — and refuses to pass on a schedule that
    /// does not hold this crate's own hard constraints.
    ///
    /// The search reports what it believes about the assignment it returns, and that belief has
    /// been wrong: a portfolio once reported success on a schedule breaking 83 hard rules,
    /// because each layer trusted the one below it. This crate owns the model, so it is the one
    /// that has to look, and it looks on every run — one pass over the constraints, against a
    /// search that just spent seconds over millions of nodes.
    pub(crate) fn solve_result(&self, outcome: SolveOutcome) -> SolveResult {
        let mut statistics = SolveStatistics {
            nodes_expanded: outcome.statistics.nodes_expanded,
            elapsed_millis: outcome.statistics.elapsed.as_millis(),
            optimal: matches!(outcome.status, UnifierSolveStatus::Optimal),
        };
        let status = match outcome.status {
            UnifierSolveStatus::Optimal | UnifierSolveStatus::Feasible => SolveStatus::Feasible,
            UnifierSolveStatus::Infeasible => SolveStatus::Infeasible,
            UnifierSolveStatus::Aborted(reason) => SolveStatus::Aborted(abort_reason(reason)),
        };
        if let Some(solution) = &outcome.solution
            && !self.holds_hard_constraints(&solution.assignment)
        {
            // Whatever the search claimed about this assignment, it is not a schedule. Nothing
            // is known about the problem itself either, so neither `Feasible` nor `Infeasible`
            // would be honest, and an optimum that violates the rules is not an optimum.
            statistics.optimal = false;
            // Withheld as a schedule, kept as an answer: this is the closest complete
            // assignment the search produced, and the caller has a right to see how close.
            let best_effort = outcome
                .solution
                .map(|solution| self.public_solution(solution));
            return SolveResult {
                status: SolveStatus::Aborted(AbortReason::RejectedSolution),
                solution: None,
                best_effort,
                statistics,
            };
        }

        // `unifier` fills `best_effort` only while it has no solution of its own, so the two
        // cannot both arrive here — the `if let` below leaves it `None` whenever a schedule
        // exists.
        let best_effort = outcome
            .best_effort
            .map(|solution| self.public_solution(solution));
        let solution = outcome
            .solution
            .map(|solution| self.public_solution(solution));

        SolveResult {
            status,
            solution,
            best_effort,
            statistics,
        }
    }

    /// Whether `assignment` covers the model and breaks none of its hard constraints.
    ///
    /// Deliberately judged on the search's own assignment rather than on the [`Solution`] handed
    /// to callers: that projection and [`Self::assignment_map`] do not round-trip for a
    /// participant an activity both names directly and offers as a choice — the projection lists
    /// them either way, and mapping back then reads a chosen candidate where the search left
    /// none. Checking the projection would reject perfectly good schedules over that.
    fn holds_hard_constraints(&self, assignment: &UnifierAssignment) -> bool {
        self.graph
            .variables()
            .keys()
            .all(|variable| assignment.contains_key(variable))
            && ScoreCalculator
                .calculate_score(&self.graph, assignment)
                .is_feasible()
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
                        self.participant_conflict_severity,
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
    for load in &problem.maximum_daily_loads {
        match load.entity {
            EntityRef::Resource(resource) if !resource_ids.contains_key(&resource) => errors.push(
                format!("maximum daily load references unknown resource {resource}"),
            ),
            EntityRef::Participant(participant) if !participant_ids.contains(&participant) => {
                errors.push(format!(
                    "maximum daily load references unknown participant {participant}"
                ));
            }
            EntityRef::Activity(activity) => errors.push(format!(
                "maximum daily load must reference a resource or participant, not activity {activity}"
            )),
            _ => {}
        }
    }
    for minimum_break in &problem.minimum_breaks {
        match minimum_break.entity {
            EntityRef::Resource(resource) if !resource_ids.contains_key(&resource) => errors.push(
                format!("minimum break references unknown resource {resource}"),
            ),
            EntityRef::Participant(participant) if !participant_ids.contains(&participant) => {
                errors.push(format!(
                    "minimum break references unknown participant {participant}"
                ));
            }
            EntityRef::Activity(activity) => errors.push(format!(
                "minimum break must reference a resource or participant, not activity {activity}"
            )),
            _ => {}
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
    duration: i64,
) {
    if let Some(found) = problem
        .resources
        .iter()
        .find(|candidate| candidate.id() == resource)
    {
        let ranges = forbidden_start_ranges(
            found.unavailable_ranges(),
            found.blocked_windows(),
            duration,
        );
        if !ranges.is_empty() {
            builder.add_calendar(start, &ranges);
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
    duration: i64,
) {
    if let Some(found) = problem
        .participants
        .iter()
        .find(|candidate| candidate.id() == participant)
    {
        let ranges = forbidden_start_ranges(
            found.unavailable_ranges(),
            found.blocked_windows(),
            duration,
        );
        if !ranges.is_empty() {
            builder.add_calendar(start, &ranges);
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
    duration: i64,
) {
    if let Some(found) = problem
        .resources
        .iter()
        .find(|candidate| candidate.id() == resource)
    {
        let ranges = forbidden_start_ranges(
            found.unavailable_ranges(),
            found.blocked_windows(),
            duration,
        );
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
    duration: i64,
) {
    if let Some(found) = problem
        .participants
        .iter()
        .find(|candidate| candidate.id() == participant)
    {
        let ranges = forbidden_start_ranges(
            found.unavailable_ranges(),
            found.blocked_windows(),
            duration,
        );
        if !ranges.is_empty() {
            let forbidden = ranges
                .iter()
                .flat_map(|&(range_start, range_end)| range_start..=range_end);
            builder.add_optional(Arc::new(ForbiddenValues::new(start, forbidden)), presence);
        }
    }
}

/// The start slots an entity's unavailability forbids, as inclusive ranges: the ranges themselves
/// (start semantics) plus every start that would *occupy* a blocked window of this `duration`.
fn forbidden_start_ranges(
    unavailable: &[(i64, i64)],
    blocked: &[TimeWindow],
    duration: i64,
) -> Vec<(i64, i64)> {
    let mut ranges = unavailable.to_vec();
    ranges.extend(
        blocked
            .iter()
            .map(|window| window.blocked_starts(duration))
            .filter(|starts| !starts.is_empty())
            .map(|starts| (*starts.start(), *starts.end())),
    );
    ranges
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

/// Attaches the baseline-stability preferences and frozen domains of a stable repair (plan 31,
/// E0.2): each presence variable gets a `Strong` "stay at the baseline value" objective, and every
/// frozen activity has its time and selection variables narrowed to the baseline values.
fn apply_stability(
    builder: &mut ModelBuilder,
    stability: &StabilityBaseline,
    activities: &[Activity],
    variables: &BTreeMap<ActivityId, (VariableId, VariableId)>,
    selected_resources: &BTreeMap<ActivityId, Vec<(ResourceId, VariableId)>>,
    selected_participants: &BTreeMap<ActivityId, Vec<(ParticipantId, VariableId)>>,
) {
    for (activity, baseline) in &stability.assignments {
        let Some(&(start, end)) = variables.get(activity) else {
            continue;
        };
        if let Some(candidates) = selected_resources.get(activity) {
            for &(resource, presence) in candidates {
                builder.add_scored_objective(
                    "stability",
                    score_level(ScoreLevel::Strong),
                    Arc::new(PresenceObjective::new(
                        presence,
                        i64::from(baseline.resources.contains(&resource)),
                        stability.assignment_change_penalty,
                    )),
                );
            }
        }
        if let Some(candidates) = selected_participants.get(activity) {
            for &(participant, presence) in candidates {
                builder.add_scored_objective(
                    "stability",
                    score_level(ScoreLevel::Strong),
                    Arc::new(PresenceObjective::new(
                        presence,
                        i64::from(baseline.participants.contains(&participant)),
                        stability.assignment_change_penalty,
                    )),
                );
            }
        }
        if !stability.frozen.contains(activity) {
            continue;
        }
        builder.add_allowed_values(start, [baseline.start]);
        builder.add_allowed_values(
            end,
            [baseline
                .start
                .saturating_add(duration_as_i64(duration_of(activities, *activity)))],
        );
        for &(resource, presence) in selected_resources.get(activity).into_iter().flatten() {
            builder.add_allowed_values(
                presence,
                [i64::from(baseline.resources.contains(&resource))],
            );
        }
        for &(participant, presence) in selected_participants.get(activity).into_iter().flatten() {
            builder.add_allowed_values(
                presence,
                [i64::from(baseline.participants.contains(&participant))],
            );
        }
    }
}

/// Penalises a presence variable that leaves its baseline value (plan 31, E0.2) — the
/// assignment-side analogue of [`RuleObjective`]'s `KeepStart`.
#[derive(Debug, Clone)]
struct PresenceObjective {
    variable: VariableId,
    target: i64,
    weight: i64,
    scope: [VariableId; 1],
}

impl PresenceObjective {
    fn new(variable: VariableId, target: i64, weight: i64) -> Self {
        Self {
            variable,
            target,
            weight,
            scope: [variable],
        }
    }
}

impl Objective for PresenceObjective {
    fn name(&self) -> &str {
        "StabilityAssignment"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        if assignment.get(&self.variable) == Some(&self.target) {
            0
        } else {
            self.weight.saturating_neg()
        }
    }

    fn optimistic_bound(&self, domains: &HashMap<VariableId, Domain>) -> i64 {
        if domains
            .get(&self.variable)
            .is_some_and(|domain| domain.contains(self.target))
        {
            0
        } else {
            self.weight.saturating_neg()
        }
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

    /// Reports the overload that is already certain, instead of leaving every one of them to
    /// the leaf check.
    ///
    /// This used to return `Success` unconditionally — the constraint could not fail a search
    /// node, only a complete assignment. That is what made a step needing two instances of one
    /// resource type pathological: the two requirement slots are two tasks on the *same* start
    /// variable, the search fixes their presences near the root, and if both land on the same
    /// capacity-1 instance, nothing says so until every start time underneath has been tried
    /// (plan/19 in the Avilo repository measured 1,116,987 nodes for a problem with a single
    /// activity, two resources and a ten-hour window).
    ///
    /// The rule reports only what holds regardless of time: **tasks sharing a start variable
    /// belong to one activity and therefore always overlap**, so their demands can be added up
    /// without knowing when that activity runs. Zero-duration tasks are left out — they occupy
    /// no interval, and [`Self::overloaded`] does not count them either, because their end
    /// event cancels their start event at the same instant.
    ///
    /// Deliberately no second rule for "everything in scope is decided, so run the full
    /// overload check here": that costs a pass over the whole scope on every propagation, and
    /// a busy resource's scope is every activity that may use it. Measured on
    /// `tests/solver_scale.rs`, it cost the tree search its 24-activity instance inside the
    /// five-second budget — a check that arrives one node earlier is not worth slowing every
    /// node down. The leaf check already covers that case.
    ///
    /// # Complexity
    /// Time: O(T^2) worst case over this resource's tasks T, but T is the number of tasks that
    /// may use *one* resource and the inner scan stops at the first group hit; no allocation.
    fn propagate(&self, domains: &mut TrailedDomains) -> PropagationResult {
        let mut demand_per_start: Vec<(VariableId, i64)> = Vec::new();
        for task in &self.tasks {
            if task.duration <= 0 {
                continue;
            }
            let surely_present = match task.presence {
                None => true,
                Some(presence) => domains
                    .get(&presence)
                    .is_some_and(|domain| domain.len() == 1 && domain.min() == Some(1)),
            };
            if !surely_present {
                continue;
            }
            let demand = i64::from(task.demand);
            match demand_per_start
                .iter_mut()
                .find(|(start, _)| *start == task.start)
            {
                Some((_, total)) => {
                    *total += demand;
                    if *total > i64::from(self.capacity) {
                        return PropagationResult::Conflict;
                    }
                }
                None => {
                    if demand > i64::from(self.capacity) {
                        return PropagationResult::Conflict;
                    }
                    demand_per_start.push((task.start, demand));
                }
            }
        }
        PropagationResult::Success { changed: false }
    }

    /// Mirrors [`Self::propagate`]'s rule so the bound Branch & Bound derives from
    /// `optimistic_score` sees the same certain overload. The trait default would fall back to
    /// [`Self::is_satisfied`], which reads only the assignment and therefore answers "not
    /// violated yet" for exactly the case this constraint can already rule out.
    fn is_satisfiable(
        &self,
        domains: &HashMap<VariableId, Domain>,
        assignment: &HashMap<VariableId, i64>,
    ) -> bool {
        let mut demand_per_start: Vec<(VariableId, i64)> = Vec::new();
        for task in &self.tasks {
            if task.duration <= 0 {
                continue;
            }
            let surely_present = match task.presence {
                None => true,
                Some(presence) => {
                    assignment.get(&presence) == Some(&1)
                        || domains
                            .get(&presence)
                            .is_some_and(|domain| domain.len() == 1 && domain.min() == Some(1))
                }
            };
            if !surely_present {
                continue;
            }
            let demand = i64::from(task.demand);
            match demand_per_start
                .iter_mut()
                .find(|(start, _)| *start == task.start)
            {
                Some((_, total)) => {
                    *total += demand;
                    if *total > i64::from(self.capacity) {
                        return false;
                    }
                }
                None => {
                    if demand > i64::from(self.capacity) {
                        return false;
                    }
                    demand_per_start.push((task.start, demand));
                }
            }
        }
        self.is_satisfied(assignment)
    }
}

/// One participant's presence variables, across the activities of a [`ParticipantChoiceGroup`],
/// must all carry the same value (plan 33.1). Named so a violation reaches `check` and
/// `evaluate_changes` under the stable key `ParticipantChoiceGroup` instead of a generic `Equal`.
#[derive(Debug, Clone)]
struct ParticipantChoiceGroupConstraint {
    presences: Vec<VariableId>,
    scope: Vec<VariableId>,
}

impl ParticipantChoiceGroupConstraint {
    /// `presences` are the candidate's presence variables; `scope` additionally carries the
    /// member activities' time variables so a `check_incremental` after any move re-validates the
    /// group's agreement instead of skipping a constraint whose presence variables did not move.
    fn new(presences: Vec<VariableId>, scope: Vec<VariableId>) -> Self {
        let mut presences = presences;
        presences.sort_unstable();
        presences.dedup();
        let mut scope = scope;
        scope.sort_unstable();
        scope.dedup();
        Self { presences, scope }
    }
}

impl Constraint for ParticipantChoiceGroupConstraint {
    fn name(&self) -> &str {
        "ParticipantChoiceGroup"
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn is_satisfied(&self, assignment: &HashMap<VariableId, i64>) -> bool {
        let mut values = self.presences.iter().filter_map(|var| assignment.get(var));
        let Some(&first) = values.next() else {
            return true;
        };
        values.all(|&value| value == first)
    }

    fn explain(&self, assignment: &UnifierAssignment) -> Option<Explanation> {
        (!self.is_satisfied(assignment)).then(|| Explanation {
            constraint_name: "ParticipantChoiceGroup",
            involved: self.presences.clone(),
            message: "the group's activities must choose the same participant".to_string(),
        })
    }

    fn propagate(&self, domains: &mut TrailedDomains) -> PropagationResult {
        // Presence variables are 0/1. Equality means every domain must keep the common
        // intersection: if one is fixed to 1 all others are too, and vice versa.
        let mut all_zero = true;
        let mut all_one = true;
        for &var in &self.presences {
            if let Some(domain) = domains.get(&var) {
                all_zero &= domain.contains(0);
                all_one &= domain.contains(1);
            }
        }
        let mut changed = false;
        for &var in &self.presences {
            if let Some(domain_changed) = domains.mutate(var, |domain| {
                let mut modified = false;
                if !all_zero {
                    modified |= domain.remove(0);
                }
                if !all_one {
                    modified |= domain.remove(1);
                }
                modified
            }) {
                changed |= domain_changed;
            }
            if domains.get(&var).is_some_and(|domain| domain.is_empty()) {
                return PropagationResult::Conflict;
            }
        }
        PropagationResult::Success { changed }
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

fn abort_reason(reason: UnifierAbortReason) -> AbortReason {
    match reason {
        UnifierAbortReason::Cancelled => AbortReason::Cancelled,
        UnifierAbortReason::Timeout => AbortReason::Timeout,
        UnifierAbortReason::NodeLimit => AbortReason::NodeLimit,
        UnifierAbortReason::LocalOptimum => AbortReason::LocalOptimum,
    }
}

fn duration_as_i64(duration: u64) -> i64 {
    i64::try_from(duration).unwrap_or(i64::MAX)
}

/// Collects the load tasks an entity accumulates, mirroring how the capacity constraints gather
/// their tasks: fixed requirements unconditionally, flexible (alternative) candidates gated by
/// their presence variable. Resource tasks weigh their requirement's units; participant tasks
/// weigh 1.
#[allow(clippy::too_many_arguments)]
fn load_tasks_for(
    problem: &SchedulingProblem,
    activities: &[Activity],
    variables: &BTreeMap<ActivityId, (VariableId, VariableId)>,
    fixed_resources: &BTreeMap<ActivityId, Vec<ResourceId>>,
    fixed_participants: &BTreeMap<ActivityId, Vec<ParticipantId>>,
    selected_resources: &BTreeMap<ActivityId, Vec<(ResourceId, VariableId)>>,
    selected_participants: &BTreeMap<ActivityId, Vec<(ParticipantId, VariableId)>>,
    entity: EntityRef,
) -> Vec<BucketedTask> {
    let mut tasks = Vec::new();
    for activity in activities {
        let Some(&(start, _)) = variables.get(&activity.id()) else {
            continue;
        };
        let duration = duration_as_i64(activity.duration());
        match entity {
            EntityRef::Resource(resource) => {
                let units = i64::from(resource_units(problem, activity, resource));
                if fixed_resources
                    .get(&activity.id())
                    .is_some_and(|resources| resources.contains(&resource))
                {
                    tasks.push(BucketedTask::new(start, duration, units));
                } else if let Some(presence) =
                    selected_resources
                        .get(&activity.id())
                        .and_then(|candidates| {
                            candidates
                                .iter()
                                .find(|(candidate, _)| *candidate == resource)
                                .map(|(_, presence)| *presence)
                        })
                {
                    tasks.push(BucketedTask::new(start, duration, units).with_presence(presence));
                }
            }
            EntityRef::Participant(participant) => {
                let fixed = activity.participants().contains(&participant)
                    || fixed_participants
                        .get(&activity.id())
                        .is_some_and(|participants| participants.contains(&participant));
                if fixed {
                    tasks.push(BucketedTask::new(start, duration, 1));
                } else if let Some(presence) =
                    selected_participants
                        .get(&activity.id())
                        .and_then(|candidates| {
                            candidates
                                .iter()
                                .find(|(candidate, _)| *candidate == participant)
                                .map(|(_, presence)| *presence)
                        })
                {
                    tasks.push(BucketedTask::new(start, duration, 1).with_presence(presence));
                }
            }
            EntityRef::Activity(_) => {}
        }
    }
    tasks
}

/// Returns the units `activity` demands of `resource` (1 if no matching requirement is found).
fn resource_units(problem: &SchedulingProblem, activity: &Activity, resource: ResourceId) -> u32 {
    activity
        .requirements()
        .iter()
        .find(|requirement| resolve_requirement(problem, requirement).contains(&resource))
        .map_or(1, ResourceRequirement::units)
}

/// Returns the activities for which `entity` is deterministically assigned: either fixed outright
/// (fixed resource/participant), or bound through a requirement whose candidate set the engine
/// resolved to exactly that one entity — `exactly_one` then forces its presence, so the assignment
/// holds for every solution. Multi-candidate pools are deliberately excluded: their distance would
/// have to be conditional on the entity being selected for *both* activities, which
/// [`unifier::MinimumDistance`] cannot express.
fn entity_activity_ids(
    activities: &[Activity],
    fixed_resources: &BTreeMap<ActivityId, Vec<ResourceId>>,
    fixed_participants: &BTreeMap<ActivityId, Vec<ParticipantId>>,
    selected_resources: &BTreeMap<ActivityId, Vec<(ResourceId, VariableId)>>,
    selected_participants: &BTreeMap<ActivityId, Vec<(ParticipantId, VariableId)>>,
    entity: EntityRef,
) -> Vec<ActivityId> {
    activities
        .iter()
        .filter(|activity| match entity {
            EntityRef::Resource(resource) => {
                fixed_resources
                    .get(&activity.id())
                    .is_some_and(|resources| resources.contains(&resource))
                    || guaranteed_candidate(selected_resources.get(&activity.id()), resource)
            }
            EntityRef::Participant(participant) => {
                activity.participants().contains(&participant)
                    || fixed_participants
                        .get(&activity.id())
                        .is_some_and(|participants| participants.contains(&participant))
                    || guaranteed_candidate(selected_participants.get(&activity.id()), participant)
            }
            EntityRef::Activity(_) => false,
        })
        .map(Activity::id)
        .collect()
}

/// True when `candidates` is a single-entry selection list for `entity`: the requirement's
/// exactly-one over that sole candidate forces the entity to be selected.
fn guaranteed_candidate<T: Copy + PartialEq>(
    candidates: Option<&Vec<(T, VariableId)>>,
    entity: T,
) -> bool {
    candidates.is_some_and(|candidates| candidates.len() == 1 && candidates[0].0 == entity)
}

/// Builds the half-open value ranges one load constraint is checked against. Delegates to
/// [`bucket_windows`] so the solver and every caller reasoning about blocks share one notion of
/// a bucket (plan 25, C1).
fn load_bucket_ranges(problem: &SchedulingProblem, activities: &[Activity]) -> Vec<BucketRange> {
    let (Some(min_value), Some(max_value)) = (
        activities.iter().map(|a| a.allowed_window().start).min(),
        activities.iter().map(|a| a.allowed_window().end).max(),
    ) else {
        return Vec::new();
    };
    bucket_windows(problem.schedule_template.as_ref(), min_value, max_value)
        .into_iter()
        .map(|entry| BucketRange::new(entry.window.start, entry.window.end, entry.bucket))
        .collect()
}

/// Collects the tasks a block-pattern rule constrains: one task per listed activity, always
/// present (an activity's slot is decided in every solution, so the pattern can judge its shape).
///
/// Listed ids that are not part of the expanded model are skipped, mirroring [`load_tasks_for`]; a
/// caller building the rule from the same activity list it hands to the problem therefore never
/// loses an activity silently.
fn pattern_tasks_for(
    activities: &[Activity],
    variables: &BTreeMap<ActivityId, (VariableId, VariableId)>,
    activity_ids: &[ActivityId],
) -> Vec<BucketedTask> {
    activity_ids
        .iter()
        .filter_map(|id| {
            let &(start, _) = variables.get(id)?;
            Some(BucketedTask::new(
                start,
                duration_as_i64(duration_of(activities, *id)),
                1,
            ))
        })
        .collect()
}

fn duration_of(activities: &[Activity], id: ActivityId) -> u64 {
    activities
        .iter()
        .find(|activity| activity.id() == id)
        .map_or(0, Activity::duration)
}

/// [`Objective::name`] shared by every problem-level soft goal. Besides serving as the fallback
/// category it marks an objective as problem-level, which [`InternalCompiled::score_components`]
/// uses to report `activity == None` for it.
const SOFT_GOAL_OBJECTIVE_NAME: &str = "ProblemSoftGoal";

/// Builds the single [`Objective`] implementing one problem-level [`SoftGoal`].
///
/// All four kinds are answered by the same primitives the hard rules use: the period buckets come
/// from [`load_bucket_ranges`], participant tasks from [`load_tasks_for`], and resource selections
/// from the compiled presence variables. The objective sits behind
/// [`unifier::dsl::ModelBuilder::add_scored_objective`] so `category`/`level` are taken from the
/// goal.
#[allow(clippy::too_many_arguments)]
fn soft_goal_objective(
    problem: &SchedulingProblem,
    goal: &SoftGoal,
    activities: &[Activity],
    variables: &BTreeMap<ActivityId, (VariableId, VariableId)>,
    fixed_resources: &BTreeMap<ActivityId, Vec<ResourceId>>,
    fixed_participants: &BTreeMap<ActivityId, Vec<ParticipantId>>,
    selected_resources: &BTreeMap<ActivityId, Vec<(ResourceId, VariableId)>>,
    selected_participants: &BTreeMap<ActivityId, Vec<(ParticipantId, VariableId)>>,
) -> Arc<dyn Objective> {
    match goal.kind {
        SoftGoalKind::MinimizeParticipantIdle => {
            let ranges = load_bucket_ranges(problem, activities);
            let tracks = problem
                .participants
                .iter()
                .map(|participant| {
                    load_tasks_for(
                        problem,
                        activities,
                        variables,
                        fixed_resources,
                        fixed_participants,
                        selected_resources,
                        selected_participants,
                        EntityRef::Participant(participant.id()),
                    )
                })
                .collect();
            Arc::new(IdleObjective::new(goal.weight, tracks, ranges))
        }
        SoftGoalKind::MinimizeGroupIdle => {
            let ranges = load_bucket_ranges(problem, activities);
            let memberships = group_memberships(problem);
            let tracks = problem
                .participant_groups
                .iter()
                .map(|group| {
                    let members = group_member_participants(group.id, &memberships);
                    activities
                        .iter()
                        .filter(|activity| {
                            members.iter().any(|member| {
                                participant_is_assigned(
                                    activity,
                                    *member,
                                    fixed_participants,
                                    selected_participants,
                                )
                            })
                        })
                        .filter_map(|activity| {
                            let (start, _) = variables.get(&activity.id()).copied()?;
                            Some(BucketedTask::new(
                                start,
                                duration_as_i64(activity.duration()),
                                1,
                            ))
                        })
                        .collect()
                })
                .collect();
            Arc::new(IdleObjective::new(goal.weight, tracks, ranges))
        }
        SoftGoalKind::RoomStability => {
            let groups = activities_by_name(activities, variables, |activity, _| {
                let mut entries: Vec<(ResourceId, Option<VariableId>)> = Vec::new();
                if let Some(resources) = fixed_resources.get(&activity.id()) {
                    entries.extend(resources.iter().map(|&resource| (resource, None)));
                }
                if let Some(candidates) = selected_resources.get(&activity.id()) {
                    entries.extend(
                        candidates
                            .iter()
                            .map(|&(resource, presence)| (resource, Some(presence))),
                    );
                }
                entries
            });
            Arc::new(RoomStabilityObjective::new(goal.weight, groups))
        }
        SoftGoalKind::SpreadActivityOverDays => {
            let ranges = load_bucket_ranges(problem, activities);
            let groups = activities_by_name(activities, variables, |_, start| start);
            Arc::new(SpreadObjective::new(goal.weight, groups, ranges))
        }
    }
}

/// Groups activities that share a name, in name order, applying `payload` to each occurrence
/// together with its start variable. Activities without a compiled interval are skipped.
fn activities_by_name<T>(
    activities: &[Activity],
    variables: &BTreeMap<ActivityId, (VariableId, VariableId)>,
    payload: impl Fn(&Activity, VariableId) -> T,
) -> Vec<Vec<T>> {
    let mut groups: BTreeMap<String, Vec<T>> = BTreeMap::new();
    for activity in activities {
        let Some(&(start, _)) = variables.get(&activity.id()) else {
            continue;
        };
        groups
            .entry(activity.name().to_string())
            .or_default()
            .push(payload(activity, start));
    }
    groups.into_values().collect()
}

/// Penalty of `amount` scaled by `weight`, expressed as a soft-score contribution (never positive
/// for a non-negative weight, since solvers maximise the soft score).
fn soft_penalty(weight: i64, amount: i64) -> i64 {
    amount.saturating_mul(weight).saturating_neg()
}

/// Optimistic (never-underestimating) bound for a [`soft_penalty`] contribution. For a
/// non-negative weight the contribution is at most `0`; a negative weight would make the
/// contribution positive, so no finite bound below `i64::MAX` is sound and pruning is disabled
/// instead.
fn soft_optimistic_bound(weight: i64) -> i64 {
    if weight < 0 { i64::MAX } else { 0 }
}

/// The bucket a value falls in, per the half-open [`BucketRange`]s produced by
/// [`load_bucket_ranges`].
fn bucket_of(ranges: &[BucketRange], value: i64) -> Option<usize> {
    ranges
        .iter()
        .find(|range| range.start <= value && value < range.end)
        .map(|range| range.bucket)
}

/// Total free time between consecutive occupied intervals of one track inside each bucket. Only
/// present tasks with an assigned start contribute; intervals overlapping a bucket boundary are
/// attributed to their start's bucket, mirroring how [`load_bucket_ranges`] segments a horizon.
fn track_idle(
    track: &[BucketedTask],
    ranges: &[BucketRange],
    assignment: &HashMap<VariableId, i64>,
) -> i64 {
    let mut per_bucket: BTreeMap<usize, Vec<(i64, i64)>> = BTreeMap::new();
    for task in track {
        if let Some(presence) = task.presence
            && assignment.get(&presence) != Some(&1)
        {
            continue;
        }
        let Some(&start) = assignment.get(&task.start) else {
            continue;
        };
        let Some(bucket) = bucket_of(ranges, start) else {
            continue;
        };
        per_bucket
            .entry(bucket)
            .or_default()
            .push((start, start.saturating_add(task.duration)));
    }
    let mut idle = 0i64;
    for intervals in per_bucket.values_mut() {
        intervals.sort_unstable();
        let mut cursor = intervals[0].1;
        for &(start, end) in &intervals[1..] {
            if start > cursor {
                idle = idle.saturating_add(start - cursor);
            }
            cursor = cursor.max(end);
        }
    }
    idle
}

/// Minimises free time between a participant's (or a group's aggregated) assignments.
#[derive(Debug, Clone)]
struct IdleObjective {
    weight: i64,
    /// One track per participant or group; each entry is an occupied interval.
    tracks: Vec<Vec<BucketedTask>>,
    ranges: Vec<BucketRange>,
    scope: Vec<VariableId>,
}

impl IdleObjective {
    fn new(weight: i64, tracks: Vec<Vec<BucketedTask>>, ranges: Vec<BucketRange>) -> Self {
        let mut scope = Vec::new();
        for track in &tracks {
            for task in track {
                scope.push(task.start);
                if let Some(presence) = task.presence {
                    scope.push(presence);
                }
            }
        }
        scope.sort_unstable();
        scope.dedup();
        Self {
            weight,
            tracks,
            ranges,
            scope,
        }
    }
}

impl Objective for IdleObjective {
    fn name(&self) -> &str {
        SOFT_GOAL_OBJECTIVE_NAME
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let mut idle = 0i64;
        for track in &self.tracks {
            idle = idle.saturating_add(track_idle(track, &self.ranges, assignment));
        }
        soft_penalty(self.weight, idle)
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        soft_optimistic_bound(self.weight)
    }
}

/// Minimises the number of distinct resources used by occurrences sharing an activity name, over
/// and above the single resource such a group can be reduced to.
///
/// Outer vec: one entry per activity name. Middle vec: one entry per occurrence. Inner vec: the
/// `(resource, selection variable)` pairs each occurrence may use.
type ResourceSelections = Vec<Vec<Vec<(ResourceId, Option<VariableId>)>>>;

#[derive(Debug, Clone)]
struct RoomStabilityObjective {
    weight: i64,
    /// Group -> occurrence -> `(resource, selection variable)`. A `None` selection variable marks
    /// a resource the activity requires unconditionally.
    groups: ResourceSelections,
    scope: Vec<VariableId>,
}

impl RoomStabilityObjective {
    fn new(weight: i64, groups: ResourceSelections) -> Self {
        let mut scope = Vec::new();
        for group in &groups {
            for occurrence in group {
                for &(_, presence) in occurrence {
                    if let Some(presence) = presence {
                        scope.push(presence);
                    }
                }
            }
        }
        scope.sort_unstable();
        scope.dedup();
        Self {
            weight,
            groups,
            scope,
        }
    }
}

impl Objective for RoomStabilityObjective {
    fn name(&self) -> &str {
        SOFT_GOAL_OBJECTIVE_NAME
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let mut instability = 0i64;
        for group in &self.groups {
            let mut used = BTreeSet::new();
            for occurrence in group {
                for &(resource, presence) in occurrence {
                    if presence.is_none_or(|presence| assignment.get(&presence) == Some(&1)) {
                        used.insert(resource);
                    }
                }
            }
            instability = instability
                .saturating_add(i64::try_from(used.len().saturating_sub(1)).unwrap_or(i64::MAX));
        }
        soft_penalty(self.weight, instability)
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        soft_optimistic_bound(self.weight)
    }
}

/// Penalises occurrences of the same activity name that share a period bucket, so the solver
/// spreads them across buckets instead of clustering them.
#[derive(Debug, Clone)]
struct SpreadObjective {
    weight: i64,
    /// One group per activity name, holding each occurrence's start variable.
    groups: Vec<Vec<VariableId>>,
    ranges: Vec<BucketRange>,
    scope: Vec<VariableId>,
}

impl SpreadObjective {
    fn new(weight: i64, groups: Vec<Vec<VariableId>>, ranges: Vec<BucketRange>) -> Self {
        let mut scope: Vec<VariableId> = groups.iter().flatten().copied().collect();
        scope.sort_unstable();
        scope.dedup();
        Self {
            weight,
            groups,
            ranges,
            scope,
        }
    }
}

impl Objective for SpreadObjective {
    fn name(&self) -> &str {
        SOFT_GOAL_OBJECTIVE_NAME
    }

    fn scope(&self) -> &[VariableId] {
        &self.scope
    }

    fn evaluate(&self, assignment: &HashMap<VariableId, i64>) -> i64 {
        let mut collisions = 0i64;
        for group in &self.groups {
            let mut per_bucket: BTreeMap<usize, i64> = BTreeMap::new();
            for &start_var in group {
                let Some(&start) = assignment.get(&start_var) else {
                    continue;
                };
                let Some(bucket) = bucket_of(&self.ranges, start) else {
                    continue;
                };
                let count = per_bucket.entry(bucket).or_insert(0);
                *count = count.saturating_add(1);
            }
            for count in per_bucket.values() {
                collisions =
                    collisions.saturating_add(count.saturating_mul(count.saturating_sub(1)) / 2);
            }
        }
        soft_penalty(self.weight, collisions)
    }

    fn optimistic_bound(&self, _domains: &HashMap<VariableId, Domain>) -> i64 {
        soft_optimistic_bound(self.weight)
    }
}

/// True when `objective` is one of the problem-level soft goals built by [`soft_goal_objective`].
fn is_problem_level_objective(objective: &dyn Objective) -> bool {
    objective.name() == SOFT_GOAL_OBJECTIVE_NAME
}

/// Participant memberships grouped by their group, mirroring how [`expand_group_participants`]
/// reads them.
fn group_memberships(
    problem: &SchedulingProblem,
) -> BTreeMap<ParticipantGroupId, Vec<GroupMember>> {
    let mut memberships: BTreeMap<ParticipantGroupId, Vec<GroupMember>> = BTreeMap::new();
    for membership in &problem.group_memberships {
        memberships
            .entry(membership.group)
            .or_default()
            .push(membership.member);
    }
    memberships
}

/// Every participant reachable from `group`, following subgroup memberships. Cycles are tolerated
/// (each group is visited once) since soft goals are built after input validation.
fn group_member_participants(
    group: ParticipantGroupId,
    memberships: &BTreeMap<ParticipantGroupId, Vec<GroupMember>>,
) -> BTreeSet<ParticipantId> {
    let mut participants = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    collect_group_participants(group, memberships, &mut visiting, &mut participants);
    participants
}

fn collect_group_participants(
    group: ParticipantGroupId,
    memberships: &BTreeMap<ParticipantGroupId, Vec<GroupMember>>,
    visiting: &mut BTreeSet<ParticipantGroupId>,
    participants: &mut BTreeSet<ParticipantId>,
) {
    if !visiting.insert(group) {
        return;
    }
    for member in memberships.get(&group).into_iter().flatten() {
        match member {
            GroupMember::Participant(participant) => {
                participants.insert(*participant);
            }
            GroupMember::Group(subgroup) => {
                collect_group_participants(*subgroup, memberships, visiting, participants);
            }
        }
    }
    visiting.remove(&group);
}

/// Whether `participant` is deterministically assigned to `activity`: fixed outright, or bound
/// through a single-candidate requirement. Mirrors [`entity_activity_ids`]' participant branch.
fn participant_is_assigned(
    activity: &Activity,
    participant: ParticipantId,
    fixed_participants: &BTreeMap<ActivityId, Vec<ParticipantId>>,
    selected_participants: &BTreeMap<ActivityId, Vec<(ParticipantId, VariableId)>>,
) -> bool {
    activity.participants().contains(&participant)
        || fixed_participants
            .get(&activity.id())
            .is_some_and(|participants| participants.contains(&participant))
        || guaranteed_candidate(selected_participants.get(&activity.id()), participant)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard in `InternalCompiled::solve_result`: an assignment that breaks a hard
    /// constraint must not be handed on as a schedule, whatever the search claims about it.
    ///
    /// The problem itself is satisfiable — two four-unit activities fit one after the other in a
    /// ten-unit window — so a rejection here can only come from the returned assignment, not from
    /// the model.
    #[test]
    fn an_assignment_violating_hard_constraints_is_withheld() {
        let room = Resource::new(ResourceId(1), "Room", 1);
        let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 10), 4)
            .with_requirement(ResourceRequirement::new(room.id(), 1));
        let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 10), 4)
            .with_requirement(ResourceRequirement::new(room.id(), 1));
        let compiled = compile(&SchedulingProblem::new(
            vec![room],
            vec![],
            vec![first, second],
        ))
        .expect("problem compiles");

        // Both activities start at 0, so both hold the one unary resource at once. No real search
        // returns this; a broken portfolio did, on 83 rules at a time, and reported success.
        let mut assignment: HashMap<VariableId, i64> = HashMap::new();
        for &(start, end) in compiled.internal.variables.values() {
            assignment.insert(start, 0);
            assignment.insert(end, 4);
        }
        let claimed = unifier::Solution {
            assignment,
            score: unifier::HardSoftScore::feasible(0),
        };

        // Pin down what makes this assignment invalid, so the rejection below cannot pass for
        // some unrelated reason.
        let check = compiled
            .internal
            .check(&compiled.internal.public_solution(claimed.clone()));
        assert!(
            check
                .hard_violations
                .iter()
                .any(|conflict| conflict.constraint_name == "NoOverlap"),
            "expected the double-booked resource to show up, got {:?}",
            check.hard_violations
        );
        assert!(
            !compiled
                .internal
                .holds_hard_constraints(&claimed.assignment),
            "and the guard has to see it on the search's own assignment, which is what it reads"
        );

        let outcome = SolveOutcome {
            status: UnifierSolveStatus::Feasible,
            solution: Some(claimed),
            statistics: SearchStatistics::default(),
            bound: None,
            best_effort: None,
        };

        let result = compiled.internal.solve_result(outcome);

        assert_eq!(
            result.status,
            SolveStatus::Aborted(AbortReason::RejectedSolution)
        );
        assert!(
            result.solution.is_none(),
            "an assignment that breaks the rules must not leave as a schedule"
        );
        assert!(
            !result.statistics.optimal,
            "an optimum that violates hard constraints is not an optimum"
        );
    }

    /// The guard stays out of the way of a schedule that does hold: same problem, an arrangement
    /// that actually works, passed through untouched.
    #[test]
    fn a_valid_assignment_passes_the_guard() {
        let room = Resource::new(ResourceId(1), "Room", 1);
        let first = Activity::new(ActivityId(1), "first", TimeWindow::new(0, 10), 4)
            .with_requirement(ResourceRequirement::new(room.id(), 1));
        let second = Activity::new(ActivityId(2), "second", TimeWindow::new(0, 10), 4)
            .with_requirement(ResourceRequirement::new(room.id(), 1));
        let compiled = compile(&SchedulingProblem::new(
            vec![room],
            vec![],
            vec![first, second],
        ))
        .expect("problem compiles");

        let result = compiled.solve();

        assert_eq!(result.status, SolveStatus::Feasible);
        assert!(result.solution.is_some());
    }
}
