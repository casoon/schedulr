//! Gierige Konstruktion über **Aktivitäten** statt über Variablen (timbra plan/61).
//!
//! Die Konstruktion in `unifier` setzt Variable für Variable: erst einen Start, irgendwann
//! später die Ressourcenwahl derselben Aktivität. Auf 580 Aktivitäten kam sie so nicht an einen
//! gültigen Plan heran, während eine gierige Platzierung, die je Aktivität Start, Ressourcen und
//! Teilnehmer **zusammen** entscheidet, in Millisekunden einen fand. Dieses Modul ist diese
//! Platzierung, domänenfrei: sie kennt Aktivitäten, Teilnehmer, Ressourcen und die Relationen
//! zwischen ihnen, nichts darüber hinaus.
//!
//! Das Ergebnis ist eine **Startbelegung**, kein Ergebnis. Sie muss nicht jede harte Regel
//! einhalten — Mindestabstände, Tageslasten und Nicht-Überlappungs-Relationen kennt sie nicht,
//! und was sich nicht legen ließ, liegt irgendwo. Die Reparatur in `unifier`
//! (`LocalSearchSolver::repair_from`) setzt darauf auf, und erst das Ergebnis der Reparatur geht
//! durch die übliche Prüfung.

use crate::batch::{InternalCompiled, load_bucket_ranges};
use crate::model::{
    ActivityId, ActivityRelation, EntityRef, ParticipantId, ResourceId, SchedulingProblem,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::time::Instant;
use unifier::constraint::PropagationResult;
use unifier::{BucketRange, PropagationEngine, TrailedDomains, VariableId};

/// Wie oft höchstens neu angesetzt wird. Ein Anlauf, der scheitert, rückt die gescheiterte
/// Einheit für den nächsten nach vorn — die Reihenfolge lernt, der Zufall streut.
const ATTEMPTS: u32 = 60;

/// Oberhalb dieser Horizontlänge verzichtet die Konstruktion: ein Belegungsfeld je Träger wäre
/// zu groß, und so feine Zeitraster sind nicht, wofür sie gebaut ist.
const MAX_HORIZON: i64 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Holder {
    Participant(ParticipantId),
    Resource(ResourceId),
}

/// Aktivitäten, deren Starts fest aneinander hängen (gleicher Start, fester Versatz), werden als
/// eine Einheit gelegt: `offset` ist der Abstand zum Start des Ankers.
struct Unit {
    members: Vec<(ActivityId, i64)>,
    /// Die Ankerstarts, bei denen jedes Mitglied einen erlaubten Start bekommt.
    starts: Vec<i64>,
}

/// Was die Platzierung für eine Aktivität beschlossen hat.
#[derive(Clone)]
struct Placement {
    start: i64,
    /// Je Anforderungsslot die gewählte Ressource.
    resources: Vec<ResourceId>,
    participant: Option<ParticipantId>,
}

/// Deterministischer Zufall, derselbe Generator wie in `unifier` — ein Seed, ein Ergebnis.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    fn mix<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = (self.next() as usize) % (index + 1);
            items.swap(index, other);
        }
    }
}

/// Belegung je Träger als Zähler je Zeiteinheit über den Horizont.
struct Occupancy {
    origin: i64,
    length: usize,
    counts: HashMap<Holder, Vec<u16>>,
}

impl Occupancy {
    fn new(origin: i64, length: usize) -> Self {
        Self {
            origin,
            length,
            counts: HashMap::new(),
        }
    }

    fn range(&self, start: i64, end: i64) -> std::ops::Range<usize> {
        let from = (start - self.origin).clamp(0, self.length as i64) as usize;
        let to = (end - self.origin).clamp(0, self.length as i64) as usize;
        from..to
    }

    fn free(&self, holder: Holder, capacity: u16, start: i64, end: i64) -> bool {
        match self.counts.get(&holder) {
            None => capacity > 0,
            Some(slots) => slots[self.range(start, end)]
                .iter()
                .all(|&count| count < capacity),
        }
    }

    fn take(&mut self, holder: Holder, start: i64, end: i64) {
        let range = self.range(start, end);
        let length = self.length;
        let slots = self.counts.entry(holder).or_insert_with(|| vec![0; length]);
        for count in &mut slots[range] {
            *count = count.saturating_add(1);
        }
    }
}

