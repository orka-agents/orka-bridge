import { invoke, isTauri } from '@tauri-apps/api/core';

export type ContextView = { id: string; name: string };
export type TargetView = { id: string; context_id: string; context_name: string; service_name: string; service_namespace: string; watched_namespace: string; image: string };
export type ProviderView = { id: string; name: string; ready: boolean };
export type AccessStatus = 'connected' | 'needServiceAccount' | 'denied' | 'unavailable' | 'unsupportedCredential';
export type AccessView = { status: AccessStatus; providers: ProviderView[] };
export type SelectedPlace = { kind: 'thisComputer' } | { kind: 'cluster'; target?: TargetView };
const statuses: AccessStatus[] = ['connected', 'needServiceAccount', 'denied', 'unavailable', 'unsupportedCredential'];
const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const keys = (v: Record<string, unknown>, names: string[]) => Object.keys(v).sort().join(',') === names.sort().join(',');
const text = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && v.length <= 2048 && Array.from(v).every(c => c.charCodeAt(0) >= 32 && c.charCodeAt(0) !== 127);
const id = (v: unknown): v is string => typeof v === 'string' && /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(v);
const context = (v: unknown): v is ContextView => object(v) && keys(v, ['id', 'name']) && id(v.id) && text(v.name);
const target = (v: unknown): v is TargetView => object(v) && keys(v, ['id', 'context_id', 'context_name', 'service_name', 'service_namespace', 'watched_namespace', 'image'])
  && id(v.id) && id(v.context_id) && ['context_name', 'service_name', 'service_namespace', 'watched_namespace', 'image'].every(k => text(v[k]));
const provider = (v: unknown): v is ProviderView => object(v) && keys(v, ['id', 'name', 'ready']) && id(v.id) && text(v.name) && typeof v.ready === 'boolean';
const list = <T>(v: unknown, valid: (item: unknown) => item is T): v is T[] => Array.isArray(v) && v.length <= 1024 && v.every(valid);
async function call<T>(command: string, args: Record<string, unknown>, valid: (v: unknown) => v is T): Promise<T> {
  if (!isTauri()) throw Error('Cluster access is available in the desktop app.');
  try { const v: unknown = await invoke(command, args); if (valid(v)) return v; } catch (error) {
    if (command === 'cluster_discover' && error === 'unsupportedCredential') throw Error('This context uses a sign-in method Bridge cannot use yet. No sign-in was started.', { cause: error });
    if (command === 'cluster_discover' && error === 'denied') throw Error('Orka discovery was denied. Ask your cluster administrator to check your access.', { cause: error });
  }
  throw Error('Cluster information could not be read. Check your kubeconfig and try again.');
}
export const contexts = () => call('cluster_contexts', {}, (v): v is ContextView[] => list(v, context));
export const discover = (contextId: string) => call('cluster_discover', { contextId }, (v): v is TargetView[] => list(v, target) && v.every(t => t.context_id === contextId));
export const connectCluster = (targetId: string, saName: string | null) => call('cluster_use', { targetId, saName }, (v): v is AccessView => object(v) && keys(v, ['status', 'providers'])
  && statuses.includes(v.status as AccessStatus) && list(v.providers, provider) && (v.status === 'connected' ? v.providers.length > 0 : v.providers.length === 0));
