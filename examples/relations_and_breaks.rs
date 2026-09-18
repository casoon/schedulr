//! One training day with a lunch break, a trainer who is only available in the
//! afternoon, and hard relations between activities.
//!
//! Breaks, availability ranges and activity relations were added after the
//! 0.8.0 release; this example needs the current master branch.
//!
//! One time unit is one hour; hour 0 is Monday 00:00.
//!
//! Run with `cargo run --example relations_and_breaks`.

use schedulr::{
    AcademicPeriod, Activity, ActivityId, ActivityRelation, ActivityRelationConstraint,
    BreakTemplate, DayTemplate, Participant, ParticipantId, Resource, ResourceId,
    ResourceRequirement, ScheduleTemplate, SchedulingProblem, SlotTemplate, Solution, TimeWindow,
    compile,
};

fn main() {
    let aurora = ResourceId(1);
    let birch = ResourceId(2);
    let lab = ResourceId(3);
    let library = ResourceId(4);
    let resources = vec![
        Resource::new(aurora, "Room Aurora", 24),
        Resource::new(birch, "Room Birch", 12),
        Resource::new(lab, "Lab 1", 12),
        Resource::new(library, "Library", 6),
    ];
    let ana = ParticipantId(1);
    let ben = ParticipantId(2);
    let chris = ParticipantId(3);
    let participants = vec![
        Participant::new(ana, "Ana"),
        // Ben cannot start anything between 09:00 and 13:00 (inclusive).
        Participant::new(ben, "Ben").with_unavailable_range(9, 13),
        Participant::new(chris, "Chris"),
    ];

    let day = TimeWindow::new(0, 24);
    let activities = vec![
        Activity::new(ActivityId(1), "Welcome", day, 1)
            .with_requirement(ResourceRequirement::new(aurora, 1)),
        Activity::new(ActivityId(2), "Theory", day, 2)
            .with_requirement(ResourceRequirement::new(aurora, 1))
            .with_participant(ana),
        Activity::new(ActivityId(3), "Lab", day, 1)
            .with_requirement(ResourceRequirement::new(lab, 1))
            .with_participant(ana),
        Activity::new(ActivityId(4), "Review", day, 1)
            .with_requirement(ResourceRequirement::new(birch, 1))
            .with_participant(ben),
        Activity::new(ActivityId(5), "Parallel track", day, 2)
            .with_requirement(ResourceRequirement::new(birch, 1))
            .with_participant(chris),
        Activity::new(ActivityId(6), "Office hour", day, 1)
            .with_requirement(ResourceRequirement::new(library, 1))
            .with_participant(ben),
    ];

    // Slots start every hour from 09 to 16 and end by 17:00; lunch is 12:00-13:00.
    let mut template = DayTemplate::new(0).with_break(BreakTemplate {
        name: "Lunch".to_string(),
        window: TimeWindow::new(12, 13),
    });
    for hour in 9..17 {
        template = template.with_slot(SlotTemplate::new(
            format!("{hour:02}:00"),
            hour,
            (17 - hour) as u64,
        ));
    }

    let relations = [
        (1, 2, ActivityRelation::Precedence { min_gap: 1 }),
        (2, 3, ActivityRelation::Consecutive),
        (3, 4, ActivityRelation::Precedence { min_gap: 0 }),
        (2, 5, ActivityRelation::SameStart),
        (1, 6, ActivityRelation::NoOverlap),
    ];
    let mut problem = SchedulingProblem::new(resources, participants, activities).with_calendar(
        AcademicPeriod { window: day },
        ScheduleTemplate::new(24).with_day(template),
    );
    for (first, second, relation) in relations {
        problem = problem.with_relation(ActivityRelationConstraint::new(
            ActivityId(first),
            ActivityId(second),
            relation,
        ));
    }

    let compiled = compile(&problem).expect("the training day compiles");
    let result = compiled.solve();
    println!("Status: {:?}", result.status);
    let solution = result.solution.expect("the training day is feasible");
    println!();

    println!(
        "{:<13} {:<19} {:<13} Participants",
        "When", "Activity", "Resource"
    );
    let mut ordered = solution.assignments.clone();
    ordered.sort_by_key(|assignment| (assignment.window.start, assignment.activity));
    for assignment in &ordered {
        let activity = problem
            .activities
            .iter()
            .find(|activity| activity.id() == assignment.activity)
            .expect("assigned activity exists");
        let resource = assignment
            .resources
            .iter()
            .map(|id| {
                problem
                    .resources
                    .iter()
                    .find(|resource| resource.id() == *id)
                    .map_or("?", Resource::name)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let people = assignment
            .participants
            .iter()
            .map(|id| {
                problem
                    .participants
                    .iter()
                    .find(|participant| participant.id() == *id)
                    .map_or("?", Participant::name)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let line = format!(
            "{:<13} {:<19} {:<13} {}",
            format!(
                "{}-{}",
                clock(assignment.window.start),
                clock(assignment.window.end)
            ),
            format!("{} {}", assignment.activity, activity.name()),
            resource,
            people
        );
        println!("{}", line.trim_end());
    }
    println!();

    println!("Checks against the solution");
    for relation in &problem.relations {
        let first = window_of(&solution, relation.first);
        let second = window_of(&solution, relation.second);
        let (name, detail) = match relation.relation {
            ActivityRelation::SameStart => (
                "SameStart".to_string(),
                format!(
                    "{} and {} both start at {}",
                    relation.first,
                    relation.second,
                    clock(second.start)
                ),
            ),
            ActivityRelation::Consecutive => (
                "Consecutive".to_string(),
                format!(
                    "{} ends {}, {} starts {}",
                    relation.first,
                    clock(first.end),
                    relation.second,
                    clock(second.start)
                ),
            ),
            ActivityRelation::Precedence { min_gap } => (
                format!("Precedence (gap {min_gap}h)"),
                format!(
                    "{} ends {}, {} starts {}",
                    relation.first,
                    clock(first.end),
                    relation.second,
                    clock(second.start)
                ),
            ),
            ActivityRelation::FixedOffset { offset } => (
                format!("FixedOffset (offset {offset})"),
                format!(
                    "{} starts {}, {} starts {}",
                    relation.first,
                    clock(first.start),
                    relation.second,
                    clock(second.start)
                ),
            ),
            ActivityRelation::NoOverlap => (
                "NoOverlap".to_string(),
                format!(
                    "{} {}-{}, {} {}-{}",
                    relation.first,
                    clock(first.start),
                    clock(first.end),
                    relation.second,
                    clock(second.start),
                    clock(second.end)
                ),
            ),
        };
        println!("  {name:<22} {detail}");
    }
    let lunch = TimeWindow::new(12, 13);
    let during_lunch = solution
        .assignments
        .iter()
        .filter(|assignment| {
            assignment.window.start < lunch.end && lunch.start < assignment.window.end
        })
        .count();
    println!(
        "  {:<22} {during_lunch} activities overlap the break",
        "Lunch 12:00-13:00"
    );
    println!(
        "  {:<22} a6 starts {}, a4 starts {}",
        "Ben unavailable 09-13",
        clock(window_of(&solution, ActivityId(6)).start),
        clock(window_of(&solution, ActivityId(4)).start)
    );
}

fn window_of(solution: &Solution, activity: ActivityId) -> TimeWindow {
    solution
        .assignments
        .iter()
        .find(|assignment| assignment.activity == activity)
        .expect("every activity is assigned")
        .window
}

fn clock(hour: i64) -> String {
    format!("{:02}:00", hour % 24)
}
