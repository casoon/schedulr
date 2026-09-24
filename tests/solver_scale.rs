//! Regression guards on how large an instance the solver still schedules.
//!
//! These exist because the search strength was, for a long time, something people remembered
//! rather than something anything checked: a change that quietly made the solver weaker only
//! showed up the next time somebody ran a demo by hand. Each test below fixes one instance
//! size that the solver handles comfortably today and fails if that stops being true.
//!
//! There are two kinds of guard here, because no single one covers both failure modes.
//!
//! **The scale guards** solve growing instances with the default strategy under a time budget.
//! A run spends its whole budget by design — optimization continues until the limit — so the
//! budget is not headroom, it is what the test costs; what decides red or green is the
//! construction phase, which gets half of it and needs milliseconds here. These catch a
//! categorical loss: the solver no longer schedules a size it used to.
//!
//! **The node-count guard** solves the deterministic single-solver path and pins how much tree
//! it had to walk. Node counts do not depend on machine speed or load, so this one can be tight
//! without ever flaking — which is what makes it the sensitive half.
//!
//! **What they measured, honestly.** These were calibrated by breaking the solver on purpose
//! and checking which guard noticed:
//!
//! - Remove the `dom/wdeg` variable ordering: the node-count guard fails hard — 4k nodes
//!   becomes 700k without a schedule at all. Caught.
//! - Remove the least-constraining-value ordering: nothing moves on these instances. Missed.
//! - Make each search node propagate the whole graph again (a sixfold throughput loss): the
//!   scale guards still pass. Missed.
//!
//! So: they catch losing a search heuristic outright, and they do not catch constant-factor
//! slowdowns. The boundary between those two is where this file's protection ends, and anyone
//! relying on it should know that rather than infer it.
//!
//! The instances are shaped after what a real timetable looks like, because a guard over a
//! shape nobody uses guards nothing: most activities sit in the room their own group owns (one
//! candidate), a minority compete for a small shared pool (which is what creates the selection
//! variables that cost), and teachers span groups (which is what couples them). They are
//! satisfiable by construction — every group fits in its own room, and no teacher carries more
//! than a week — so a red test means the search got worse, never that the instance was
//! impossible.

use schedulr::{
    Activity, ActivityId, Participant, ParticipantId, Resource, ResourceId, ResourcePool,
    ResourcePoolId, ResourceRequirement, SchedulingProblem, ScoreLevel, SoftGoal, SoftGoalKind,
    SolveOptions, SolveStatus, SolveStrategy, TimeWindow, compile,
};
use std::time::Duration;

/// A week of 40 slots, as five days of eight.
const WEEK: i64 = 40;
/// Every fourth activity of a group needs the shared pool instead of its own room.
const SPECIAL_EVERY: u64 = 4;

fn instance(groups: u64, per_group: u64, specials: u64, teachers: u64) -> SchedulingProblem {
    let mut resources: Vec<Resource> = (1..=groups)
        .map(|group| Resource::new(ResourceId(group), format!("home{group}"), 1))
        .collect();
    resources.extend(
        (1..=specials).map(|special| {
            Resource::new(ResourceId(1000 + special), format!("special{special}"), 1)
        }),
    );

    let mut participants: Vec<Participant> = (1..=groups)
        .map(|group| Participant::new(ParticipantId(group), format!("group{group}")))
        .collect();
    participants.extend(
        (1..=teachers)
            .map(|teacher| Participant::new(ParticipantId(1000 + teacher), format!("t{teacher}"))),
    );

    let mut activities = Vec::new();
    let mut id = 0u64;
    for group in 1..=groups {
        for index in 0..per_group {
            id += 1;
            let teacher = 1000 + (id % teachers) + 1;
            let requirement = if index % SPECIAL_EVERY == 0 {
                ResourceRequirement::from_pool(ResourcePoolId(1), 1)
            } else {
                ResourceRequirement::new(ResourceId(group), 1)
            };
            activities.push(
                Activity::new(
                    ActivityId(id),
                    format!("a{id}"),
                    TimeWindow::new(0, WEEK),
                    2,
                )
                .with_requirement(requirement)
                .with_participant(ParticipantId(group))
                .with_participant(ParticipantId(teacher)),
            );
        }
    }

    SchedulingProblem::new(resources, participants, activities)
        .with_resource_pool(ResourcePool::new(
            ResourcePoolId(1),
            "specials",
            (1..=specials).map(|special| ResourceId(1000 + special)),
        ))
        .with_soft_goal(SoftGoal::new(
            "spread",
            ScoreLevel::Weak,
            1,
            SoftGoalKind::SpreadActivityOverDays,
        ))
}

