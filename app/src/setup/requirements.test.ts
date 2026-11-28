import { expect, test } from 'vitest';
import { diskDetail, setupNeeds, setupTime, type WorkspaceRequirements } from './requirements';

const base: WorkspaceRequirements = { memoryBytes: 6442450944, stateDiskBytes: 34359738368, downloadBytes: 1967572914, setupDiskBytes: 9453699072, freeDiskBytes: 13250830695, readySeconds: null };

test('setup time names the download and adds a start time only where one was measured', () => {
  expect(setupTime(base)).toBe('Mostly a 2.0 GB download');
  expect(setupTime({ ...base, readySeconds: 40 })).toBe('Mostly a 2.0 GB download, then about 1 minute to start');
  expect(setupTime({ ...base, readySeconds: 10 })).toBe('Mostly a 2.0 GB download, then about 1 minute to start');
  expect(setupTime({ ...base, readySeconds: 134 })).toBe('Mostly a 2.0 GB download, then about 2 minutes to start');
  expect(setupTime({ ...base, downloadBytes: null, readySeconds: 134 })).toBe('Mostly a download');
  expect(setupTime(undefined)).toBe('Mostly a download');
});

test('the card gives one disk figure, the free disk setup needs, whenever it was measured', () => {
  expect(setupNeeds(base)).toBe('Downloads about 2.0 GB. Needs virtualization, 6.4 GB of memory and 13.3 GB of free disk.');
  expect(setupNeeds({ ...base, freeDiskBytes: null })).toBe('Downloads about 2.0 GB. Needs virtualization and 6.4 GB of memory.');
  expect(setupNeeds({ ...base, setupDiskBytes: null })).toBe(setupNeeds(base));
  // 24.21 GB would round down to 24.2; the need rounds up instead.
  expect(setupNeeds({ ...base, freeDiskBytes: 24210000000 })).toContain('24.3 GB of free disk');
  expect(setupNeeds({ ...base, freeDiskBytes: 24200000000 })).toContain('24.2 GB of free disk');
});

test('the kept disk and data-disk growth stay in the detail', () => {
  expect(diskDetail(base)).toBe('About 9.5 GB stays in use after setup. Data disk can grow to 34.4 GB.');
  expect(diskDetail({ ...base, setupDiskBytes: null })).toBe('Data disk can grow to 34.4 GB.');
  expect(diskDetail({ ...base, freeDiskBytes: null })).toBe(diskDetail(base));
});

test('a download already on this computer is named as such, not as a download to come', () => {
  const measured = { memoryBytes: 6 * 1024 ** 3, stateDiskBytes: 32 * 1024 ** 3, downloadBytes: 2_000_000_000, setupDiskBytes: null, freeDiskBytes: 13_300_000_000, readySeconds: 134 };
  expect(setupTime(measured, true)).toBe('Already downloaded, then about 2 minutes to start');
  expect(setupTime({ ...measured, readySeconds: null }, true)).toBe('Already downloaded');
  expect(setupTime(measured)).toBe('Mostly a 2.0 GB download, then about 2 minutes to start');
  expect(setupNeeds(measured, true)).toBe('Already downloaded. Needs virtualization, 6.4 GB of memory and 13.3 GB of free disk.');
  expect(setupNeeds(measured)).toBe('Downloads about 2.0 GB. Needs virtualization, 6.4 GB of memory and 13.3 GB of free disk.');
});
