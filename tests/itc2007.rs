//! ITC-2007 Track 3 (curriculum-based course timetabling) as an external reference.
//!
//! The other scale guards use instances shaped by us; these are the competition's own, with
//! published best values, so they say how far the solver is from the state of the art rather
//! than only whether it got worse. The data and its source are in `tests/data/itc2007/`.
//!
//! Mapping: every lecture is an activity of duration 1 named after its course. Its teacher,
//! its course and each of its curricula are participants, so no participant ever has two
//! lectures at once — that is the competition's teacher and curriculum conflicts plus "the
//! lectures of a course take distinct periods". A course's unavailable periods are blocked
//! windows of the course participant. Rooms form one pool. The soft side is approximated with
//! the goals `schedulr` has (spread over days, idle time, room stability) and room capacity as
//! a resource cost per course: a room costs the students it lacks, as in UD2. What counts is the **independent** evaluation below, which scores the result
//! exactly as the competition did (formulation UD2).
//!
//! The parse test runs always; the solve is `#[ignore]`, because it spends its whole budget:
//!
//! ```text
//! cargo test --release --test itc2007 -- --ignored --nocapture
//! ```
//!
//! `ITC_TIME_LIMIT` (seconds, default 60) sets the budget per instance.

#![allow(clippy::print_stdout)]

use schedulr::{
    Activity, ActivityId, DayTemplate, Participant, ParticipantId, Resource, ResourceCostGoal,
    ResourceId, ResourcePool, ResourcePoolId, ResourceRequirement, ScheduleTemplate,
    SchedulingProblem, ScoreLevel, SlotTemplate, SoftGoal, SoftGoalKind, Solution, SolveOptions,
    TimeWindow, compile,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

struct Course {
    id: String,
    teacher: String,
    lectures: u32,
    min_days: u32,
    students: u32,
}

struct Instance {
    days: i64,
    periods: i64,
    courses: Vec<Course>,
    rooms: Vec<(String, u32)>,
    curricula: Vec<(String, Vec<String>)>,
    unavailable: Vec<(String, i64, i64)>,
}

fn parse(text: &str) -> Instance {
    let header = |name: &str| -> i64 {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}:")))
            .unwrap_or_else(|| panic!("header {name}"))
            .trim()
            .parse()
            .unwrap()
    };
    let section = |name: &str| -> Vec<Vec<&str>> {
        text.lines()
            .skip_while(|line| line.trim() != format!("{name}:"))
            .skip(1)
            .take_while(|line| !line.trim().is_empty() && !line.trim_end().ends_with(':'))
            .map(|line| line.split_whitespace().collect())
            .collect()
    };
    Instance {
        days: header("Days"),
        periods: header("Periods_per_day"),
        courses: section("COURSES")
            .into_iter()
            .map(|f| Course {
                id: f[0].to_string(),
                teacher: f[1].to_string(),
                lectures: f[2].parse().unwrap(),
                min_days: f[3].parse().unwrap(),
                students: f[4].parse().unwrap(),
            })
            .collect(),
        rooms: section("ROOMS")
            .into_iter()
            .map(|f| (f[0].to_string(), f[1].parse().unwrap()))
            .collect(),
        curricula: section("CURRICULA")
            .into_iter()
            .map(|f| {
                (
                    f[0].to_string(),
                    f[2..].iter().map(|c| c.to_string()).collect(),
                )
            })
            .collect(),
        unavailable: section("UNAVAILABILITY_CONSTRAINTS")
            .into_iter()
            .map(|f| {
                (
                    f[0].to_string(),
                    f[1].parse().unwrap(),
                    f[2].parse().unwrap(),
                )
            })
            .collect(),
    }
}

fn load(name: &str) -> Instance {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/itc2007")
        .join(format!("{name}.ctt"));
    parse(&std::fs::read_to_string(path).expect("instance file"))
}

/// Participant and activity ids, kept apart by range.
const COURSE_BASE: u64 = 10_000;
const CURRICULUM_BASE: u64 = 20_000;

