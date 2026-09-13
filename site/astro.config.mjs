// @ts-check
import casoonPages from '@casoon/pages-theme';
import { defineConfig } from 'astro/config';

// Project page: https://casoon.github.io/schedulr/ — `base` is the GitHub Pages path.
export default defineConfig({
  site: 'https://casoon.github.io/schedulr',
  base: '/schedulr/',
  integrations: [
    casoonPages({
      name: 'schedulr',
      description:
        'Scheduling framework for Rust: describe activities, resources and time windows, then solve, check or repair the plan with the unifier constraint solver.',
      repo: 'casoon/schedulr',
      version: '0.8.0',
      license: 'MIT',
      branch: 'master',
      packages: [
        { label: 'crates.io', href: 'https://crates.io/crates/schedulr' },
        { label: 'docs.rs', href: 'https://docs.rs/schedulr' },
      ],
      docsGroups: {
        'getting-started': 'Getting started',
        guides: 'Guides',
        concepts: 'Concepts',
        reference: 'Reference',
      },
      // schedulr has no CHANGELOG.md and no git tags yet.
      changelog: false,
    }),
  ],
});