impl InternalCompiled {
    /// Eine vollständige Startbelegung aller Start-, End- und Auswahlvariablen, die die
    /// Aktivitäten möglichst konfliktfrei legt — oder `None`, wo die Konstruktion nicht greift
    /// (leere Domäne nach der Propagation, zu langer Horizont, keine Aktivitäten).
    pub(crate) fn greedy_start(
        &self,
        problem: &SchedulingProblem,
        deadline: Option<Instant>,
    ) -> Option<HashMap<VariableId, i64>> {
        if self.activities.is_empty() {
            return None;
        }
        let mut domains = TrailedDomains::new(self.graph.domains().clone());
        if PropagationEngine::new().propagate(&self.graph, &mut domains, None)
            == PropagationResult::Conflict
        {
            return None;
        }

        // Erlaubte Starts je Aktivität: was die Propagation übrig ließ, also mitsamt
        // Kalendersperren, Pausen und Tagesgrenzen.
        let mut allowed: BTreeMap<ActivityId, Vec<i64>> = BTreeMap::new();
        for (&activity, &(start, _)) in &self.variables {
            let values = domains.get(&start)?.values();
            if values.is_empty() {
                return None;
            }
            allowed.insert(activity, values);
        }
        let origin = self
            .activities
            .values()
            .map(|activity| activity.allowed_window().start)
            .min()?;
        let horizon_end = self
            .activities
            .values()
            .map(|activity| activity.allowed_window().end)
            .max()?;
        if horizon_end - origin > MAX_HORIZON {
            return None;
        }
        let length = usize::try_from(horizon_end - origin).ok()?;

        let units = self.units(problem, &allowed);
        let capacities: HashMap<ResourceId, u16> = problem
            .resources
            .iter()
            .map(|resource| {
                (
                    resource.id(),
                    u16::try_from(resource.capacity()).unwrap_or(u16::MAX),
                )
            })
            .collect();
        let choice_group: HashMap<ActivityId, usize> = problem
            .participant_choice_groups
            .iter()
            .enumerate()
            .flat_map(|(index, group)| group.activities.iter().map(move |&id| (id, index)))
            .collect();
        // Zwei Blöcke derselben Gruppe, die sich berühren, lesen sich als ein längerer Block.
        // Das zu vermeiden ist nur richtig, wo keine erlaubte Form einen Block verlangt, der
        // länger ist als die Aktivitäten selbst — sonst ist genau das Zusammenwachsen verlangt.
        let pattern_group: HashMap<ActivityId, usize> = problem
            .bucket_load_patterns
            .iter()
            .enumerate()
            .filter(|(_, pattern)| pattern.is_active())
            .filter(|(_, pattern)| {
                let longest_activity = pattern
                    .activities
                    .iter()
                    .filter_map(|id| self.activities.get(id))
                    .map(|activity| activity.duration())
                    .max()
                    .unwrap_or(0);
                pattern
                    .allowed
                    .iter()
                    .flatten()
                    .all(|&block| block <= longest_activity)
            })
            .flat_map(|(index, pattern)| pattern.activities.iter().map(move |&id| (id, index)))
            .collect();
        // Je Aktivität die Start-Obergrenzen, zu denen sie zählt (Index, Grenze).
        let mut start_limits: HashMap<ActivityId, Vec<(usize, u64)>> = HashMap::new();
        for (index, rule) in problem.maximum_bucket_starts.iter().enumerate() {
            for &activity in &rule.activities {
                start_limits
                    .entry(activity)
                    .or_default()
                    .push((index, rule.limit));
            }
        }
        // Träger, die nur an wenigen Buckets aktiv sein dürfen.
        let active_limits: HashMap<Holder, usize> = problem
            .maximum_active_buckets
            .iter()
            .filter_map(|rule| {
                let holder = match rule.entity {
                    EntityRef::Participant(participant) => Holder::Participant(participant),
                    EntityRef::Resource(resource) => Holder::Resource(resource),
                    // Eine Aktivität ist nie „an Tagen aktiv“ — die Regel spricht über Träger.
                    EntityRef::Activity(_) => return None,
                };
                Some((holder, usize::try_from(rule.limit).unwrap_or(usize::MAX)))
            })
            .collect();
        let activity_list: Vec<_> = self.activities.values().cloned().collect();
        let buckets = load_bucket_ranges(problem, &activity_list);

        // Engste Einheit zuerst: wenig Startauswahl, viele Mitglieder, lange Dauer.
        let mut order: Vec<usize> = (0..units.len()).collect();
        let mut rng = Lcg(0x5eed_c0de);
        rng.mix(&mut order);
        order.sort_by_key(|&index| {
            let unit = &units[index];
            let duration: u64 = unit
                .members
                .iter()
                .map(|(id, _)| self.activities[id].duration())
                .sum();
            (
                unit.starts.len(),
                std::cmp::Reverse(unit.members.len()),
                std::cmp::Reverse(duration),
            )
        });

        let mut best: Option<(usize, BTreeMap<ActivityId, Placement>)> = None;
        for attempt in 0..ATTEMPTS {
            if attempt > 0 && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            let mut rng = Lcg(0x5eed_c0de ^ u64::from(attempt).wrapping_mul(0x9e37_79b9));
            let mut state = Attempt {
                occupancy: Occupancy::new(origin, length),
                pattern_blocks: HashMap::new(),
                starts: HashMap::new(),
                active: HashMap::new(),
                chosen: HashMap::new(),
                load: HashMap::new(),
                placements: BTreeMap::new(),
            };
            let mut failures = 0usize;
            let mut first_failure: Option<usize> = None;
            for &index in &order {
                let unit = &units[index];
                let mut starts = unit.starts.clone();
                rng.mix(&mut starts);
                let placed = starts.iter().find_map(|&anchor| {
                    self.try_place(
                        unit,
                        anchor,
                        &state,
                        &Rules {
                            capacities: &capacities,
                            choice_group: &choice_group,
                            pattern_group: &pattern_group,
                            start_limits: &start_limits,
                            active_limits: &active_limits,
                            buckets: &buckets,
                        },
                        &mut rng,
                    )
                });
                match placed {
                    Some(placements) => self.commit(
                        placements,
                        &mut state,
                        &Rules {
                            capacities: &capacities,
                            choice_group: &choice_group,
                            pattern_group: &pattern_group,
                            start_limits: &start_limits,
                            active_limits: &active_limits,
                            buckets: &buckets,
                        },
                    ),
                    None => {
                        failures += 1;
                        first_failure.get_or_insert(index);
                        // Irgendwo hinlegen, aber nicht belegen: die Reparatur verschiebt es,
                        // und die übrigen Einheiten werden davon nicht verdrängt.
                        let anchor = starts[0];
                        for &(activity, offset) in &unit.members {
                            let placement = self.fallback_placement(activity, anchor + offset);
                            state.placements.insert(activity, placement);
                        }
                    }
                }
            }
            if best.as_ref().is_none_or(|(held, _)| failures < *held) {
                best = Some((failures, state.placements));
            }
            if failures == 0 {
                break;
            }
            if let Some(failed) = first_failure {
                order.retain(|&index| index != failed);
                order.insert(0, failed);
            }
        }

        let (_, placements) = best?;
        Some(self.to_assignment(&placements))
    }