/// The instance as a scheduling problem, plus the course of every activity.
fn problem(instance: &Instance) -> (SchedulingProblem, BTreeMap<ActivityId, usize>) {
    let horizon = instance.days * instance.periods;
    let teachers: BTreeSet<&str> = instance
        .courses
        .iter()
        .map(|c| c.teacher.as_str())
        .collect();
    let teacher_id =
        |name: &str| ParticipantId(teachers.iter().position(|t| *t == name).unwrap() as u64 + 1);
    let course_index = |id: &str| instance.courses.iter().position(|c| c.id == id).unwrap();

    let mut participants: Vec<Participant> = teachers
        .iter()
        .map(|name| Participant::new(teacher_id(name), *name))
        .collect();
    for (index, course) in instance.courses.iter().enumerate() {
        let mut participant =
            Participant::new(ParticipantId(COURSE_BASE + index as u64), &course.id);
        for (_, day, period) in instance
            .unavailable
            .iter()
            .filter(|(c, _, _)| *c == course.id)
        {
            let slot = day * instance.periods + period;
            participant = participant.with_blocked_window(TimeWindow::new(slot, slot + 1));
        }
        participants.push(participant);
    }
    for (index, (name, _)) in instance.curricula.iter().enumerate() {
        participants.push(Participant::new(
            ParticipantId(CURRICULUM_BASE + index as u64),
            name,
        ));
    }

    // Capacity in `schedulr` is concurrent use, not seats: one lecture per room and period.
    // Seats are a resource cost below.
    let rooms: Vec<Resource> = instance
        .rooms
        .iter()
        .enumerate()
        .map(|(index, (name, _))| Resource::new(ResourceId(index as u64 + 1), name, 1))
        .collect();

    let mut activities = Vec::new();
    let mut course_of = BTreeMap::new();
    for (index, course) in instance.courses.iter().enumerate() {
        for _ in 0..course.lectures {
            let id = ActivityId(activities.len() as u64 + 1);
            let mut activity = Activity::new(id, &course.id, TimeWindow::new(0, horizon), 1)
                .with_requirement(ResourceRequirement::from_pool(ResourcePoolId(1), 1))
                .with_participant(teacher_id(&course.teacher))
                .with_participant(ParticipantId(COURSE_BASE + index as u64));
            for (curriculum, (_, members)) in instance.curricula.iter().enumerate() {
                if members.iter().any(|member| course_index(member) == index) {
                    activity = activity
                        .with_participant(ParticipantId(CURRICULUM_BASE + curriculum as u64));
                }
            }
            course_of.insert(id, index);
            activities.push(activity);
        }
    }

    let mut template = ScheduleTemplate::new(horizon);
    for day in 0..instance.days {
        let mut day_template = DayTemplate::new(day * instance.periods);
        for period in 0..instance.periods {
            day_template =
                day_template.with_slot(SlotTemplate::new(format!("d{day}p{period}"), period, 1));
        }
        template = template.with_day(day_template);
    }

    let mut lectures_of: BTreeMap<usize, Vec<ActivityId>> = BTreeMap::new();
    for (activity, course) in &course_of {
        lectures_of.entry(*course).or_default().push(*activity);
    }

    let mut problem = SchedulingProblem::new(rooms, participants, activities)
        .with_resource_pool(ResourcePool::new(
            ResourcePoolId(1),
            "rooms",
            (1..=instance.rooms.len() as u64).map(ResourceId),
        ))
        .with_schedule_template(template)
        .with_soft_goal(SoftGoal::new(
            "min working days",
            ScoreLevel::Weak,
            5,
            SoftGoalKind::SpreadActivityOverDays,
        ))
        .with_soft_goal(SoftGoal::new(
            "compactness",
            ScoreLevel::Weak,
            2,
            SoftGoalKind::MinimizeParticipantIdle,
        ))
        .with_soft_goal(SoftGoal::new(
            "room stability",
            ScoreLevel::Weak,
            1,
            SoftGoalKind::RoomStability,
        ));
    // UD2 charges every student over the room's capacity, once per lecture.
    for (course, lectures) in lectures_of {
        let students = instance.courses[course].students;
        problem = problem.with_resource_cost_goal(ResourceCostGoal {
            category: "room capacity".to_string(),
            level: ScoreLevel::Weak,
            weight: 1,
            activities: lectures,
            costs: instance
                .rooms
                .iter()
                .enumerate()
                .map(|(index, (_, seats))| {
                    (
                        ResourceId(index as u64 + 1),
                        i64::from(students.saturating_sub(*seats)),
                    )
                })
                .collect(),
            default_cost: 0,
        });
    }
    (problem, course_of)
}