/// Solves `problem` within `budget` and checks the schedule independently of what the solver
/// claims about it — the point of the whole exercise is not to take the search at its word.
fn schedules_within(problem: &SchedulingProblem, budget: Duration, size: &str) {
    let compiled = compile(problem).expect("the instance compiles");
    let result = compiled.solve_with(&SolveOptions {
        time_limit: Some(budget),
        ..SolveOptions::default()
    });

    assert_eq!(
        result.status,
        SolveStatus::Feasible,
        "{size}: this instance is satisfiable by construction, so anything but a schedule means \
         the search got weaker (statistics: {:?})",
        result.statistics
    );
    let solution = result
        .solution
        .expect("a feasible status carries a schedule");
    let check = compiled.check(&solution);
    assert!(
        check.is_feasible,
        "{size}: the schedule breaks hard constraints: {:?}",
        check.hard_violations
    );
}

/// 24 activities over 3 groups — the size a single classroom's worth of planning reaches.
#[test]
fn schedules_a_small_instance() {
    schedules_within(
        &instance(3, 8, 2, 6),
        Duration::from_secs(1),
        "24 activities",
    );
}

/// 48 activities over 6 groups, with twice the contention for the shared pool.
#[test]
fn schedules_a_medium_instance() {
    schedules_within(
        &instance(6, 8, 3, 10),
        Duration::from_secs(2),
        "48 activities",
    );
}

/// 96 activities over 12 groups: past this the search is at its limit today, so this is the
/// size that will notice first when the limit moves the wrong way.
#[test]
fn schedules_a_large_instance() {
    schedules_within(
        &instance(12, 8, 5, 18),
        Duration::from_secs(3),
        "96 activities",
    );
}

/// How much tree the plain tree search needs for the 24-activity instance. Measured at 62 and
/// exactly reproducible; the ceiling sits roughly three times above it, which is far enough to
/// absorb an honest change in search order and nowhere near the collapse a lost heuristic
/// causes (that run walks 700k nodes and still finds nothing). Before unifier 0.5.3 asked each
/// node whether `SelectedResourceCapacity` could still be satisfied, the same search needed
/// 2,622 nodes — a return to that is a regression this ceiling reports.
const NODE_CEILING: u64 = 200;

/// The sensitive half: [`SolveStrategy::ConstructThenOptimize`] is single-threaded and
/// deterministic, so this pins a *number* rather than a duration and cannot flake on a slow or
/// busy machine.
///
/// The instance is small on purpose. The portfolio carries the larger ones — the plain tree
/// search does not schedule 32 activities of this shape at all — so pinning its effort means
/// staying where it is genuinely at home.
#[test]
fn the_tree_search_still_finds_the_small_instance_in_few_nodes() {
    let problem = instance(3, 8, 2, 6);
    let compiled = compile(&problem).expect("the instance compiles");
    let result = compiled.solve_with(&SolveOptions {
        time_limit: Some(Duration::from_secs(5)),
        strategy: SolveStrategy::ConstructThenOptimize,
        ..SolveOptions::default()
    });

    let solution = result
        .solution
        .as_ref()
        .expect("the tree search alone schedules 24 activities of this shape");
    assert!(
        compiled.check(solution).is_feasible,
        "the schedule breaks hard constraints: {:?}",
        compiled.check(solution).hard_violations
    );
    assert!(
        result.statistics.nodes_expanded <= NODE_CEILING,
        "the search needed {} nodes where {NODE_CEILING} is the ceiling — it is finding the same \
         schedule by walking far more tree, which is what losing a heuristic looks like",
        result.statistics.nodes_expanded
    );
}

