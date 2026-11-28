import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const win = vi.hoisted(() => ({ minimize: vi.fn(), toggleMaximize: vi.fn(), close: vi.fn(), isMaximized: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => win }));
import WindowControls, { customTitleBar } from './WindowControls';

const windowsAgent = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0';
const macAgent = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';

describe('custom title bar', () => {
  beforeEach(() => {
    (globalThis as { isTauri?: boolean }).isTauri = true;
    win.isMaximized.mockResolvedValue(false);
  });
  afterEach(() => {
    cleanup(); vi.clearAllMocks(); vi.unstubAllGlobals();
    delete (globalThis as { isTauri?: boolean }).isTauri;
  });

  it('is drawn only by the Windows desktop app', () => {
    expect(customTitleBar(windowsAgent)).toBe(true);
    expect(customTitleBar(macAgent)).toBe(false);
    delete (globalThis as { isTauri?: boolean }).isTauri;
    expect(customTitleBar(windowsAgent)).toBe(false);
  });

  it('keeps the native frame elsewhere', () => {
    vi.stubGlobal('navigator', { ...navigator, userAgent: macAgent });
    const { container } = render(<WindowControls />);
    expect(container.innerHTML).toBe('');
  });

  it('minimizes, maximizes and closes through the window, and shows Restore when maximized', async () => {
    vi.stubGlobal('navigator', { ...navigator, userAgent: windowsAgent });
    render(<WindowControls />);
    fireEvent.click(screen.getByRole('button', { name: 'Minimize' }));
    fireEvent.click(screen.getByRole('button', { name: 'Maximize' }));
    // Close is a close request, so Bridge's close-to-tray rules still decide what happens.
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(win.minimize).toHaveBeenCalledOnce();
    expect(win.toggleMaximize).toHaveBeenCalledOnce();
    expect(win.close).toHaveBeenCalledOnce();
    win.isMaximized.mockResolvedValue(true);
    fireEvent(window, new Event('resize'));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore' })).toBeTruthy());
  });
});
