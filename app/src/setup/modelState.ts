import { invoke, isTauri } from '@tauri-apps/api/core';
import { isOsCode } from './recovery-info';

const errors = {
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
};
export type ModelFailure = { message: string; category: keyof typeof errors | 'unknown'; codes: number[] };
export type ModelResult = { ok: true } | ({ ok: false } & ModelFailure);
export async function connectModel(): Promise<ModelResult> {
  if (!isTauri()) return { ok: false, category: 'unknown', codes: [], message: 'Native model setup is not available in this preview.' };
  try {
    const value = await invoke<unknown>('setup_connect_model');
    if (typeof value === 'object' && value !== null && !Array.isArray(value)
      && Object.keys(value).join(',') === 'ready' && 'ready' in value && value.ready === true) {
      return { ok: true };
    }
  } catch (error: unknown) {
    // Only a native finite category selects text. Never render a supplied message.
    if (typeof error === 'object' && error !== null && 'kind' in error
      && typeof error.kind === 'string' && Object.hasOwn(errors, error.kind)) {
      const category = error.kind as keyof typeof errors;
      if (category === 'modelStore' && 'native' in error) {
        const detail = error.native;
        if (typeof detail === 'object' && detail !== null && 'call' in detail && 'code' in detail
          && (detail.call === 'CredWriteW' || detail.call === 'CredReadW' || detail.call === 'CredDeleteW')
          && isOsCode(detail.code)) {
          const operation = detail.call === 'CredWriteW' ? 'save' : detail.call === 'CredReadW' ? 'load' : 'remove';
          const reason = detail.call === 'CredWriteW' && detail.code === 8
            ? "Windows couldn't save the sign-in. Windows Credential Manager is full"
            : `Windows couldn't ${operation} the sign-in`;
          return { ok: false, category, codes: [detail.code], message: `${reason} (${detail.call}, error ${detail.code}).` };
        }
      }
      return { ok: false, category, codes: [], message: errors[category] };
    }
  }
  return { ok: false, category: 'unknown', codes: [], message: 'The model connection could not be completed.' };
}
