import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// Existing lifecycle tests exercise local controls, now reached through the Places row.
export async function openLocalSetup(user = userEvent.setup()) {
  await waitFor(() => {
    if (document.querySelector('main[data-page="Home"]')) return;
    if (!screen.queryByRole('region', { name: 'Workspace' }) && !screen.queryByRole('list', { name: 'Setup steps' }) && !screen.queryByRole('heading', { name: 'Where should your agents run?' }) && !screen.queryByRole('button', { name: 'Back to clusters' }) && !screen.queryByRole('region', { name: 'This computer' })) throw Error('Waiting for Places or local progress');
  });
  // The page can draw before the shell finishes its launch read; let the sidebar catch up.
  await waitFor(() => { if (screen.queryByRole('navigation')?.textContent?.includes('Checking the workspace…')) throw Error('Waiting for launch'); });
  // A finished workspace opens on Home; these tests start from Clusters.
  if (document.querySelector('main[data-page="Home"]')) await openClusters(user);
  const row = screen.queryByRole('region', { name: 'This computer' });
  if (row) await user.click(within(row).getByRole('button', { name: /^(Set up|Continue setup|Connect the AI|Manage|Details|View details|View workspace)$/ }));
  // Lifecycle tests opt into the early AI choices; the dedicated layout test checks the collapsed default.
  const inline = document.querySelector<HTMLElement>('.model-compact .model-inline-head');
  if (inline) {
    const change = within(inline).getByRole('button', { name: 'Change' });
    if (change.getAttribute('aria-expanded') === 'false') await user.click(change);
  }
}

// Launch with a finished workspace or a remembered cluster lands on Home; open Clusters as a click would.
export async function openClusters(user = userEvent.setup()) {
  await waitFor(() => { if (!document.querySelector('main[data-page="Home"]')) throw Error('Waiting for launch on Home'); });
  await user.click(screen.getByRole('button', { name: 'Clusters' }));
  await waitFor(() => { if (!screen.queryByRole('region', { name: 'Workspace' }) && !screen.queryByRole('list', { name: 'Setup steps' }) && !screen.queryByRole('region', { name: 'This computer' })) throw Error('Waiting for Places or local progress'); });
}