    /// Verbindet Aktivitäten über `SameStart` und `FixedOffset` zu Einheiten mit festen Abständen.
    fn units(
        &self,
        problem: &SchedulingProblem,
        allowed: &BTreeMap<ActivityId, Vec<i64>>,
    ) -> Vec<Unit> {
        let mut edges: HashMap<ActivityId, Vec<(ActivityId, i64)>> = HashMap::new();
        for relation in &problem.relations {
            let offset = match relation.relation {
                ActivityRelation::SameStart => 0,
                ActivityRelation::FixedOffset { offset } => offset,
                _ => continue,
            };
            if !allowed.contains_key(&relation.first) || !allowed.contains_key(&relation.second) {
                continue;
            }
            edges
                .entry(relation.first)
                .or_default()
                .push((relation.second, offset));
            edges
                .entry(relation.second)
                .or_default()
                .push((relation.first, -offset));
        }
        let mut seen: HashSet<ActivityId> = HashSet::new();
        let mut units = Vec::new();
        for &root in allowed.keys() {
            if !seen.insert(root) {
                continue;
            }
            let mut members = vec![(root, 0i64)];
            let mut queue = VecDeque::from([(root, 0i64)]);
            while let Some((current, offset)) = queue.pop_front() {
                for &(next, delta) in edges.get(&current).into_iter().flatten() {
                    if seen.insert(next) {
                        members.push((next, offset + delta));
                        queue.push_back((next, offset + delta));
                    }
                }
            }
            let member_sets: Vec<(HashSet<i64>, i64)> = members
                .iter()
                .map(|(id, offset)| (allowed[id].iter().copied().collect(), *offset))
                .collect();
            let starts = allowed[&root]
                .iter()
                .copied()
                .filter(|&anchor| {
                    member_sets
                        .iter()
                        .all(|(set, offset)| set.contains(&(anchor + offset)))
                })
                .collect::<Vec<_>>();
            // Ohne gemeinsamen Start bleibt die Einheit trotzdem legbar — nur nicht konfliktfrei.
            let starts = if starts.is_empty() {
                allowed[&root].clone()
            } else {
                starts
            };
            units.push(Unit { members, starts });
        }
        units
    }

