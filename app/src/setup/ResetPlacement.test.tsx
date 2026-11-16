import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import Setup from './Setup';

afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });
test('AI recovery is on the failing step and the same running-place card, preserving pending reset across navigation', async () => {
  vi.stubGlobal('isTauri', true); localStorage.setItem('bridge.welcomeSeen.v1', '1');
  let finish!: (value: unknown) => void;
  const ipc = vi.fn((command: string) => {
    if (command === 'setup_status') return { selected: 'localVm', localVm: 'ready', model: 'available' };
    if (command === 'setup_requirements') return null;
    if (command === 'cluster_places') return { places: [], selectedKey: null, persistenceState: 'complete' };
    if (command === 'setup_connect_model') throw { kind: 'modelInstallationMismatch', recovery: {
      installation: '10000000-0000-4000-8000-000000000001', savedInstallation: '20000000-0000-4000-8000-000000000002', credentialSlot: '30000000-0000-4000-8000-000000000003',
    } };
    if (command === 'setup_reset_model') return new Promise(resolve => { finish = resolve; });
    throw { kind: 'unavailable' };
  });
  mockIPC(ipc); const user = userEvent.setup(); render(<Setup onOpenExample={() => {}} />);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' })); // Places' navigation action
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  expect(await screen.findByRole('heading', { name: "The AI couldn't connect" })).toBeTruthy();
  const row = screen.getByText('Choose the AI your agent uses').closest('li')!;
  expect(within(row).getByRole('button', { name: 'Reset AI connection' })).toBeTruthy();
  expect(screen.getByText('The local cluster is running on this computer.')).toBeTruthy();
  await user.click(within(row).getByRole('button', { name: 'Show details' }));
  await user.click(screen.getByRole('button', { name: 'Back to clusters' }));
  const place = screen.getByRole('region', { name: 'This computer' });
  expect(within(place).getByRole('button', { name: 'Reset AI connection' })).toBeTruthy();
  expect(within(place).getByRole('button', { name: 'Hide details' })).toBeTruthy();
  expect(within(place).getByText('Workspace running')).toBeTruthy();
  await user.click(within(place).getByRole('button', { name: 'Reset AI connection' }));
  await user.click(within(place).getByRole('button', { name: 'Reset' }));
  await user.click(within(place).getByRole('button', { name: 'Connect the AI' }));
  const heading = screen.getByRole('heading', { level: 1 }); heading.focus();
  await act(async () => { finish({ ready: false }); });
  expect(screen.getByText('Connection reset. Choose the AI again.')).toBeTruthy();
  expect(ipc.mock.calls.filter(([name]) => name === 'setup_reset_model')).toHaveLength(1);
  expect(document.activeElement).toBe(heading);
});
