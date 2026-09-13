import { ansiToHtml } from '@casoon/pages-theme/ansi';
import type { ShowcaseExample } from '@casoon/pages-theme/showcase';

// Every example is a runnable program in examples/*.rs (`cargo run --example <name>`).
// scripts/regenerate-examples.sh runs them with schedulr and captures the output in
// examples/output/<name>.txt, so the site shows real solver output next to its source.
const sources = import.meta.glob<string>('../../examples/*.rs', {
  query: '?raw',
  import: 'default',
  eager: true,
});
const outputs = import.meta.glob<string>('../../examples/output/*.txt', {
  query: '?raw',
  import: 'default',
  eager: true,
});

const catalogue = [
  {
    slug: 'workshop-plan',
    name: 'workshop_plan',
    title: 'Workshop plan',
    tags: ['batch solve', 'pools', 'groups', 'calendar', 'scores'],
    description:
      'Five activities, three rooms and three trainers on a two-day slot calendar, solved with branch and bound. Rooms and trainers come from pools; the timeline shows the resulting intervals per resource and person. Several plans reach the best score, so a rerun can pick a different one with the same score.',
  },
  {
    slug: 'relations-and-breaks',
    name: 'relations_and_breaks',
    title: 'Relations and breaks',
    tags: ['relations', 'breaks', 'availability', 'unreleased'],
    description:
      'One training day with a lunch break, a trainer who is unavailable in the morning, and SameStart, Consecutive, Precedence and NoOverlap relations. These features are on master and not yet in the 0.8.0 release.',
  },
  {
    slug: 'explain-conflicts',
    name: 'explain_conflicts',
    title: 'Explaining conflicts',
    tags: ['explain', 'infeasible', 'compile errors'],
    description:
      'What schedulr reports when a plan cannot work: blocking and advisory conflicts from explain(), and the collected CompileError messages for invalid input.',
  },
  {
    slug: 'booking-desk',
    name: 'booking_desk',
    title: 'Booking desk',
    tags: ['SchedulingState', 'no search', 'move', 'cancel'],
    description:
      'Create, check, move and cancel single appointments with SchedulingState. Each check evaluates only the constraints touching the proposal; no solver search runs.',
  },
];

export const examples: ShowcaseExample[] = catalogue.map(({ name, ...meta }) => {
  const source = sources[`../../examples/${name}.rs`];
  const output = outputs[`../../examples/output/${name}.txt`];
  if (source === undefined || output === undefined) {
    throw new Error(`Missing example source or captured output for ${name}`);
  }
  return {
    ...meta,
    file: `examples/${name}.rs`,
    input: { code: source, lang: 'rust' },
    output: { html: ansiToHtml(output), kind: 'terminal' },
  };
});
