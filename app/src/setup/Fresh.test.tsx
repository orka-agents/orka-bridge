import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import App from '../App';
import Model from './Model';
import { type FreshStatus, freshDetails, isFreshPlan, isFreshStatus } from './fresh';
import { formatNeededGB, formatRequirementGB } from './requirements';
import type { SetupStatus } from './state';
import { openLocalSetup } from './test-navigation';

const transaction = '40000000-0000-4000-8000-000000000004';
const retired = 'C:\\Users\\you\\AppData\\Local\\ai.orka.bridge\\discarded-40000000-0000-4000-8000-000000000004';
const refused = { kind: 'modelReconnect', message: 'SECRET', cleanup: ['disconnectRetainedModel', 'refused'] };
let status: SetupStatus;
let fresh: () => Promise<FreshStatus>;
let refusals: unknown[] = [];
const ipc = vi.fn((command: string): unknown => {
  if (command === 'setup_status') return status;
  if (command === 'setup_requirements') return null;
  if (command === 'setup_connect_model' || command === 'setup_remove_model') throw refusals.shift() ?? refused;
  if (command === 'setup_check_model') return { account: 'octocat', models: [{ id: 'gpt-4.1' }], preselected: 'gpt-4.1' };
  if (command === 'setup_select_local') return status = { selected: 'localVm', localVm: 'starting', model: 'locked', modelSupported: true };
  if (command === 'workspace_fresh_start') return fresh();
  if (command === 'agents_list') return [];
  if (command === 'agent_creation_root') return '/agents';
  if (command === 'cluster_contexts') return [];
  throw Error('Unexpected command: ' + command);
});
let clipboard = '';
// user-event installs its own clipboard; watch what Bridge writes to it.
function watchCopies() { vi.spyOn(navigator.clipboard, 'writeText').mockImplementation(async text => { clipboard = text; }); }
beforeEach(() => {
  vi.stubGlobal('isTauri', true); localStorage.setItem('bridge.welcomeSeen.v1', '1');
  localStorage.setItem('bridge.firstTest.v1', 'attempted');
  status = { selected: 'localVm', localVm: 'ready', model: 'available', modelSupported: true, workspaceFreshPlan: { image: { kind: 'cached' } } };
  refusals = [];
  fresh = async () => ({ state: 'replaced', transaction, ai: 'removed', reclaimable: [retired], cleaning: true });
  clipboard = ''; ipc.mockClear(); mockIPC(ipc);
});
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); localStorage.clear(); });
const calls = (name: string) => ipc.mock.calls.filter(([command]) => command === name);
async function observe(value: SetupStatus) { status = value; await act(async () => window.dispatchEvent(new Event('focus'))); }