/// Hard violations and the UD2 penalty, computed from the instance alone — not from anything
/// the solver reports.
fn evaluate(
    instance: &Instance,
    solution: &Solution,
    course_of: &BTreeMap<ActivityId, usize>,
) -> (Vec<String>, [u32; 4]) {
    let mut hard = Vec::new();
    let periods = instance.periods;
    // (course, slot, room index)
    let placed: Vec<(usize, i64, usize)> = solution
        .assignments
        .iter()
        .map(|assignment| {
            let room = assignment.resources.first().expect("a room").0 as usize - 1;
            (
                course_of[&assignment.activity],
                assignment.window.start,
                room,
            )
        })
        .collect();

    for (index, course) in instance.courses.iter().enumerate() {
        let count = placed.iter().filter(|(c, _, _)| *c == index).count();
        if count != course.lectures as usize {
            hard.push(format!(
                "{}: {count} von {} Vorlesungen",
                course.id, course.lectures
            ));
        }
    }
    let mut room_use = BTreeSet::new();
    let mut teacher_use = BTreeSet::new();
    let mut course_use = BTreeSet::new();
    for &(course, slot, room) in &placed {
        if !room_use.insert((slot, room)) {
            hard.push(format!("Raum {room} doppelt in Slot {slot}"));
        }
        if !teacher_use.insert((slot, instance.courses[course].teacher.as_str())) {
            hard.push(format!("Lehrkraft doppelt in Slot {slot}"));
        }
        let id = &instance.courses[course].id;
        if !course_use.insert((slot, course)) {
            hard.push(format!("Kurs {id} doppelt in Slot {slot}"));
        }
        if instance
            .unavailable
            .iter()
            .any(|(c, day, period)| c == id && day * periods + period == slot)
        {
            hard.push(format!("{id} in gesperrtem Slot {slot}"));
        }
    }
    for (curriculum, (name, members)) in instance.curricula.iter().enumerate() {
        let busy: Vec<i64> = placed
            .iter()
            .filter(|(course, _, _)| members.contains(&instance.courses[*course].id))
            .map(|(_, slot, _)| *slot)
            .collect();
        let distinct: BTreeSet<i64> = busy.iter().copied().collect();
        if distinct.len() != busy.len() {
            hard.push(format!("Curriculum {name} ({curriculum}) doppelt belegt"));
        }
    }

    // capacity, min working days, compactness, room stability
    let mut penalty = [0u32; 4];
    for &(course, _, room) in &placed {
        penalty[0] += instance.courses[course]
            .students
            .saturating_sub(instance.rooms[room].1);
    }
    for (index, course) in instance.courses.iter().enumerate() {
        let days: BTreeSet<i64> = placed
            .iter()
            .filter(|(c, _, _)| *c == index)
            .map(|(_, slot, _)| slot / periods)
            .collect();
        penalty[1] += 5 * course.min_days.saturating_sub(days.len() as u32);
        let rooms: BTreeSet<usize> = placed
            .iter()
            .filter(|(c, _, _)| *c == index)
            .map(|(_, _, room)| *room)
            .collect();
        penalty[3] += rooms.len().saturating_sub(1) as u32;
    }
    for (_, members) in &instance.curricula {
        let busy: BTreeSet<i64> = placed
            .iter()
            .filter(|(course, _, _)| members.contains(&instance.courses[*course].id))
            .map(|(_, slot, _)| *slot)
            .collect();
        for &slot in &busy {
            let neighbour = |other: i64| other / periods == slot / periods && busy.contains(&other);
            if !neighbour(slot - 1) && !neighbour(slot + 1) {
                penalty[2] += 2;
            }
        }
    }
    (hard, penalty)
}

#[test]
fn the_instances_parse_and_compile() {
    for (name, courses, lectures) in [("comp01", 30, 160), ("comp11", 30, 162)] {
        let instance = load(name);
        assert_eq!(instance.courses.len(), courses, "{name}");
        let total: u32 = instance.courses.iter().map(|c| c.lectures).sum();
        assert_eq!(total, lectures, "{name}");
        let (problem, _) = problem(&instance);
        compile(&problem).expect("the instance compiles");
    }
}

fn solve(name: &str, best_known: u32) {
    let budget = std::env::var("ITC_TIME_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60);
    let instance = load(name);
    let (problem, course_of) = problem(&instance);
    let compiled = compile(&problem).expect("the instance compiles");
    let started = std::time::Instant::now();
    let result = compiled.solve_with(&SolveOptions {
        time_limit: Some(Duration::from_secs(budget)),
        ..SolveOptions::default()
    });
    let seconds = started.elapsed().as_secs_f64();
    let solution = result.solution.unwrap_or_else(|| {
        panic!(
            "{name}: kein hart gültiger Plan in {budget} s ({:?})",
            result.status
        )
    });
    let (hard, penalty) = evaluate(&instance, &solution, &course_of);
    println!(
        "{name}: {seconds:.1} s, {} harte Verstöße, ITC-Strafe {} (Bestwert {best_known}): \
         Kapazität {}, Mindesttage {}, Kompaktheit {}, Raumstabilität {}",
        hard.len(),
        penalty.iter().sum::<u32>(),
        penalty[0],
        penalty[1],
        penalty[2],
        penalty[3]
    );
    assert!(hard.is_empty(), "{name}: {:?}", &hard[..hard.len().min(5)]);
}

#[test]
#[ignore = "spends its whole budget: see the module docs"]
fn comp01_against_its_best_known_value() {
    solve("comp01", 5);
}

#[test]
#[ignore = "spends its whole budget: see the module docs"]
fn comp11_against_its_best_known_value() {
    solve("comp11", 0);
}
