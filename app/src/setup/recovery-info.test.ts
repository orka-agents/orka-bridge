import { expect, test } from 'vitest';
import { helpDetails, isWorkspaceFailure, workspaceRecovery, type WorkspaceFailure } from './recovery-info';

const failure = (cause: WorkspaceFailure['cause'], operation: WorkspaceFailure['operation'] = 'start', codes: number[] = []): WorkspaceFailure => ({ cause, operation, codes });

test('workspace metadata only accepts finite categories and at most four unsigned native codes', () => {
  expect(isWorkspaceFailure(failure('permission', 'start', [0, 0xffffffff]))).toBe(true);
  for (const value of [null, [], {}, failure('unknown', 'start', [-1]), failure('unknown', 'start', [1.5]),
    failure('unknown', 'start', [0x100000000]), failure('unknown', 'start', [1, 2, 3, 4, 5]),
    { ...failure('unknown'), operation: 'PRIVATE' }, { ...failure('unknown'), cause: 'PRIVATE' },
    { ...failure('unknown'), secret: 'PRIVATE' }]) expect(isWorkspaceFailure(value)).toBe(false);
});

test('cause and operation select plain copy without interpreting diagnostics', () => {
  const denied = workspaceRecovery(failure('permission'));
  expect(denied.summary).toContain('operating system denied');
  expect(denied.heading).toBe("Your workspace couldn't start");
  const download = workspaceRecovery(failure('network', 'download'));
  expect(download.heading).toBe("Your workspace files couldn't be downloaded");
  expect(download.preservation).toContain('verified');
  expect(download.preservation).not.toContain('What you downloaded is kept');
  expect(workspaceRecovery(failure('preparation', 'prepare')).preservation).toContain('copies');
  expect(workspaceRecovery(failure('stopUnconfirmed', 'stop')).retryBlocked).toBe(true);
  expect(workspaceRecovery().summary).toContain("couldn't establish");
});

test('copied help is fixed labels and accepted codes, not a free-form summary or diagnostic', () => {
  const info = workspaceRecovery(failure('permission', 'start', [5]));
  const copied = helpDetails(info, [5]);
  expect(copied).toContain('Operation: Starting workspace');
  expect(copied).toContain('Category: permission');
  expect(copied).toContain('OS code: 5');
  expect(copied).not.toContain(info.summary);
});
