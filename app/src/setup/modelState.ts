import { invoke, isTauri } from '@tauri-apps/api/core';
import { isOsCode } from './recovery-info';

const errors = {
  // Native tells these apart: no GitHub CLI where Bridge looks for it, or GitHub CLI there whose sign-in it couldn't read.
  modelCaptureUnavailable: "Bridge couldn't find GitHub CLI on this computer.",
  modelCaptureFailed: "GitHub CLI isn't signed in to github.com, or Bridge couldn't read its sign-in.",
  modelCaptureInvalid: 'The stored GitHub sign-in returned invalid credential data.',
  modelCaptureTimedOut: 'Reading the stored GitHub sign-in timed out.',
  modelStore: 'Bridge could not save or load your sign-in.',
  modelGuest: 'The workspace is not ready to connect the AI.',
  modelDelivery: 'Bridge could not confirm that the workspace received the sign-in.',
  modelConnect: 'Bridge could not confirm the AI connection. Check again to reconnect.',
  modelBusy: 'Another operation is using the workspace. Wait for it to finish.',
  modelSession: 'The workspace is not available to connect the AI.',
  modelUnavailable: 'The AI connection is unavailable or could not be confirmed.',
  modelInstallationMismatch: "Bridge's saved AI connection is from an earlier setup.",
  modelReconnect: "Bridge couldn't remove the previous AI connection from the workspace. The saved sign-in was kept.",
  modelReconnectImage: 'Bridge could not clean up the previous model connection. The workspace may need a newer image. The saved sign-in was kept.',
  modelKeyInvalid: 'Paste the whole key.',
  modelAlreadyAssociated: 'Change or remove the current AI connection before adding a key.',
  modelCredentialRejected: 'GitHub Copilot did not accept the saved sign-in. Check your GitHub CLI sign-in.',
  modelProviderUnreachable: 'The workspace could not reach GitHub Copilot. Check your internet connection.',
  modelModelUnavailable: 'The default model is not listed for this Copilot sign-in. Check your Copilot plan.',
  modelKeyCleanup: 'Key removal has not been confirmed. Retry removal before adding another key.',
  modelKeyUnsupported: "The workspace on this computer can't use your own API key yet. Choose GitHub Copilot instead.",
  modelCopilotTokenType: "This GitHub CLI sign-in uses a classic token, which GitHub Copilot doesn't accept here.",
  modelCopilotSignInRejected: "GitHub didn't accept your GitHub CLI sign-in.",
  modelCopilotAccessDenied: "GitHub Copilot isn't available to this GitHub account.",
  modelCopilotUnreachable: "This computer couldn't reach GitHub Copilot.",
  modelCopilotMalformed: "GitHub Copilot sent a reply Bridge doesn't understand.",
  modelCopilotModelNotOffered: "The chosen model isn't offered to this GitHub account any more.",
  modelCopilotChooseModel: 'Choose a model to connect.',
};
// Copilot preflight outcomes: checked on this computer, before anything reaches the workspace.
export const preflightCategories = ['modelCopilotTokenType', 'modelCopilotSignInRejected', 'modelCopilotAccessDenied', 'modelCopilotUnreachable', 'modelCopilotMalformed', 'modelCopilotModelNotOffered'] as const;
export function isPreflightCategory(category: string): category is typeof preflightCategories[number] {
  return (preflightCategories as readonly string[]).includes(category);
}
export type KeyVendor = 'openai' | 'anthropic';
export const companies = {
  openai: { name: 'OpenAI', model: 'gpt-4.1', prefix: 'sk-', site: 'platform.openai.com' },
  anthropic: { name: 'Anthropic', model: 'claude-sonnet-5-5', prefix: 'sk-ant-', site: 'console.anthropic.com' },
} as const;
// A Copilot model id as the native side accepts one: the account's own catalog names it.
export const isModelId = (id: unknown): id is string => typeof id === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/.test(id) && id.toLowerCase() !== 'auto';
// `account` is the GitHub login verified when this sign-in was saved; absent means unavailable.
export type ModelConnection = { kind: 'copilot'; model: string; account?: string } | { kind: 'key'; vendor: KeyVendor; model: string };
const githubLogin = /^[A-Za-z0-9][A-Za-z0-9-]{0,38}$/;
export type CopilotModel = { id: string; name?: string };
// The account's usable models, from GitHub's own reply, and the row the picker starts on.
export type CopilotCatalog = { account?: string; models: CopilotModel[]; preselected?: string };
function modelList(value: unknown): CopilotModel[] | undefined {
  if (!Array.isArray(value) || value.length > 50) return;
  const models: CopilotModel[] = [];
  for (const item of value) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return;
    const m = item as Record<string, unknown>;
    const keys = Object.keys(m).sort().join(',');
    if (!isModelId(m.id) || models.some(other => other.id === m.id)) return;
    if (keys === 'id') models.push({ id: m.id });
    else if (keys === 'id,name' && typeof m.name === 'string' && /^[\x20-\x7e]{1,80}$/.test(m.name)) models.push({ id: m.id, name: m.name });
    else return;
  }
  return models;
}
function catalogInfo(value: Record<string, unknown>, extra: string[]): CopilotCatalog | undefined {
  if (Object.keys(value).some(key => ![...extra, 'account', 'models', 'preselected'].includes(key))) return;
  const models = modelList(value.models);
  if (!models || ('account' in value && !(typeof value.account === 'string' && githubLogin.test(value.account)))) return;
  if ('preselected' in value && !(isModelId(value.preselected) && models.some(m => m.id === value.preselected))) return;
  return { models, ...(typeof value.account === 'string' ? { account: value.account } : {}), ...(typeof value.preselected === 'string' ? { preselected: value.preselected } : {}) };
}
function connectionInfo(value: unknown): ModelConnection | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return;
  const c = value as Record<string, unknown>;
  const keys = Object.keys(c).sort().join(',');
  if (c.kind === 'copilot' && isModelId(c.model)) {
    if (keys === 'kind,model') return { kind: 'copilot', model: c.model };
    if (keys === 'account,kind,model' && typeof c.account === 'string' && githubLogin.test(c.account)) return { kind: 'copilot', model: c.model, account: c.account };
    return;
  }
  if (Object.keys(c).sort().join(',') === 'kind,model,vendor' && c.kind === 'key' && (c.vendor === 'openai' || c.vendor === 'anthropic') && c.model === companies[c.vendor].model) return { kind: 'key', vendor: c.vendor, model: companies[c.vendor].model };
}
export function validKey(vendor: KeyVendor, key: string) {
  return key.length > companies[vendor].prefix.length && key.length <= 16384 && /^[\x21-\x7e]+$/.test(key)
    && key.startsWith(companies[vendor].prefix) && (vendor !== 'openai' || !key.startsWith('sk-ant-'));
}
export function keyFailure(error: ModelFailure, vendor: KeyVendor): string {
  const c = companies[vendor];
  switch (error.category) {
    case 'modelKeyInvalid': return `Paste the whole key. ${c.name} keys start with ${c.prefix}.`;
    case 'modelCredentialRejected': return `${c.name} didn't accept this key. Copy it again from ${c.site}, or create a new one.`;
    case 'modelProviderUnreachable': return `Your workspace couldn't reach ${c.name}. Check this computer's internet connection, then try again.`;
    case 'modelModelUnavailable': return `${c.model} isn't listed for this key. Check your ${c.name} plan, or choose Copilot.`;
    default: return error.message.replaceAll('sign-in', 'key');
  }
}
export type ModelRecovery = { installation: string; savedInstallation: string; credentialSlot: string; resetStage?: 'credentialRemovalUnconfirmed' | 'recordRemovalUnconfirmed' };
function recoveryInfo(value: unknown): ModelRecovery | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return;
  const r = value as Record<string, unknown>;
  const uuid = (id: unknown): id is string => typeof id === 'string' && /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(id) && id !== '00000000-0000-0000-0000-000000000000';
  if (!uuid(r.installation) || !uuid(r.savedInstallation) || !uuid(r.credentialSlot) || r.installation.toLowerCase() === r.savedInstallation.toLowerCase()) return;
  if (Object.keys(r).some(key => !['installation', 'savedInstallation', 'credentialSlot', 'resetStage'].includes(key))) return;
  if (r.resetStage !== undefined && r.resetStage !== 'credentialRemovalUnconfirmed' && r.resetStage !== 'recordRemovalUnconfirmed') return;
  return { installation: r.installation, savedInstallation: r.savedInstallation, credentialSlot: r.credentialSlot, ...(r.resetStage ? { resetStage: r.resetStage } : {}) };
}
const cleanupOperations = ['modelStatus', 'modelStatusPreconditions', 'disconnectRetainedModel', 'connectionUnknown'] as const;
const cleanupClasses = ['timeout', 'unsupported', 'protocol', 'connection', 'permission', 'refused', 'busy', 'io'] as const;
type CleanupDiagnostic = [typeof cleanupOperations[number], typeof cleanupClasses[number]];
function cleanupInfo(value: unknown): CleanupDiagnostic | undefined {
  if (!Array.isArray(value) || value.length !== 2) return;
  const operation = cleanupOperations.find(item => item === value[0]);
  const classification = cleanupClasses.find(item => item === value[1]);
  return operation && classification ? [operation, classification] : undefined;
}
// `account`, `wanted` and `catalog` come only with Copilot preflight outcomes and the model choice.
export type ModelFailure = { message: string; category: keyof typeof errors | 'unknown'; codes: number[]; vendor?: KeyVendor; recovery?: ModelRecovery; cleanup?: CleanupDiagnostic; account?: string; wanted?: string; catalog?: CopilotCatalog };
export type ModelResult = { ok: true; connection?: ModelConnection } | ({ ok: false } & ModelFailure);
export const connectModel = (vendor?: KeyVendor, model?: string) => call('setup_connect_model', true, 'The AI connection could not be completed.', vendor ? { vendor } : model ? { model } : undefined);
// Host-only: the GitHub CLI sign-in and the account's models, while the workspace is still on its way.
export async function checkCopilot(): Promise<{ ok: true; catalog: CopilotCatalog } | ({ ok: false } & ModelFailure)> {
  if (!isTauri()) return { ok: false, category: 'unknown', codes: [], message: 'AI setup is not available in this preview.' };
  try {
    const value = await invoke<unknown>('setup_check_model');
    const catalog = value && typeof value === 'object' && !Array.isArray(value) ? catalogInfo(value as Record<string, unknown>, []) : undefined;
    if (catalog) return { ok: true, catalog };
  } catch (error: unknown) {
    return failure(error, 'The GitHub sign-in could not be checked.');
  }
  return { ok: false, category: 'unknown', codes: [], message: 'The GitHub sign-in could not be checked.' };
}
export const resetModel = () => call('setup_reset_model', false, 'Bridge could not confirm the AI connection was reset.');
export const removeModel = () => call('setup_remove_model', false, 'Bridge could not confirm sign-in removal. Try Remove sign-in again.');
export const connectKey = (vendor: KeyVendor, key: string) => call('setup_connect_key', true, 'The AI connection could not be completed.', { vendor, key });
async function call(command: string, ready: boolean, fallback: string, args?: { vendor: KeyVendor; key?: string } | { model: string }): Promise<ModelResult> {
  if (!isTauri()) return { ok: false, category: 'unknown', codes: [], message: 'AI setup is not available in this preview.' };
  try {
    const value = await (args ? invoke<unknown>(command, args) : invoke<unknown>(command));
    if (typeof value === 'object' && value !== null && !Array.isArray(value) && 'ready' in value && value.ready === ready) {
      const keys = Object.keys(value).sort().join(',');
      if (keys === 'ready' && (!args || 'model' in args)) return { ok: true };
      if (ready && keys === 'connection,ready' && 'connection' in value) {
        const connection = connectionInfo(value.connection);
        if (connection && (!args || ('model' in args ? connection.kind === 'copilot' && connection.model === args.model : connection.kind === 'key' && connection.vendor === args.vendor))) return { ok: true, connection };
      }
    }
  } catch (error: unknown) {
    return failure(error, fallback);
  }
  return { ok: false, category: 'unknown', codes: [], message: fallback };
}

