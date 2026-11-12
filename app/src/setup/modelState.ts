import { invoke, isTauri } from '@tauri-apps/api/core';

const errors: Record<string, string> = {
  modelCaptureUnavailable: 'Install GitHub CLI and sign in to github.com before connecting.',
  modelCaptureFailed: 'The stored GitHub sign-in could not be read. Check your GitHub CLI sign-in.',
  modelCaptureInvalid: 'The stored GitHub sign-in returned invalid credential data.',
  modelCaptureTimedOut: 'Reading the stored GitHub sign-in timed out.',
  modelStore: 'The native credential store could not save or load the sign-in.',
  modelGuest: 'The guest model handler is unavailable or is not idle.',
  modelDelivery: 'Private credential delivery was not acknowledged. Completion is unknown.',
  modelConnect: 'Guest Connect did not acknowledge a ready model. Completion is unknown.',
  modelBusy: 'Another operation is using the local VM. Wait for it to finish.',
  modelSession: 'The local VM is not available for model connection.',
  modelUnavailable: 'The stored model connection is unavailable or its completion is unknown.',
  modelReconnect: 'Bridge could not clean up the previous model connection. This workspace may need a newer image. The saved sign-in was kept.',
};
type ModelResult = { ok: true } | { ok: false; message: string };
export const connectModel = () => call('setup_connect_model', true, 'The model connection could not be completed.');
export const removeModel = () => call('setup_remove_model', false, 'The saved sign-in could not be removed. It has been kept for another attempt.');
async function call(command: string, ready: boolean, fallback: string): Promise<ModelResult> {
  if (!isTauri()) return { ok: false, message: 'Native model setup is not available in this preview.' };
  try {
    const value = await invoke<unknown>(command);
    if (typeof value === 'object' && value !== null && !Array.isArray(value)
      && Object.keys(value).join(',') === 'ready' && 'ready' in value && value.ready === ready) {
      return { ok: true };
    }
  } catch (error: unknown) {
    // Only a native finite category selects text. Never render a supplied message.
    if (typeof error === 'object' && error !== null && 'kind' in error
      && typeof error.kind === 'string' && Object.hasOwn(errors, error.kind)) {
      if (error.kind === 'modelStore' && 'native' in error) {
        const detail = error.native;
        if (typeof detail === 'object' && detail !== null && 'call' in detail && 'code' in detail
          && (detail.call === 'CredWriteW' || detail.call === 'CredReadW' || detail.call === 'CredDeleteW')
          && typeof detail.code === 'number' && Number.isInteger(detail.code)
          && detail.code >= 0 && detail.code <= 0xffffffff) {
          const operation = detail.call === 'CredWriteW' ? 'save' : detail.call === 'CredReadW' ? 'load' : 'remove';
          const reason = detail.call === 'CredWriteW' && detail.code === 8
            ? "Windows couldn't save the sign-in. Windows Credential Manager is full"
            : `Windows couldn't ${operation} the sign-in`;
          return { ok: false, message: `${reason} (${detail.call}, error ${detail.code}).` };
        }
      }
      return { ok: false, message: errors[error.kind] };
    }
  }
  return { ok: false, message: fallback };
}
