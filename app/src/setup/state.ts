import { invoke, isTauri } from '@tauri-apps/api/core';

export type SetupStatus = { selected: 'localVm' | null; localVm: 'notIntegrated'; model: 'locked' };
type Result = { ok: true; status: SetupStatus } | { ok: false; message: string };

async function call(command: 'setup_status' | 'setup_select_local'): Promise<Result> {
  if (!isTauri()) return { ok: false, message: 'Native setup is not available in this preview.' };
  try {
    const value = await invoke<unknown>(command);
    if (typeof value !== 'object' || value === null || Array.isArray(value)
      || Object.keys(value).sort().join(',') !== 'localVm,model,selected'
      || !('selected' in value) || (value.selected !== null && value.selected !== 'localVm')
      || !('localVm' in value) || value.localVm !== 'notIntegrated'
      || !('model' in value) || value.model !== 'locked'
      || (command === 'setup_select_local' && value.selected !== 'localVm')) {
      return { ok: false, message: 'Setup information is not supported by this build.' };
    }
    return { ok: true, status: { selected: value.selected, localVm: value.localVm, model: value.model } };
  } catch {
    return { ok: false, message: 'The setup choice could not be read or saved.' };
  }
}
export const setupStatus = () => call('setup_status');
export const selectLocal = () => call('setup_select_local');