function failure(error: unknown, fallback: string): { ok: false } & ModelFailure {
    // Only a native finite category selects text. Never render a supplied message.
    if (typeof error === 'object' && error !== null && 'kind' in error
      && typeof error.kind === 'string' && Object.hasOwn(errors, error.kind)) {
      const category = error.kind as keyof typeof errors;
      const vendor = 'vendor' in error && (error.vendor === 'openai' || error.vendor === 'anthropic') ? error.vendor : undefined;
      const recovery = 'recovery' in error ? recoveryInfo(error.recovery) : undefined;
      const cleanup = 'cleanup' in error ? cleanupInfo(error.cleanup) : undefined;
      const account = 'account' in error && typeof error.account === 'string' && githubLogin.test(error.account) ? error.account : undefined;
      const wanted = 'wanted' in error && isModelId(error.wanted) ? error.wanted : undefined;
      const catalog = 'models' in error ? catalogInfo(error as Record<string, unknown>, ['kind', 'message', 'account', 'wanted', 'models', 'preselected', 'vendor']) : undefined;
      const context: Pick<ModelFailure, 'vendor' | 'recovery' | 'cleanup' | 'account' | 'wanted' | 'catalog'> = { ...(vendor ? { vendor } : {}), ...(recovery ? { recovery } : {}), ...(cleanup ? { cleanup } : {}), ...(account ? { account } : {}), ...(wanted ? { wanted } : {}), ...(catalog ? { catalog } : {}) };
      if (category === 'modelStore' && 'native' in error) {
        const detail = error.native;
        if (typeof detail === 'object' && detail !== null && 'call' in detail && 'code' in detail
          && (detail.call === 'CredWriteW' || detail.call === 'CredReadW' || detail.call === 'CredDeleteW')
          && isOsCode(detail.code)) {
          const operation = detail.call === 'CredWriteW' ? 'save' : detail.call === 'CredReadW' ? 'load' : 'remove';
          const reason = detail.call === 'CredWriteW' && detail.code === 8
            ? "Windows couldn't save the sign-in. Windows Credential Manager is full"
            : `Windows couldn't ${operation} the sign-in`;
          return { ok: false, category, codes: [detail.code], message: `${reason} (${detail.call}, error ${detail.code}).`, ...context };
        }
      }
      return { ok: false, category, codes: [], message: errors[category], ...context };
    }
    return { ok: false, category: 'unknown', codes: [], message: fallback };
}
