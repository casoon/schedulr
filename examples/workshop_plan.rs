//! A two-day workshop plan: rooms, trainers and five activities placed on a
//! weekly slot calendar. Uses only the API released in schedulr 0.8.0.
//!
//! One time unit is one hour; hour 0 is Monday 00:00.
//!
//! Run with `cargo run --example workshop_plan`.

use schedulr::{
    AcademicPeriod, Activity, ActivityId, Assignment, DayTemplate, GroupMembership, Participant,
    ParticipantGroup, ParticipantGroupId, ParticipantId, ParticipantPool, ParticipantPoolId,
    ParticipantRequirement, Resource, ResourceId, ResourcePool, ResourcePoolId,
    ResourceRequirement, ScheduleTemplate, SchedulingProblem, ScoreLevel, ScoreRule, SlotTemplate,
    TimeWindow, compile,
};

const DAYS: [&str; 2] = ["Mon", "Tue"];
const FIRST_HOUR: i64 = 9;
const LAST_HOUR: i64 = 16;

fn main() {
    let resources = vec![
        Resource::new(ResourceId(1), "Room Aurora", 24)
            .with_type("room")
            .with_feature("projector"),
        Resource::new(ResourceId(2), "Room Birch", 12)
            .with_type("room")
            .with_feature("projector"),
        Resource::new(ResourceId(3), "Lab 1", 12)
            .with_type("lab")
            .with_feature("workstations"),
    ];
    let participants = vec![
        Participant::new(ParticipantId(1), "Ana"),
        Participant::new(ParticipantId(2), "Ben"),
        Participant::new(ParticipantId(3), "Chris"),
    ];
    let trainers = ParticipantGroupId(1);
    let seminar_rooms = ResourcePoolId(1);
    let rust_trainers = ParticipantPoolId(1);

    let activities = vec![
        Activity::new(ActivityId(1), "Kickoff", TimeWindow::new(0, 48), 1)
            .with_requirement(ResourceRequirement::matching("room", 1).with_minimum_capacity(20))
            .with_participant_group(trainers),
        Activity::new(
            ActivityId(2),
            "Rust fundamentals",
            TimeWindow::new(0, 48),
            3,
        )
        .with_requirement(
            ResourceRequirement::from_pool(seminar_rooms, 1).with_feature("projector"),
        )
        .with_participant_requirement(ParticipantRequirement::from_pool(rust_trainers)),
        Activity::new(ActivityId(3), "Hands-on lab", TimeWindow::new(0, 48), 2)
            .with_requirement(ResourceRequirement::matching("lab", 1).with_feature("workstations"))
            .with_participant_requirement(ParticipantRequirement::from_pool(rust_trainers)),
        Activity::new(ActivityId(4), "Solver deep dive", TimeWindow::new(0, 48), 2)
            .with_requirement(ResourceRequirement::from_pool(seminar_rooms, 1))
            .with_participant(ParticipantId(3)),
        Activity::new(ActivityId(5), "Q&A", TimeWindow::new(24, 48), 1)
            .with_requirement(ResourceRequirement::matching("room", 1).with_minimum_capacity(20))
            .with_participant_group(trainers),
    ];

    // Every day offers slots at 09, 10, 11 (ending by 12:00) and 13, 14, 15 (ending by 16:00).
    // Tuesday 13:00-14:59 is closed as a one-off exception.
    let mut day = DayTemplate::new(0);
    for (offset, length) in [(9, 3), (10, 2), (11, 1), (13, 3), (14, 2), (15, 1)] {
        day = day.with_slot(SlotTemplate::new(format!("{offset:02}:00"), offset, length));
    }
    let calendar = ScheduleTemplate::new(24)
        .with_day(day)
        .with_unavailable_range(37, 38);

    let problem = SchedulingProblem::new(resources, participants, activities)
        .with_resource_pool(ResourcePool::new(
            seminar_rooms,
            "Seminar rooms",
            [ResourceId(1), ResourceId(2)],
        ))
        .with_participant_pool(ParticipantPool::new(
            rust_trainers,
            "Rust trainers",
            [ParticipantId(1), ParticipantId(2)],
        ))
        .with_participant_group(ParticipantGroup::new(trainers, "Trainers"))
        .with_group_membership(GroupMembership::participant(trainers, ParticipantId(1)))
        .with_group_membership(GroupMembership::participant(trainers, ParticipantId(2)))
        .with_group_membership(GroupMembership::participant(trainers, ParticipantId(3)))
        .with_calendar(
            AcademicPeriod {
                window: TimeWindow::new(0, 48),
            },
            calendar,
        )
        .with_score_rule(ScoreRule::prefer_window(
            "kickoff on Monday at 09:00",
            ScoreLevel::Strong,
            ActivityId(1),
            TimeWindow::new(9, 10),
            1,
        ))
        .with_score_rule(ScoreRule::prefer_window(
            "Q&A on Tuesday afternoon",
            ScoreLevel::Medium,
            ActivityId(5),
            TimeWindow::new(37, 40),
            1,
        ))
        .with_score_rule(ScoreRule::prefer_window(
            "lab on Tuesday morning",
            ScoreLevel::Weak,
            ActivityId(3),
            TimeWindow::new(33, 36),
            1,
        ));

    println!(
        "Input: {} resources, {} participants, {} activities, 1 resource pool, 1 participant pool, 1 group",
        problem.resources.len(),
        problem.participants.len(),
        problem.activities.len(),
    );
    println!("Calendar: 24-hour cycle, starts at 09 10 11 13 14 15, Tue 13:00-14:59 closed");
    println!();

    let compiled = compile(&problem).expect("the workshop problem compiles");
    let result = compiled.solve();
    println!(
        "Status: {:?} (optimal: {})",
        result.status, result.statistics.optimal
    );
    let solution = result.solution.expect("the workshop problem is feasible");
    println!();

    println!(
        "{:<17} {:<21} {:<13} Participants",
        "When", "Activity", "Resource"
    );
    let mut ordered: Vec<&Assignment> = solution.assignments.iter().collect();
    ordered.sort_by_key(|assignment| (assignment.window.start, assignment.activity));
    for assignment in &ordered {
        let activity = problem
            .activities
            .iter()
            .find(|activity| activity.id() == assignment.activity)
            .expect("assigned activity exists");
        let resources = assignment
            .resources
            .iter()
            .map(|id| resource_name(&problem, *id))
            .collect::<Vec<_>>()
            .join(", ");
        let people = assignment
            .participants
            .iter()
            .map(|id| participant_name(&problem, *id))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{:<17} {:<21} {:<13} {}",
            format_window(assignment.window),
            format!("{} {}", assignment.activity, activity.name()),
            resources,
            people
        );
    }
    println!();

    println!("Timeline (one column per hour, 09-16)");
    let mut header = format!("{:<13}", "");
    for name in DAYS {
        header.push_str(&format!(" {:<21}", name));
    }
    println!("{}", header.trim_end());
    let mut hours = format!("{:<13}", "");
    for _ in DAYS {
        hours.push(' ');
        for hour in FIRST_HOUR..LAST_HOUR {
            hours.push_str(&format!("{hour:02} "));
        }
    }
    println!("{}", hours.trim_end());
    for resource in &problem.resources {
        let booked: Vec<&Assignment> = solution
            .assignments
            .iter()
            .filter(|assignment| assignment.resources.contains(&resource.id()))
            .collect();
        println!("{}", timeline_row(resource.name(), &booked));
    }
    for participant in &problem.participants {
        let booked: Vec<&Assignment> = solution
            .assignments
            .iter()
            .filter(|assignment| assignment.participants.contains(&participant.id()))
            .collect();
        println!("{}", timeline_row(participant.name(), &booked));
    }
    println!();

    println!(
        "Score: hard {}, strong {}, medium {}, weak {}",
        solution.score.hard, solution.score.strong, solution.score.medium, solution.score.weak
    );
    for component in &solution.score_components {
        let activity = component
            .activity
            .map_or_else(String::new, |id| id.to_string());
        println!(
            "  {:<7} {:<27} {:<3} {}",
            format!("{:?}", component.level),
            component.category,
            activity,
            component.value
        );
    }
}