    /// Die festen Träger einer Aktivität: Teilnehmer (auch aus aufgelösten Gruppen), fest
    /// verlangte Teilnehmer und fest verlangte Ressourcen.
    fn fixed_holders(&self, activity: ActivityId) -> Vec<Holder> {
        let mut holders: Vec<Holder> = self.activities[&activity]
            .participants()
            .iter()
            .map(|&participant| Holder::Participant(participant))
            .collect();
        holders.extend(
            self.fixed_participants
                .get(&activity)
                .into_iter()
                .flatten()
                .map(|&participant| Holder::Participant(participant)),
        );
        holders.extend(
            self.fixed_resources
                .get(&activity)
                .into_iter()
                .flatten()
                .map(|&resource| Holder::Resource(resource)),
        );
        holders.sort_unstable();
        holders.dedup();
        holders
    }

    fn try_place(
        &self,
        unit: &Unit,
        anchor: i64,
        state: &Attempt,
        rules: &Rules<'_>,
        rng: &mut Lcg,
    ) -> Option<Vec<(ActivityId, Placement)>> {
        let Rules {
            capacities,
            choice_group,
            pattern_group,
            start_limits,
            active_limits,
            buckets,
        } = *rules;
        // Ob ein Träger in diesem Bucket noch aktiv werden darf: schon aktiv, oder noch Platz.
        let mut opened: HashMap<Holder, Vec<usize>> = HashMap::new();
        let may_be_active =
            |opened: &HashMap<Holder, Vec<usize>>, holder: Holder, bucket: usize| {
                let Some(&limit) = active_limits.get(&holder) else {
                    return true;
                };
                let used = state.active.get(&holder);
                if used.is_some_and(|set| set.contains(&bucket))
                    || opened.get(&holder).is_some_and(|new| new.contains(&bucket))
                {
                    return true;
                }
                used.map_or(0, BTreeSet::len) + opened.get(&holder).map_or(0, Vec::len) < limit
            };
        // Starts, die diese Einheit selbst schon je (Regel, Bucket) beansprucht.
        let mut own_starts: HashMap<(usize, usize), u64> = HashMap::new();
        // Was diese Einheit selbst schon belegt: zwei Mitglieder dürfen sich keinen Träger teilen.
        let mut own: Vec<(Holder, i64, i64)> = Vec::new();
        let taken_by_unit = |own: &[(Holder, i64, i64)], holder: Holder, start: i64, end: i64| {
            own.iter()
                .any(|&(other, from, to)| other == holder && from < end && start < to)
        };
        let capacity = |holder: Holder| match holder {
            Holder::Participant(_) => 1,
            Holder::Resource(resource) => capacities.get(&resource).copied().unwrap_or(1),
        };
        let mut result = Vec::with_capacity(unit.members.len());
        for &(activity, offset) in &unit.members {
            let start = anchor + offset;
            let end = start + i64::try_from(self.activities[&activity].duration()).ok()?;
            let bucket = bucket_of(buckets, start);
            for holder in self.fixed_holders(activity) {
                if !state.occupancy.free(holder, capacity(holder), start, end)
                    || taken_by_unit(&own, holder, start, end)
                    || !may_be_active(&opened, holder, bucket)
                {
                    return None;
                }
                own.push((holder, start, end));
                opened.entry(holder).or_default().push(bucket);
            }
            for &(rule, limit) in start_limits.get(&activity).into_iter().flatten() {
                let key = (rule, bucket_of(buckets, start));
                let used = state.starts.get(&key).copied().unwrap_or(0)
                    + own_starts.get(&key).copied().unwrap_or(0);
                if used >= limit {
                    return None;
                }
                *own_starts.entry(key).or_default() += 1;
            }
            if let Some(&pattern) = pattern_group.get(&activity) {
                let bucket = bucket_of(buckets, start);
                let touches = state
                    .pattern_blocks
                    .get(&(pattern, bucket))
                    .into_iter()
                    .flatten()
                    .any(|&(from, to)| start <= to && from <= end);
                if touches {
                    return None;
                }
            }
            let mut resources = Vec::new();
            for slot in self.selected_resources.get(&activity).into_iter().flatten() {
                let mut candidates: Vec<ResourceId> =
                    slot.iter().map(|&(resource, _)| resource).collect();
                rng.mix(&mut candidates);
                let resource = candidates.into_iter().find(|&resource| {
                    let holder = Holder::Resource(resource);
                    state.occupancy.free(holder, capacity(holder), start, end)
                        && !taken_by_unit(&own, holder, start, end)
                        && may_be_active(&opened, holder, bucket)
                })?;
                own.push((Holder::Resource(resource), start, end));
                opened
                    .entry(Holder::Resource(resource))
                    .or_default()
                    .push(bucket);
                resources.push(resource);
            }
            let mut participant = None;
            if let Some(candidates) = self.selected_participants.get(&activity) {
                let decided = choice_group
                    .get(&activity)
                    .and_then(|group| state.chosen.get(group));
                let mut options: Vec<ParticipantId> = match decided {
                    Some(&chosen) => vec![chosen],
                    None => candidates
                        .iter()
                        .map(|&(participant, _)| participant)
                        .collect(),
                };
                rng.mix(&mut options);
                options.sort_by_key(|option| state.load.get(option).copied().unwrap_or(0));
                let picked = options.into_iter().find(|&option| {
                    let holder = Holder::Participant(option);
                    state.occupancy.free(holder, 1, start, end)
                        && !taken_by_unit(&own, holder, start, end)
                        && may_be_active(&opened, holder, bucket)
                })?;
                own.push((Holder::Participant(picked), start, end));
                opened
                    .entry(Holder::Participant(picked))
                    .or_default()
                    .push(bucket);
                participant = Some(picked);
            }
            result.push((
                activity,
                Placement {
                    start,
                    resources,
                    participant,
                },
            ));
        }
        Some(result)
    }

