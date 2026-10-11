import { defineConfig } from 'vitest/config';
export default defineConfig({ test: { globals: true, projects: [
  { test: { name: 'unit', globals: true, dir: './src', setupFiles: ['src/setupTests.ts'], include: ['**/*.test.ts'] } },
  { test: { name: 'other', globals: true, dir: './test', setupFiles: ['src/setupTests.ts'], include: ['**/*.test.ts'] } } ] } });