fn format_window(window: TimeWindow) -> String {
    let day = DAYS[usize::try_from(window.start / 24).expect("non-negative day")];
    format!(
        "{day} {:02}:00-{:02}:00",
        window.start % 24,
        window.end - window.start / 24 * 24
    )
}

fn timeline_row(name: &str, booked: &[&Assignment]) -> String {
    let mut row = format!("{name:<13}");
    for day in 0..DAYS.len() as i64 {
        row.push(' ');
        for hour in FIRST_HOUR..LAST_HOUR {
            let time = day * 24 + hour;
            let cell = booked
                .iter()
                .find(|assignment| assignment.window.start <= time && time < assignment.window.end)
                .map_or_else(
                    || "..".to_string(),
                    |assignment| assignment.activity.to_string(),
                );
            row.push_str(&format!("{cell} "));
        }
    }
    row.trim_end().to_string()
}

fn resource_name(problem: &SchedulingProblem, id: ResourceId) -> &str {
    problem
        .resources
        .iter()
        .find(|resource| resource.id() == id)
        .map_or("?", Resource::name)
}

fn participant_name(problem: &SchedulingProblem, id: ParticipantId) -> &str {
    problem
        .participants
        .iter()
        .find(|participant| participant.id() == id)
        .map_or("?", Participant::name)
}