    fn commit(
        &self,
        placements: Vec<(ActivityId, Placement)>,
        state: &mut Attempt,
        rules: &Rules<'_>,
    ) {
        let Rules {
            choice_group,
            pattern_group,
            start_limits,
            active_limits,
            buckets,
            ..
        } = *rules;
        for (activity, placement) in placements {
            let duration = i64::try_from(self.activities[&activity].duration()).unwrap_or(0);
            let (start, end) = (placement.start, placement.start + duration);
            let bucket = bucket_of(buckets, start);
            let mut holders = self.fixed_holders(activity);
            holders.extend(placement.resources.iter().map(|&r| Holder::Resource(r)));
            holders.extend(placement.participant.map(Holder::Participant));
            for &holder in &holders {
                if active_limits.contains_key(&holder) {
                    state.active.entry(holder).or_default().insert(bucket);
                }
            }
            for holder in self.fixed_holders(activity) {
                state.occupancy.take(holder, start, end);
            }
            for &resource in &placement.resources {
                state.occupancy.take(Holder::Resource(resource), start, end);
            }
            if let Some(participant) = placement.participant {
                state
                    .occupancy
                    .take(Holder::Participant(participant), start, end);
                *state.load.entry(participant).or_default() += duration;
                if let Some(&group) = choice_group.get(&activity) {
                    state.chosen.entry(group).or_insert(participant);
                }
            }
            for &(rule, _) in start_limits.get(&activity).into_iter().flatten() {
                *state
                    .starts
                    .entry((rule, bucket_of(buckets, start)))
                    .or_default() += 1;
            }
            if let Some(&pattern) = pattern_group.get(&activity) {
                state
                    .pattern_blocks
                    .entry((pattern, bucket_of(buckets, start)))
                    .or_default()
                    .push((start, end));
            }
            state.placements.insert(activity, placement);
        }
    }

