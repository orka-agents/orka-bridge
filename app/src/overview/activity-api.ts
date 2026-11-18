import { invoke, isTauri } from '@tauri-apps/api/core';

export type Phase = 'pending' | 'scheduled' | 'running' | 'finalizing' | 'succeeded' | 'failed' | 'cancelled' | 'unknown';
export type ActivityEntry = {
  task: string; uid: string; agent: string | null; agentUid: string | null; phase: Phase;
  createdAt: number; startedAt: number | null; completedAt: number | null; message: string | null; test: boolean;
};
export type PlaceState = 'complete' | 'partial' | 'denied' | 'unavailable' | 'revoked' | 'disconnected';
/** `connection` is the place's connection revision: cached runs belong to one connection only. */
export type PlaceActivity = { key: string; state: PlaceState; observedAt: number | null; entries: ActivityEntry[]; connection: number | null };
export type Activity = { places: PlaceActivity[] };
export type ToolCall = { tool: string; failed: boolean; at: number };
export type ToolCalls = { state: PlaceState; calls: ToolCall[] };

const phases: Phase[] = ['pending', 'scheduled', 'running', 'finalizing', 'succeeded', 'failed', 'cancelled', 'unknown'];
const states: PlaceState[] = ['complete', 'partial', 'denied', 'unavailable', 'revoked', 'disconnected'];
const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const text = (v: unknown, max: number): v is string => typeof v === 'string' && v.length > 0 && v.length <= max;
const nullable = (v: unknown, max: number) => v === null || text(v, max);
const time = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const nullableTime = (v: unknown) => v === null || time(v);

function isEntry(v: unknown): v is ActivityEntry {
  return object(v) && text(v.task, 253) && text(v.uid, 128) && nullable(v.agent, 253) && nullable(v.agentUid, 128) && phases.includes(v.phase as Phase)
    && time(v.createdAt) && nullableTime(v.startedAt) && nullableTime(v.completedAt) && nullable(v.message, 512) && typeof v.test === 'boolean'
    && (v.agentUid === null || v.agent !== null);
}
export function isActivity(v: unknown): v is Activity {
  if (!object(v) || !Array.isArray(v.places)) return false;
  const keys = new Set<string>();
  return v.places.every(p => {
    if (!object(p) || !text(p.key, 4096) || keys.has(p.key) || !states.includes(p.state as PlaceState) || !nullableTime(p.observedAt) || !nullableTime(p.connection) || !Array.isArray(p.entries)) return false;
    keys.add(p.key);
    const read = p.state === 'complete' || p.state === 'partial';
    // Only a successful read carries entries and a time; failures carry neither.
    if (read ? p.observedAt === null || p.entries.length > 50 : p.observedAt !== null || p.entries.length > 0) return false;
    const uids = new Set<string>();
    return p.entries.every(e => { if (!isEntry(e) || uids.has(e.uid)) return false; uids.add(e.uid); return true; });
  });
}
export function isToolCalls(v: unknown): v is ToolCalls {
  return object(v) && states.includes(v.state as PlaceState) && Array.isArray(v.calls) && v.calls.length <= 200
    && (v.state === 'complete' || v.state === 'partial' || v.calls.length === 0)
    && v.calls.every(c => object(c) && text(c.tool, 256) && typeof c.failed === 'boolean' && time(c.at));
}
export async function readActivity(): Promise<Activity | undefined> {
  if (isTauri()) { try { const v: unknown = await invoke('overview_activity'); if (isActivity(v)) return v; } catch { /* No native diagnostics reach Overview. */ } }
}
export async function readToolCalls(key: string, task: string): Promise<ToolCalls> {
  if (isTauri()) { try { const v: unknown = await invoke('overview_tool_calls', { key, task }); if (isToolCalls(v)) return v; } catch { /* No native diagnostics reach Overview. */ } }
  return { state: 'unavailable', calls: [] };
}

export const latest = (e: ActivityEntry) => Math.max(e.createdAt, e.startedAt ?? 0, e.completedAt ?? 0);
export type Filter = 'all' | 'failed' | 'active';
const active: Phase[] = ['pending', 'scheduled', 'running', 'finalizing'];
export const matches = (e: ActivityEntry, filter: Filter) => filter === 'all' || (filter === 'failed' ? e.phase === 'failed' : active.includes(e.phase));
/** `readAt` is when Bridge read the run's place, so expanded details can follow refreshes. */
export type Row = ActivityEntry & { placeKey: string; readAt: number | null };
/** Every read place's entries, newest first across places, then by place and Task name for a stable order. */
export function merge(places: PlaceActivity[], filter: Filter): Row[] {
  return places.flatMap(p => p.entries.map(e => ({ ...e, placeKey: p.key, readAt: p.observedAt })))
    .filter(e => matches(e, filter))
    .sort((a, b) => latest(b) - latest(a) || a.placeKey.localeCompare(b.placeKey) || a.task.localeCompare(b.task));
}
export const phaseView: Record<Phase, { status: 'passed' | 'failed' | 'running' | 'stale' | 'not-started'; pill: string }> = {
  pending: { status: 'not-started', pill: 'Waiting' },
  scheduled: { status: 'not-started', pill: 'Scheduled' },
  running: { status: 'running', pill: 'Running' },
  finalizing: { status: 'running', pill: 'Finishing' },
  succeeded: { status: 'passed', pill: 'Succeeded' },
  failed: { status: 'failed', pill: 'Failed' },
  cancelled: { status: 'stale', pill: 'Cancelled' },
  unknown: { status: 'stale', pill: 'Unknown' },
};
