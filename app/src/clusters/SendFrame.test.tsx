import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import Send from './Send';

const documentId = '9b2b93c4-375c-4815-9717-39852a96e411';
const id = '00000000-0000-4000-8000-000000000004';
const source = { agent: 'a'.repeat(64), tests: 'b'.repeat(64) };
const target = { id, context_id: id, context_name: 'team-dev', service_name: 'orka', service_namespace: 'control', watched_namespace: 'agents', image: 'orka' };
const provider = { id, name: 'team-model', ready: true };
const place = { kind: 'cluster' as const, target, providers: [provider], providerId: id };
const allowed = { label: 'Access', status: 'allowed', message: 'Access is ready.' };
const check = { documentId, source, targetId: id, providerId: id, model: 'gpt-5.6-terra', rows: [allowed, { label: 'Inference', status: 'unverified', message: 'Checked when tests run.' }], canSend: true, canSeparate: false, action: 'unchanged' };
const failed = { id: 'reads-page', question: 'Read it?', phase: 'Succeeded', answer: 'No page', completedTools: [], assertions: [{ id: 'value', kind: 'contains', expected: 'page', verdict: 'failed' }], verdict: 'failed' };
const result = { documentId, id, targetId: id, providerId: id, targetName: 'team-dev', source, model: 'gpt-5.6-terra', agentName: 'generated-agent', agentAction: 'unchanged', state: 'complete', stage: 'done', acknowledged: [{ kind: 'Agent', name: 'generated-agent', uid: 'uid' }], pending: null, report: { verdict: 'failed', cases: [failed] }, error: null, storageError: false, cancellation: null };
function native(view: unknown = null, preflight: unknown = check, start?: Promise<unknown>) {
  vi.stubGlobal('isTauri', true);
  mockIPC(command => command === 'send_history' ? [] : command === 'send_preflight' ? preflight : command === 'send_start' && start ? start : { documentId, view, busy: null });
}
afterEach(() => { cleanup(); clearMocks(); vi.unstubAllGlobals(); });
const card = () => screen.getByRole('region', { name: 'Send and cluster tests' });

test('destination head and one summary precede honest collapsed checks and separate history card', async () => {
  const choose = vi.fn(); native();
  render(<Send place={place} source={source} stale={false} agentName="web-reader" testCount={2} onChooseCluster={choose} />);
  await screen.findByText('Ready to send: 1 check passed');
  expect(within(card()).getByRole('heading', { name: 'To team-dev' })).toBeTruthy();
  expect(within(card()).getByRole('region', { name: 'What you send' }).textContent).toContain('web-reader with 2 tests');
  expect(within(card()).getByRole('region', { name: 'What you send' }).textContent).toContain('Into control/orka on team-dev, namespace agents.');
  const checks = screen.getByText('Ready to send: 1 check passed').closest('details')!;
  expect(checks.open).toBe(false);
  expect(checks.contains(screen.getByText('Checked when tests run.'))).toBe(true);
  expect(screen.getByRole('region', { name: 'Past Sends' }).classList.contains('card')).toBe(true);
  await userEvent.click(screen.getByRole('button', { name: 'Change' })); expect(choose).toHaveBeenCalledOnce();
});
test('only blocking checks are exposed; passing and advisory rows stay folded', async () => {
  native(null, { ...check, canSend: false, action: 'blocked', rows: [...check.rows, { label: 'Missing tool', status: 'blocked', message: 'Install page-reader.' }] });
  render(<Send place={place} source={source} stale={false} />);
  expect((await screen.findByText('Install page-reader.')).closest('details')).toBeNull();
  expect(screen.getByText('Access is ready.').closest('details')?.open).toBe(false);
  expect(screen.getByText('Checked when tests run.').closest('details')?.open).toBe(false);
  expect(screen.getByRole('button', { name: 'Update agent and run tests' }).hasAttribute('disabled')).toBe(true);
});
test.each(['unchanged', 'update'] as const)('current failed result offers editor repair before the precise %s retry', async action => {
  native(result, { ...check, action }); const fix = vi.fn();
  render(<Send place={place} source={source} stale={false} onFixInDesigner={fix} onSeeInOverview={() => {}} />);
  const repair = await screen.findByRole('button', { name: 'Fix in Designer' });
  await waitFor(() => expect(repair.className).toContain('button-primary'));
  expect(screen.getByRole('button', { name: action === 'unchanged' ? 'Run cluster tests again' : 'Update agent and run tests' }).className).toContain('button-secondary');
  await userEvent.click(repair); expect(fix).toHaveBeenCalledOnce();
});
test('a failed result that finished before confirmed cancellation still offers repair', async () => {
  native({ ...result, cancellation: 'confirmed' });
  render(<Send place={place} source={source} stale={false} onFixInDesigner={() => {}} />);
  expect((await screen.findByRole('button', { name: 'Fix in Designer' })).className).toContain('button-primary');
  expect(screen.getByRole('button', { name: 'Run cluster tests again' }).className).toContain('button-secondary');
});
test.each(['foreign', 'stale', 'status', 'reload', 'check'] as const)('%s result or recovery never promotes Fix in Designer', async mode => {
  native({ ...result, ...(mode === 'foreign' ? { targetId: '00000000-0000-4000-8000-000000000005' } : {}), ...(mode === 'status' ? { cancellation: 'unconfirmed' } : {}) }, { ...check, ...(mode === 'reload' ? { source: { ...source, tests: 'c'.repeat(64) } } : {}), ...(mode === 'check' ? { canSend: false } : {}) });
  render(<Send place={place} source={source} stale={mode === 'stale'} onFixInDesigner={() => {}} onSeeInOverview={() => {}} />);
  await waitFor(() => expect(card().querySelector('[aria-busy="true"]')).toBeNull());
  expect(screen.queryByRole('button', { name: 'Fix in Designer' })).toBeNull();
});
test('Change is disabled with a reason during admission, before a running snapshot arrives', async () => {
  native(null, check, new Promise(() => {}));
  render(<Send place={place} source={source} stale={false} onChooseCluster={() => {}} />);
  const send = await screen.findByRole('button', { name: 'Update agent and run tests' });
  await waitFor(() => expect(send.hasAttribute('disabled')).toBe(false));
  await userEvent.click(send);
  const change = screen.getByRole('button', { name: 'Change' });
  expect(change.hasAttribute('disabled')).toBe(true);
  expect(change.title).toContain('Wait');
});
