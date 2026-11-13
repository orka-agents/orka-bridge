import { useEffect } from 'react';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import type { RunSnapshot } from './designer/run-api';
import App from './App';

const result = vi.hoisted(() => ({ snapshot: undefined as RunSnapshot | undefined }));
// Inject the Designer callback boundary; exercise App, Setup and Steps together.
vi.mock('./designer/Example', () => ({ default: function Example({ onRun }: { onRun: (value: RunSnapshot | undefined, documentId: string) => void }) {
  useEffect(() => { onRun(result.snapshot, result.snapshot?.documentId ?? '9b2b93c4-375c-4815-9717-39852a96e411'); }, [onRun]);
  return <h1>Agent</h1>;
} }));
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test.each(['unavailable', 'other', 'sample'] as const)('setup keeps %s Run evidence separate from workspace readiness', async kind => {
  const sample = '9b2b93c4-375c-4815-9717-39852a96e411';
  result.snapshot = kind === 'unavailable' ? undefined : {
    documentId: kind === 'sample' ? sample : '00000000-0000-4000-8000-000000000001', busy: null,
    state: 'complete', canRun: true, report: { verdict: 'passed', cases: [] }, acknowledged: [], pending: null, error: null,
  };
  vi.stubGlobal('isTauri', true); localStorage.setItem('bridge.welcomeSeen.v1', '1');
  mockIPC(command => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'setup_example') return { agent: '---\nname: web-reader\n---\nRead.', tests: 'Checks' };
    throw Error('unavailable');
  });
  const user = userEvent.setup(); render(<App />);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  await user.click(await screen.findByRole('button', { name: 'Read its instructions' }));
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await waitFor(() => expect(screen.getByRole('list', { name: 'Setup steps' }).children[4].textContent)
    .toContain({ unavailable: 'Result unknown', other: 'Needs you', sample: 'Passed' }[kind]));
  const navigation = within(screen.getByRole('navigation'));
  expect(navigation.getByText(kind === 'sample' ? 'This computer ready' : 'Setting up, step 5 of 5')).toBeTruthy();
});
