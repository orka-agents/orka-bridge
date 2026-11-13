import { openLocalSetup } from './test-navigation';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import Setup from './Setup';

function native() {
  vi.stubGlobal('isTauri', true);
  let selected: 'localVm' | null = 'localVm';
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected, localVm: selected ? 'ready' : 'notIntegrated', model: selected ? 'available' : 'locked' };
    if (command === 'setup_requirements') return null;
    if (command === 'setup_connect_model') return { ready: true };
    if (command === 'setup_change_local') { selected = null; return { selected, localVm: 'notIntegrated', model: 'locked' }; }
    if (command === 'cluster_contexts') return [];
    throw Error('Unexpected command: ' + command);
  });
  mockIPC(ipc); return ipc;
}
beforeEach(() => localStorage.setItem('bridge.welcomeSeen.v1', '1'));
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

test('AI connection is the only next primary and its prerequisite is visible before clicking', async () => {
  const ipc = native(); const user = userEvent.setup(); render(<App />); await openLocalSetup();
  const connect = await screen.findByRole('button', { name: 'Connect the AI' });
  expect(connect.className).toBe('workspace-primary');
  expect(screen.getByText(/GitHub CLI installed and signed in/)).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Open example' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Change where agents run' }).className).toBe('workspace-link');
  await user.click(connect);
  expect(await screen.findByText('Connected with GitHub Copilot.')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Open your first test' }).className).toBe('workspace-primary');
  expect(document.body.textContent).not.toMatch(/guest|\bVM\b|inference|gpt-5.6-terra/);
  expect(screen.queryByText('Engine diagnostics')).toBeNull();
  expect(ipc.mock.calls.some(([command]) => command === 'engine_call')).toBe(false);
});

test.each(['Run the test again', 'Open your test result'])('%s selects this computer before its guarded local handoff, without starting a VM', async action => {
  const ipc = native(); const user = userEvent.setup(); const calls: string[] = [];
  render(<Setup place={{ kind: 'cluster' }} firstCheck="stale" firstTestResult={{ failed: 0 }}
    onPlace={place => calls.push(place.kind)} onOpenExample={() => calls.push('Designer')} />);
  await openLocalSetup(user);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  expect(calls).toEqual(['thisComputer']);
  calls.length = 0;
  await user.click(await screen.findByRole('button', { name: action }));
  expect(calls).toEqual(['thisComputer', 'Designer']);
  expect(ipc.mock.calls.map(([command]) => command)).not.toContain('setup_select_local');
});

test('Change focuses its explanation, makes Keep primary and restores focus without stopping', async () => {
  const ipc = native(); const user = userEvent.setup(); render(<App />); await openLocalSetup();
  const change = await screen.findByRole('button', { name: 'Change where agents run' });
  await user.click(change);
  const panel = screen.getByRole('region', { name: 'Change where agents run' });
  expect(panel.contains(document.activeElement)).toBe(true);
  expect(within(panel).getByText(/downloaded workspace files and agent files are kept/)).toBeTruthy();
  const keep = within(panel).getByRole('button', { name: 'Keep workspace running' });
  expect(keep.className).toBe('workspace-primary');
  expect(screen.getByRole('button', { name: 'Connect the AI' }).className).toBe('workspace-secondary');
  expect(within(panel).getByRole('button', { name: 'Stop workspace and choose a place' }).className).toBe('workspace-secondary');
  await user.click(keep);
  expect(document.activeElement).toBe(change);
  expect(screen.getByRole('button', { name: 'Connect the AI' }).className).toBe('workspace-primary');
  expect(ipc.mock.calls.some(([command]) => command === 'setup_change_local')).toBe(false);
});

test('cluster picking is explicitly separate from stopping and does not stop the local workspace', async () => {
  const ipc = native(); const user = userEvent.setup(); render(<App />); await openLocalSetup();
  await user.click(await screen.findByRole('button', { name: 'Change where agents run' }));
  await user.click(screen.getByRole('button', { name: 'Choose a cluster, keep this workspace running' }));
  expect(await screen.findByRole('combobox', { name: 'kubectl context' })).toBeTruthy();
  expect(ipc.mock.calls.some(([command]) => command === 'setup_change_local')).toBe(false);
});