async function stuck() {
  const user = userEvent.setup(); watchCopies();
  render(<App />);
  await openLocalSetup(user);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  // The first refusal offers Check again once.
  expect((await screen.findByRole('alert')).textContent).toContain("Bridge couldn't remove the previous AI connection from the workspace.");
  expect(screen.queryByRole('button', { name: 'Start a fresh workspace' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  // The second goes straight to the way out, instead of another round.
  await screen.findByRole('button', { name: 'Start a fresh workspace' });
  return user;
}

test('the stuck AI card leads to one confirmation, then the ordinary first-setup screen', async () => {
  const user = await stuck();
  const alert = screen.getByRole('alert');
  expect(alert.textContent).toContain('A fresh workspace is the sure way back: Bridge replaces the workspace on this computer, keeps your agent files, and you connect the AI again.');
  expect(alert.textContent).not.toContain("can't start one from this card yet");
  expect(screen.queryByRole('button', { name: 'Check again' })).toBeNull();
  expect(screen.queryByRole('button', { name: /^Remove (sign-in|saved sign-in)$/ })).toBeNull();
  expect(screen.getByRole('button', { name: 'Start a fresh workspace' }).classList.contains('button-primary')).toBe(true);
  // One confirmation, in the glossary's words, with the keeping action first.
  await user.click(screen.getByRole('button', { name: 'Start a fresh workspace' }));
  const confirm = await screen.findByRole('region', { name: 'Start a fresh workspace?' });
  expect(document.activeElement).toBe(within(confirm).getByRole('heading', { name: 'Start a fresh workspace?' }));
  expect(within(confirm).getAllByRole('listitem').map(item => item.textContent)).toEqual([
    'The workspace on this computer is replaced with a new one. Bridge stops it first.',
    'Your agent files are kept, and so are your test results and Send history.',
    'Bridge removes its saved AI sign-in or key, and you connect the AI again.',
    'Anything that exists only inside the workspace is lost: the agents on the local cluster, and the tasks run there with their answers, including Chat and Try a task conversations. Running your tests again puts your agents back on the local cluster.',
    'Bridge reuses the download already on this computer.',
  ]);
  expect(within(confirm).getAllByRole('button').map(button => button.textContent)).toEqual(['Keep this workspace', 'Start a fresh workspace']);
  await user.click(within(confirm).getByRole('button', { name: 'Keep this workspace' }));
  expect(screen.queryByRole('region', { name: 'Start a fresh workspace?' })).toBeNull();
  expect(calls('workspace_fresh_start')).toHaveLength(0);
  // Confirmed: the replacement shows while it runs, and the renderer sends nothing with it.
  let finish!: (value: FreshStatus) => void;
  fresh = () => new Promise(resolve => { finish = resolve; });
  await user.click(screen.getByRole('button', { name: 'Start a fresh workspace' }));
  await user.click(within(await screen.findByRole('region', { name: 'Start a fresh workspace?' })).getByRole('button', { name: 'Start a fresh workspace' }));
  const replacing = await screen.findByRole('region', { name: 'Starting a fresh workspace' });
  expect(within(replacing).getByRole('status').textContent).toBe('Stopping the workspace and setting it aside. Your agent files are kept.');
  expect(calls('workspace_fresh_start')).toEqual([['workspace_fresh_start', {}]]);
  status = { selected: 'localVm', localVm: 'installing', model: 'locked', modelSupported: true, workspaceProgress: { phase: 'verifying', bytes: null, totalBytes: null, elapsedMs: 1000, remainingMs: null },
    workspaceFresh: { state: 'replaced', transaction, ai: 'removed', reclaimable: [retired], cleaning: true } };
  await act(async () => { finish({ state: 'replaced', transaction, ai: 'removed', reclaimable: [retired], cleaning: true }); });
  // The ordinary first-setup screen: the workspace getting ready beside the AI card, which can check Copilot early.
  const setup = await screen.findByRole('region', { name: 'Workspace setup' });
  expect(within(setup).getByRole('progressbar', { name: 'Workspace setup progress' })).toBeTruthy();
  expect(screen.queryByRole('region', { name: 'Workspace' })).toBeNull();
  expect(screen.getByRole('heading', { name: 'This computer', level: 1 })).toBeTruthy();
  expect(screen.getByRole('radio', { name: 'GitHub Copilot' })).toBeTruthy();
  expect((screen.getByRole('button', { name: 'Connect the AI' }) as HTMLButtonElement).disabled).toBe(false);
  const done = screen.getByRole('region', { name: 'Fresh workspace' });
  expect(within(done).getByRole('status').textContent).toBe('Fresh workspace started. Your agent files are kept. Connect the AI again when the workspace is ready.');
  expect(within(done).getByText('Deleting the previous workspace…')).toBeTruthy();
  expect(screen.queryByText("Bridge couldn't remove the previous AI connection from the workspace.", { exact: false })).toBeNull();
  // Deleted in the background: the note goes quiet once it is done.
  await observe({ ...status, workspaceFresh: { state: 'replaced', transaction, ai: 'removed', reclaimable: [], cleaning: false } });
  await waitFor(() => expect(screen.queryByText('Deleting the previous workspace…')).toBeNull());
  expect(screen.queryByText(/delete this folder/)).toBeNull();
  // The new workspace runs its first test automatically again once the AI is connected.
  expect(localStorage.getItem('bridge.firstTest.v1')).toBe('pending');
});

test('a stop that cannot be confirmed refuses and says the workspace is as it was', async () => {
  fresh = async () => { status = { selected: 'localVm', localVm: 'failed', model: 'locked', localVmError: 'Stopping the local VM failed', workspaceFailure: { operation: 'stop', cause: 'stopUnconfirmed', codes: [] }, workspaceFresh: { state: 'refused', reason: 'stopUnconfirmed', restarted: false } }; return { state: 'refused', reason: 'stopUnconfirmed', restarted: false }; };
  const user = await stuck();
  await user.click(screen.getByRole('button', { name: 'Start a fresh workspace' }));
  await user.click(within(await screen.findByRole('region', { name: 'Start a fresh workspace?' })).getByRole('button', { name: 'Start a fresh workspace' }));
  const heading = await screen.findByRole('heading', { name: "Bridge didn't start a fresh workspace" });
  const card = heading.closest('section')!;
  expect(card.textContent).toContain("Bridge couldn't confirm the workspace stopped, so it didn't replace it. Quit Bridge and reopen it, then try again.");
  expect(card.textContent).toContain('The workspace on this computer is as it was. Nothing was moved or deleted.');
  await user.click(within(card).getByRole('button', { name: 'Copy details for help' }));
  expect(clipboard).toBe('Bridge fresh workspace\nFresh workspace: refused (stopUnconfirmed)');
  await user.click(within(card).getByRole('button', { name: 'Done' }));
  expect(screen.queryByRole('heading', { name: "Bridge didn't start a fresh workspace" })).toBeNull();
});

test('a workspace with different cluster files offers a fresh workspace beside Open the matching Bridge version', async () => {
  status = { selected: 'localVm', localVm: 'failed', model: 'locked', localVmError: 'Image(WorkspaceVersionUnsupported)', workspaceFailure: { operation: 'verify', cause: 'workspaceVersionUnsupported', codes: [] } };
  const user = userEvent.setup();
  render(<App />);
  await openLocalSetup(user);
  const card = await screen.findByRole('region', { name: 'Workspace' });
  expect(within(card).getByText('Open the matching Bridge version.', { selector: 'p' })).toBeTruthy();
  expect(within(card).getByText('Or start a fresh workspace on this computer. Your agent files are kept.')).toBeTruthy();
  const offer = within(card).getByRole('button', { name: 'Start a fresh workspace' });
  expect(offer.classList.contains('button-secondary')).toBe(true);
  expect((within(card).getByRole('button', { name: 'Restart workspace' }) as HTMLButtonElement).disabled).toBe(true);
  await user.click(offer);
  expect(await screen.findByRole('region', { name: 'Start a fresh workspace?' })).toBeTruthy();
});

test('an ordinary start failure offers a fresh workspace only after the same failure follows a retry', async () => {
  const failed: SetupStatus = { selected: 'localVm', localVm: 'failed', model: 'locked', localVmError: 'Protocol(ConnectionReset)', workspaceFailure: { operation: 'start', cause: 'guestConnection', codes: [] } };
  status = failed;
  const user = userEvent.setup();
  render(<App />);
  await openLocalSetup(user);
  const card = await screen.findByRole('region', { name: 'Workspace' });
  expect(within(card).queryByRole('button', { name: 'Start a fresh workspace' })).toBeNull();
  await user.click(within(card).getByRole('button', { name: 'Restart workspace' }));
  await observe(failed);
  expect(await within(card).findByRole('button', { name: 'Start a fresh workspace' })).toBeTruthy();
  expect(within(card).getByText('If it keeps failing, start a fresh workspace on this computer. Your agent files are kept.')).toBeTruthy();
  // A different failure afterwards is a new one, with its own Try again first.
  await observe({ ...failed, workspaceFailure: { operation: 'services', cause: 'services', codes: [] } });
  await waitFor(() => expect(within(card).queryByRole('button', { name: 'Start a fresh workspace' })).toBeNull());
});

test('Manage keeps a quiet Start a fresh workspace in the workspace details, and reports space left to reclaim', async () => {
  const user = userEvent.setup();
  render(<App />);
  await openLocalSetup(user);
  const card = await screen.findByRole('region', { name: 'Workspace' });
  expect(within(card).queryByRole('button', { name: 'Start a fresh workspace…' })).toBeNull();
  await user.click(within(card).getByRole('button', { name: "What's inside" }));
  const quiet = within(card).getByRole('button', { name: 'Start a fresh workspace…' });
  expect(quiet.classList.contains('button-quiet')).toBe(true);
  fresh = async () => { status = { selected: 'localVm', localVm: 'preparing', model: 'locked', modelSupported: true, workspaceFresh: { state: 'replaced', transaction, ai: 'kept', reclaimable: [retired], cleaning: false } }; return status.workspaceFresh!; };
  await user.click(quiet);
  await user.click(within(await screen.findByRole('region', { name: 'Start a fresh workspace?' })).getByRole('button', { name: 'Start a fresh workspace' }));
  const done = await screen.findByRole('region', { name: 'Fresh workspace' });
  expect(done.textContent).toContain("Bridge couldn't confirm its saved sign-in or key was removed. When you connect the AI, choose Reset AI connection to try again.");
  expect(done.textContent).toContain("Bridge couldn't delete the previous workspace. It may still hold that workspace's copy of your sign-in. To remove it and free its space, quit Bridge and delete this folder:");
  expect(within(done).getByText(retired, { selector: 'code' })).toBeTruthy();
  await user.click(within(done).getByRole('button', { name: 'Done' }));
  expect(screen.queryByRole('region', { name: 'Fresh workspace' })).toBeNull();
});

test('removal refused twice also leads to a fresh workspace, and copied details name the outcome', async () => {
  const user = userEvent.setup(); watchCopies();
  const ask = vi.fn();
  vi.stubGlobal('isTauri', true);
  render(<Model enabled helpLines={freshDetails({ state: 'replaced', transaction, ai: 'removed', reclaimable: [retired], cleaning: false })} onFreshWorkspace={ask} />);
  await user.click(screen.getByRole('button', { name: 'Connect the AI' }));
  await screen.findByRole('alert');
  await user.click(screen.getByRole('button', { name: 'Remove sign-in' }));
  await user.click(within(screen.getByRole('group', { name: 'Remove sign-in confirmation' })).getByRole('button', { name: 'Remove sign-in' }));
  await screen.findByRole('heading', { name: "Couldn't remove the saved sign-in" });
  expect(screen.queryByRole('button', { name: 'Retry removal' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Start a fresh workspace' }));
  expect(ask).toHaveBeenCalledOnce();
  await user.click(screen.getByRole('button', { name: 'Copy details for help' }));
  expect(clipboard).toContain('Operation: Removing sign-in\nCategory: modelReconnect\nCleanup operation: disconnectRetainedModel\nCleanup class: refused');
  expect(clipboard).toContain(`Fresh workspace: replaced (${transaction})\nFresh workspace AI sign-in: removed\nFresh workspace cleanup: 1 folder left to delete`);
  expect(clipboard).not.toContain('AppData');
});

test('fresh workspace replies are exact', () => {
  expect(isFreshStatus({ state: 'replacing' })).toBe(true);
  expect(isFreshStatus({ state: 'replacing', path: '/x' })).toBe(false);
  expect(isFreshStatus({ state: 'refused', reason: 'stopUnconfirmed', restarted: false })).toBe(true);
  expect(isFreshStatus({ state: 'refused', reason: 'stopUnconfirmed' })).toBe(false);
  expect(isFreshStatus({ state: 'refused', reason: 'noSpace', restarted: false, neededBytes: 7 })).toBe(true);
  expect(isFreshStatus({ state: 'refused', reason: 'offline', restarted: false, neededBytes: 7 })).toBe(false);
  expect(isFreshStatus({ state: 'refused', reason: 'layout', restarted: true, reclaimable: [retired] })).toBe(true);
  expect(isFreshStatus({ state: 'refused', reason: 'layout', restarted: true, reclaimable: [] })).toBe(false);
  expect(isFreshStatus({ state: 'interrupted', transaction })).toBe(true);
  expect(isFreshStatus({ state: 'interrupted', transaction: 'x' })).toBe(false);
  expect(isFreshStatus({ state: 'conflict', found: [retired] })).toBe(true);
  expect(isFreshStatus({ state: 'conflict', found: [] })).toBe(false);
  expect(isFreshStatus({ state: 'replaced', transaction, ai: 'unconfirmed', reclaimable: [], cleaning: false })).toBe(true);
  expect(isFreshPlan({ image: { kind: 'cached' } })).toBe(true);
  expect(isFreshPlan({ blocked: 'updateHistory', image: { kind: 'download', bytes: 9 } })).toBe(true);
  expect(isFreshPlan({ image: { kind: 'download' } })).toBe(true);
  expect(isFreshPlan({ image: { kind: 'download', bytes: -1 } })).toBe(false);
  expect(isFreshPlan({ blocked: 'nope', image: { kind: 'cached' } })).toBe(false);
  expect(isFreshPlan({ image: { kind: 'cached' }, extra: 1 })).toBe(false);
  expect(isFreshStatus({ state: 'refused', reason: 'elsewhere' })).toBe(false);
  expect(isFreshStatus({ state: 'replaced', transaction, ai: 'removed', reclaimable: [], cleaning: false })).toBe(true);
  expect(isFreshStatus({ state: 'replaced', transaction: 'nope', ai: 'removed', reclaimable: [], cleaning: false })).toBe(false);
  expect(isFreshStatus({ state: 'replaced', transaction, ai: 'removed', reclaimable: [7], cleaning: false })).toBe(false);
  expect(isFreshStatus({ state: 'replaced', transaction, ai: 'removed', reclaimable: [], cleaning: false, extra: 1 })).toBe(false);
});

test('the confirmation says when first install downloads the workspace again, and how much', async () => {
  status = { ...status, workspaceFreshPlan: { image: { kind: 'download', bytes: 2_380_070_756 } } };
  const user = userEvent.setup();
  render(<App />);
  await openLocalSetup(user);
  const card = await screen.findByRole('region', { name: 'Workspace' });
  await user.click(within(card).getByRole('button', { name: "What's inside" }));
  await user.click(within(card).getByRole('button', { name: 'Start a fresh workspace…' }));
  const confirm = await screen.findByRole('region', { name: 'Start a fresh workspace?' });
  expect(within(confirm).getAllByRole('listitem').at(-1)!.textContent).toBe(`Bridge downloads the workspace again: about ${formatRequirementGB(2_380_070_756)}.`);
});

test('a download it cannot make is refused before anything changes, and a restart is said', async () => {
  fresh = async () => ({ state: 'refused', reason: 'noSpace', restarted: false, neededBytes: 19_560_000_000 });
  const user = await stuck();
  await user.click(screen.getByRole('button', { name: 'Start a fresh workspace' }));
  await user.click(within(await screen.findByRole('region', { name: 'Start a fresh workspace?' })).getByRole('button', { name: 'Start a fresh workspace' }));
  const card = (await screen.findByRole('heading', { name: "Bridge didn't start a fresh workspace" })).closest('section')!;
  expect(card.textContent).toContain(`Bridge needs to download the workspace again and this computer doesn't have enough free space: it needs about ${formatNeededGB(19_560_000_000)} free. Free up space, then try again.`);
  expect(card.textContent).toContain('The workspace on this computer is as it was. Nothing was moved or deleted.');
  await user.click(within(card).getByRole('button', { name: 'Done' }));
  // A refusal found after the stop restarts the workspace and says so, keeping folders left to delete.
  fresh = async () => ({ state: 'refused', reason: 'inUse', restarted: true, reclaimable: [retired] });
  await user.click(screen.getByRole('button', { name: 'Start a fresh workspace' }));
  await user.click(within(await screen.findByRole('region', { name: 'Start a fresh workspace?' })).getByRole('button', { name: 'Start a fresh workspace' }));
  const again = (await screen.findByRole('heading', { name: "Bridge didn't start a fresh workspace" })).closest('section')!;
  expect(again.textContent).toContain('The workspace on this computer is as it was. Nothing was moved or deleted, and Bridge started it again.');
  expect(within(again).getByText(retired, { selector: 'code' })).toBeTruthy();
});

test('a transaction that was set aside but not finished says so and finishes on the next try', async () => {
  const interrupted: FreshStatus = { state: 'interrupted', transaction };
  status = { selected: 'localVm', localVm: 'failed', model: 'locked', localVmError: 'Bridge must finish starting a fresh workspace before starting the local cluster.', workspaceFailure: { operation: 'verify', cause: 'unknown', codes: [] }, workspaceFresh: interrupted, workspaceFreshPlan: { image: { kind: 'cached' } } };
  const user = userEvent.setup(); watchCopies();
  render(<App />);
  await openLocalSetup(user);
  const card = (await screen.findByRole('heading', { name: "Bridge didn't finish starting a fresh workspace" })).closest('section')!;
  expect(card.textContent).toContain("Bridge set the workspace on this computer aside, but couldn't finish replacing it. Your agent files are kept.");
  expect(card.textContent).toContain("The workspace can't start until this is finished. Bridge finishes it when you try again, or the next time Bridge opens.");
  expect(card.textContent).not.toContain('Nothing was moved');
  expect(within(card).queryByRole('button', { name: 'Done' })).toBeNull();
  // The workspace card's own retry can't start it, so only the way forward is offered.
  expect(screen.queryByRole('button', { name: /^(Restart workspace|Try again)$/ })).toBeNull();
  await user.click(within(card).getByRole('button', { name: 'Copy details for help' }));
  expect(clipboard).toBe(`Bridge fresh workspace\nFresh workspace: interrupted (${transaction})`);
  // Finishing needs no second confirmation: nothing more is lost.
  fresh = async () => { status = { selected: 'localVm', localVm: 'preparing', model: 'locked', modelSupported: true, workspaceFresh: { state: 'replaced', transaction, ai: 'removed', reclaimable: [], cleaning: false } }; return status.workspaceFresh!; };
  await user.click(within(card).getByRole('button', { name: 'Finish starting a fresh workspace' }));
  expect(screen.queryByRole('region', { name: 'Start a fresh workspace?' })).toBeNull();
  expect(await screen.findByRole('region', { name: 'Fresh workspace' })).toBeTruthy();
  expect(calls('workspace_fresh_start')).toHaveLength(1);
});

test('folders Bridge did not expect are named, with what to do', async () => {
  const found = ['C:\\Users\\you\\AppData\\Local\\ai.orka.bridge\\local-vm-v1', retired];
  status = { selected: 'localVm', localVm: 'failed', model: 'locked', localVmError: 'Bridge must finish starting a fresh workspace before starting the local cluster.', workspaceFailure: { operation: 'verify', cause: 'unknown', codes: [] }, workspaceFresh: { state: 'conflict', found } };
  const user = userEvent.setup(); watchCopies();
  render(<App />);
  await openLocalSetup(user);
  const card = (await screen.findByRole('heading', { name: "Bridge found workspace folders it didn't expect" })).closest('section')!;
  expect(card.textContent).toContain("To continue, quit Bridge and move these out of Bridge's folder, for example to your desktop, then reopen Bridge. Keep them until the new workspace works:");
  expect(within(card).getAllByRole('listitem').map(item => item.textContent)).toEqual(found);
  await user.click(within(card).getByRole('button', { name: 'Copy details for help' }));
  expect(clipboard).toBe('Bridge fresh workspace\nFresh workspace: unexpected folders (2)');
});

test('when native says a fresh workspace cannot start, the stuck card keeps its own retry and says why', async () => {
  status = { ...status, workspaceFreshPlan: { blocked: 'updateHistory', image: { kind: 'cached' } } };
  const user = userEvent.setup(); watchCopies();
  render(<App />);
  await openLocalSetup(user);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  await screen.findByRole('alert');
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('this workspace came from a workspace update'));
  expect(screen.queryByRole('button', { name: 'Start a fresh workspace' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Check again' })).toBeTruthy();
  await user.click(screen.getAllByRole('button', { name: 'Copy details for help' })[0]);
  expect(clipboard).toContain('Fresh workspace available: no (updateHistory)');
  // Nor is it offered from Manage.
  const card = screen.getByRole('region', { name: 'Workspace' });
  await user.click(within(card).getByRole('button', { name: "What's inside" }));
  expect(within(card).queryByRole('button', { name: 'Start a fresh workspace…' })).toBeNull();
});

test('a guest still settling is worth another check, not a refusal towards a fresh workspace', async () => {
  const busy = { kind: 'modelReconnect', message: 'SECRET', cleanup: ['modelStatusPreconditions', 'busy'] };
  refusals = [refused, busy, refused];
  const user = userEvent.setup();
  render(<App />);
  await openLocalSetup(user);
  await user.click(await screen.findByRole('button', { name: 'Connect the AI' }));
  await screen.findByRole('alert');
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  await user.click(await screen.findByRole('button', { name: 'Check again' }));
  // Refused, busy, refused: never two refusals in a row, so Check again stays.
  await screen.findByRole('button', { name: 'Check again' });
  expect(screen.queryByRole('button', { name: 'Start a fresh workspace' })).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Check again' }));
  expect(await screen.findByRole('button', { name: 'Start a fresh workspace' })).toBeTruthy();
});