    /// Eine Platzierung ohne Rücksicht auf Konflikte: erster Kandidat je Slot.
    fn fallback_placement(&self, activity: ActivityId, start: i64) -> Placement {
        Placement {
            start,
            resources: self
                .selected_resources
                .get(&activity)
                .into_iter()
                .flatten()
                .filter_map(|slot| slot.first().map(|&(resource, _)| resource))
                .collect(),
            participant: self
                .selected_participants
                .get(&activity)
                .and_then(|candidates| candidates.first().map(|&(participant, _)| participant)),
        }
    }

    /// Übersetzt die Platzierungen in Werte für Start-, End- und Auswahlvariablen. Was hier
    /// fehlt, setzt `repair_from` selbst.
    fn to_assignment(
        &self,
        placements: &BTreeMap<ActivityId, Placement>,
    ) -> HashMap<VariableId, i64> {
        let mut assignment = HashMap::new();
        for (activity, placement) in placements {
            let Some(&(start, end)) = self.variables.get(activity) else {
                continue;
            };
            let duration = i64::try_from(self.activities[activity].duration()).unwrap_or(0);
            assignment.insert(start, placement.start);
            assignment.insert(end, placement.start + duration);
            for (slot, chosen) in self
                .selected_resources
                .get(activity)
                .into_iter()
                .flatten()
                .zip(&placement.resources)
            {
                for &(resource, presence) in slot {
                    assignment.insert(presence, i64::from(resource == *chosen));
                }
            }
            if let Some(chosen) = placement.participant {
                for &(participant, presence) in self
                    .selected_participants
                    .get(activity)
                    .into_iter()
                    .flatten()
                {
                    assignment.insert(presence, i64::from(participant == chosen));
                }
            }
        }
        assignment
    }
}

/// Der Zustand eines Anlaufs.
struct Attempt {
    occupancy: Occupancy,
    /// Je (Blockform-Gruppe, Bucket) die gelegten Blöcke — zwei Blöcke derselben Gruppe dürfen
    /// sich in einem Bucket nicht berühren, sonst läsen sie sich als ein längerer Block.
    pattern_blocks: HashMap<(usize, usize), Vec<(i64, i64)>>,
    /// Je (Start-Obergrenze, Bucket), wie viele Aktivitäten dort schon beginnen.
    starts: HashMap<(usize, usize), u64>,
    /// Je Träger mit Bucket-Grenze die Buckets, in denen er schon aktiv ist.
    active: HashMap<Holder, BTreeSet<usize>>,
    /// Je Auswahlgruppe der gewählte Teilnehmer.
    chosen: HashMap<usize, ParticipantId>,
    /// Belegte Zeit je wählbarem Teilnehmer, damit die Wahl sich verteilt.
    load: HashMap<ParticipantId, i64>,
    placements: BTreeMap<ActivityId, Placement>,
}

/// Die Regeln, die ein Anlauf beim Legen beachtet — gebündelt, weil jede Platzierung sie alle
/// braucht.
#[derive(Clone, Copy)]
struct Rules<'a> {
    capacities: &'a HashMap<ResourceId, u16>,
    choice_group: &'a HashMap<ActivityId, usize>,
    pattern_group: &'a HashMap<ActivityId, usize>,
    start_limits: &'a HashMap<ActivityId, Vec<(usize, u64)>>,
    active_limits: &'a HashMap<Holder, usize>,
    buckets: &'a [BucketRange],
}

fn bucket_of(buckets: &[BucketRange], value: i64) -> usize {
    buckets
        .iter()
        .find(|range| range.start <= value && value < range.end)
        .map_or(usize::MAX, |range| range.bucket)
}