/// How much tree the two-instances-of-one-type shape needs. Measured at 7 nodes; the ceiling
/// sits far above it because the number that matters here is the *order of magnitude*: before
/// `SelectedResourceCapacity` propagated, this exact instance walked 1,116,987 nodes and only
/// squeezed past the default budget by luck of the machine.
const SIBLING_NODE_CEILING: u64 = 500;

/// One activity needing two distinct instances of the same resource type, in a day-long window.
///
/// This is the shape a treatment step with `required_resource_count = 2` produces (Avilo,
/// `plan/19`), and it was pathological for a reason worth guarding: the two requirement slots
/// are two tasks on the *same* start variable, so the search fixes their presences near the
/// root — and when both landed on the same capacity-1 instance, nothing said so until every one
/// of the 34,200 start times below had been tried. The overload is independent of *when* the
/// activity runs, and `SelectedResourceCapacity::propagate` now says so at the node that causes
/// it.
///
/// A node count rather than a duration, for the reason the module docs give: it cannot flake on
/// a busy machine. And it is the guard that would have caught this, where the instances above
/// did not — their activities each hold a single requirement, so no two of their tasks ever
/// share a start variable.
#[test]
fn an_activity_needing_two_instances_of_one_type_stays_cheap() {
    let problem = SchedulingProblem::new(
        vec![
            Resource::new(ResourceId(1), "device-a", 1).with_type("device"),
            Resource::new(ResourceId(2), "device-b", 1).with_type("device"),
        ],
        Vec::new(),
        vec![
            Activity::new(ActivityId(1), "step", TimeWindow::new(0, 36_000), 1_800)
                .with_requirement(ResourceRequirement::matching("device", 1))
                .with_requirement(ResourceRequirement::matching("device", 1)),
        ],
    );

    let compiled = compile(&problem).expect("the instance compiles");
    let result = compiled.solve_with(&SolveOptions {
        time_limit: Some(Duration::from_secs(5)),
        strategy: SolveStrategy::ConstructThenOptimize,
        ..SolveOptions::default()
    });

    let solution = result
        .solution
        .as_ref()
        .expect("two capacity-1 instances satisfy two single-unit requirements");

    // No `compiled.check(solution)` here, unlike the guard above, and not by oversight:
    // `check` maps the public `Solution` back onto the model's variables, and that projection
    // does not round-trip for an activity whose *two* requirements draw from one pool — it
    // lists both chosen resources for the activity without saying which requirement took
    // which, so mapping back marks both presences for both slots and the check reports four
    // conflicts on a schedule that is correct. Reproduced on this instance against master
    // before this change, so it is a separate defect in the projection, not something the
    // propagation rule below introduced or has to carry. The assertions here therefore look at
    // the assignment itself, which is unambiguous.
    let assignment = solution
        .assignments
        .first()
        .expect("the one activity is assigned");
    assert_eq!(
        assignment.resources.len(),
        2,
        "both requirements are resolved"
    );
    assert_ne!(
        assignment.resources[0], assignment.resources[1],
        "a capacity-1 instance cannot serve both requirements, so the two must differ"
    );
    assert!(
        result.statistics.nodes_expanded <= SIBLING_NODE_CEILING,
        "the search needed {} nodes where {SIBLING_NODE_CEILING} is the ceiling — the certain \
         overload between two tasks on one start variable is going undetected again",
        result.statistics.nodes_expanded
    );
}
