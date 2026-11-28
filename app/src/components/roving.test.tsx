import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, expect, test } from 'vitest';
import { readFileSync } from 'node:fs';
import { rovingIndex, rovingKeyDown } from './roving';

afterEach(cleanup);
function Views({ disabled = '' }: { disabled?: string }) {
  const [view, setView] = useState('Agent');
  return <><button type="button">Before</button><div className="view-switch" role="group" aria-label="Designer view" onKeyDown={rovingKeyDown}>
    {['Agent', 'Try a task', 'File'].map((name, index) => <button key={name} type="button" aria-pressed={view === name} disabled={disabled === name} tabIndex={rovingIndex(view === name, true, index === 0)} onClick={() => setView(name)}>{name}</button>)}
  </div><button type="button">After</button></>;
}
const pressed = () => screen.getAllByRole('button', { pressed: true }).map(button => button.textContent);

test('the segmented control is one Tab stop, on the chosen view', async () => {
  const user = userEvent.setup(); render(<Views />);
  await user.click(screen.getByRole('button', { name: 'Before' }));
  await user.tab(); expect(document.activeElement?.textContent).toBe('Agent');
  await user.tab(); expect(document.activeElement?.textContent).toBe('After');
  await user.click(screen.getByRole('button', { name: 'File' }));
  await user.click(screen.getByRole('button', { name: 'Before' }));
  await user.tab(); expect(document.activeElement?.textContent).toBe('File');
});

test('Left and Right move to the next view and choose it, wrapping at the ends; Home and End go to either end', async () => {
  const user = userEvent.setup(); render(<Views />);
  screen.getByRole('button', { name: 'Agent' }).focus();
  await user.keyboard('{ArrowRight}'); expect(document.activeElement?.textContent).toBe('Try a task'); expect(pressed()).toEqual(['Try a task']);
  await user.keyboard('{ArrowRight}'); expect(pressed()).toEqual(['File']);
  await user.keyboard('{ArrowRight}'); expect(pressed()).toEqual(['Agent']);
  await user.keyboard('{ArrowLeft}'); expect(document.activeElement?.textContent).toBe('File'); expect(pressed()).toEqual(['File']);
  await user.keyboard('{Home}'); expect(pressed()).toEqual(['Agent']);
  await user.keyboard('{End}'); expect(pressed()).toEqual(['File']);
});

test('a disabled view is skipped', async () => {
  const user = userEvent.setup(); render(<Views disabled="Try a task" />);
  screen.getByRole('button', { name: 'Agent' }).focus();
  await user.keyboard('{ArrowRight}'); expect(pressed()).toEqual(['File']);
});

test('each segment keeps the width of its bold label, so choosing one moves nothing beside the control', () => {
  const css = readFileSync('src/app.css', 'utf8');
  expect(css).toContain('.view-switch button[data-label]::after { content: attr(data-label); height: 0; overflow: hidden; visibility: hidden; font-weight: 600;');
});
