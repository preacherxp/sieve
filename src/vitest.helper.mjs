// Sieve's Vitest query (ADR 0006): which test files relate to the changed files, and which
// changed files reach a runner-loaded file. Usage:
//   node vitest.helper.mjs WORKSPACE [CHANGED_FILE...] -- [VITEST_ARGS...]
// Prints {"tests": [...], "reached": [...]} with absolute paths. With no changed files it lists
// every test file instead, for `sieve init`.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [workspace, ...rest] = process.argv.slice(2);
const separator = rest.indexOf('--');
const changed = rest.slice(0, separator);
const args = rest.slice(separator + 1);
// The project's own Vitest, as its tests run with.
const require = createRequire(resolve(workspace, 'package.json'));
const { createVitest, parseCLI } = await import(pathToFileURL(require.resolve('vitest/node')).href);
const { options, filter } = parseCLI(['vitest', ...args]);
const listing = changed.length === 0;
const vitest = await createVitest(
  'test',
  { ...options, watch: false, reporters: [], ...(listing ? {} : { related: changed }) },
  {},
  {},
);
try {
  const tests = (await vitest.getRelevantTestSpecifications(filter)).map((s) => s.moduleId);
  const reached = new Set();
  if (!listing) {
    const loaded = [];
    for (const project of vitest.projects) {
      const files = [...project.config.setupFiles, ...(project.config.globalSetup ?? [])];
      const environment = project.config.environment;
      if (/^\.{0,2}\//.test(environment)) files.push(resolve(project.config.root, environment));
      loaded.push(...files.map((file) => project.createSpecification(file)));
      // The configuration file and everything it imports.
      for (const file of project.vite.config.configFileDependencies ?? []) {
        if (changed.includes(resolve(file))) reached.add(resolve(file));
      }
    }
    // ponytail: private Vitest API (the walk getRelevantTestSpecifications uses); if a release
    // removes it this throws, and Sieve runs every test.
    if (typeof vitest.specifications?.filterTestsBySource !== 'function') {
      throw new Error('this Vitest version has no specifications.filterTestsBySource');
    }
    // Setup files are force-rerun triggers already; the walk must see their imports instead.
    vitest.config.forceRerunTriggers = [];
    for (const spec of await vitest.specifications.filterTestsBySource(loaded)) {
      reached.add(spec.moduleId);
    }
  }
  console.log(JSON.stringify({ tests: [...new Set(tests)], reached: [...reached] }));
} finally {
  await vitest.close();
}
