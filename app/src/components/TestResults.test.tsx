import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import type { RunSnapshot } from '../designer/run-api';
import { TestResults, TestProgress, resultLabel, changedFiles } from './TestResults';
afterEach(cleanup);
const failed = { id: 'reads-page', question: 'What did the page say?', phase: 'Succeeded', answer: '<script>wrong answer</script>', completedTools: ['web_fetch'], verdict: 'failed' as const, assertions: [{ id: 'value', kind: 'contains' as const, expected: 'checksum', verdict: 'failed' as const }] };
const passed = { ...failed, id: 'has-date', verdict: 'passed' as const, assertions: [{ ...failed.assertions[0], verdict: 'passed' as const }] };
const report: NonNullable<RunSnapshot['report']> = { verdict: 'failed', cases: [passed, failed] };
test('failed tests lead and open next to their frozen question, with a plain reason', () => {
  const { container } = render(<TestResults report={report} stale={false} />);
  const rows = container.querySelectorAll<HTMLDetailsElement>('.test-result');
  expect(rows[0].open).toBe(true); expect(rows[1].open).toBe(false);
  expect(rows[0].textContent).toContain('reads-page');
  expect(rows[0].textContent).toContain('What did the page say?');
  expect(screen.getAllByText('The answer did not include “checksum”.')).toHaveLength(1);
  expect(screen.getAllByText('<script>wrong answer</script>')[0].className).toContain('reading');
  expect(container.querySelector('script')).toBeNull();
  expect(rows[0].querySelector('details')?.open).toBe(false);
  expect(resultLabel(report)).toBe('1 of 2 passed');
});
test('passed rows reveal the same question and answer on click', async () => {
  const { container } = render(<TestResults report={{ verdict: 'passed', cases: [passed] }} stale={false} />);
  const row = container.querySelector<HTMLDetailsElement>('.test-result')!;
  expect(row.open).toBe(false);
  await userEvent.click(row.querySelector('summary')!);
  expect(row.open).toBe(true);
  expect(row.textContent).toContain('What did the page say?');
});
test('stale rows have no passing icons or current verdict words', () => {
  const { container } = render(<TestResults report={report} stale />);
  expect(container.querySelector('.status-passed, .status-failed')).toBeNull();
  expect(container.textContent).not.toContain('Passed');
  expect(container.textContent).toContain('Earlier result');
});
test('unknown tool evidence is not a fabricated failure', () => {
  const unknown = { ...failed, verdict: 'unknown' as const, completedTools: [], answer: null, assertions: [{ id: 'fetch', kind: 'toolCalled' as const, expected: 'web_fetch', verdict: 'unknown' as const }] };
  render(<TestResults report={{ verdict: 'unknown', cases: [unknown] }} stale={false} />);
  expect(screen.getByText('No answer was observed.')).toBeTruthy();
  expect(screen.getByText('A call to web_fetch was not observed; this result is unknown.')).toBeTruthy();
});
test('a failed Task has a plain failure reason even when its expectations pass', () => {
  render(<TestResults report={{ verdict: 'failed', cases: [{ ...passed, phase: 'Failed', verdict: 'failed' }] }} stale={false} />);
  expect(screen.getByText('The agent could not finish this test. Try running it again.')).toBeTruthy();
});
test('repeated calls to one tool render without duplicate keys', () => {
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
  try {
    render(<TestResults report={{ verdict: 'failed', cases: [{ ...failed, completedTools: ['web_fetch', 'web_fetch'] }] }} stale={false} />);
    expect(errors).not.toHaveBeenCalled();
  } finally { errors.mockRestore(); }
});
test('stages and stale file differences report only known facts', () => {
  render(<TestProgress stage="Waiting for answers and evaluating tests" />);
  expect(screen.getByRole('status').textContent).toBe('Waiting for answers and evaluating tests');
  expect(screen.getByText('Observed for 0s').closest('[role="status"]')).toBeNull();
  expect(screen.getByText('Observed for 0s')).toBeTruthy();
  expect(changedFiles({ agent: 'b', tests: 'c' }, { agent: 'a', tests: 'c' }, false)).toBe('Agent file changed since this run.');
  expect(changedFiles({ agent: 'b', tests: 'd' }, { agent: 'a', tests: 'c' }, true)).toBe('Agent and test files changed since this run.');
  expect(changedFiles(undefined, undefined, true)).toBe('Your agent has unsaved edits.');
});
