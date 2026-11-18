import { invoke, isTauri } from '@tauri-apps/api/core';
import type { PlaceState } from './activity-api';

export type SessionEntry = {
  name: string; sessionType: string | null; messageCount: number; createdAt: number; updatedAt: number;
  activeTask: string | null; agent: string | null;
};
/** `connection` is the place's connection revision: cached Sessions belong to one connection only. */
export type PlaceSessions = { key: string; state: PlaceState; observedAt: number | null; entries: SessionEntry[]; connection: number | null };
export type Sessions = { places: PlaceSessions[] };
export type Message = { role: string; text: string; cut: boolean; tools: string[]; at: number | null };
export type Transcript = { state: PlaceState; messages: Message[] };

const states: PlaceState[] = ['complete', 'partial', 'denied', 'unavailable', 'revoked', 'disconnected'];
const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const text = (v: unknown, max: number): v is string => typeof v === 'string' && v.length > 0 && v.length <= max;
const nullable = (v: unknown, max: number) => v === null || text(v, max);
const count = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const nullableCount = (v: unknown) => v === null || count(v);

function isEntry(v: unknown): v is SessionEntry {
  return object(v) && text(v.name, 253) && nullable(v.sessionType, 32) && count(v.messageCount) && count(v.createdAt) && count(v.updatedAt)
    && nullable(v.activeTask, 253) && nullable(v.agent, 253) && (v.agent === null || v.activeTask !== null);
}
export function isSessions(v: unknown): v is Sessions {
  if (!object(v) || !Array.isArray(v.places)) return false;
  const keys = new Set<string>();
  return v.places.every(p => {
    if (!object(p) || !text(p.key, 4096) || keys.has(p.key) || !states.includes(p.state as PlaceState) || !nullableCount(p.observedAt) || !nullableCount(p.connection) || !Array.isArray(p.entries)) return false;
    keys.add(p.key);
    const read = p.state === 'complete' || p.state === 'partial';
    // Only a successful read carries entries and a time; failures carry neither.
    if (read ? p.observedAt === null || p.entries.length > 50 : p.observedAt !== null || p.entries.length > 0) return false;
    const names = new Set<string>();
    return p.entries.every(e => { if (!isEntry(e) || names.has(e.name)) return false; names.add(e.name); return true; });
  });
}
export function isTranscript(v: unknown): v is Transcript {
  return object(v) && states.includes(v.state as PlaceState) && Array.isArray(v.messages) && v.messages.length <= 200
    && (v.state === 'complete' || v.state === 'partial' || v.messages.length === 0)
    && v.messages.every(m => object(m) && text(m.role, 32) && typeof m.text === 'string' && m.text.length <= 16384 && typeof m.cut === 'boolean'
      && Array.isArray(m.tools) && m.tools.every(t => text(t, 256)) && nullableCount(m.at));
}
export async function readSessions(): Promise<Sessions | undefined> {
  if (isTauri()) { try { const v: unknown = await invoke('overview_sessions'); if (isSessions(v)) return v; } catch { /* No native diagnostics reach Overview. */ } }
}
export async function readTranscript(key: string, name: string): Promise<Transcript> {
  if (isTauri()) { try { const v: unknown = await invoke('overview_transcript', { key, name }); if (isTranscript(v)) return v; } catch { /* No native diagnostics reach Overview. */ } }
  return { state: 'unavailable', messages: [] };
}

/** `connection` and `readAt` are the place's connection revision and read time, so expanded details follow both. */
export type Row = SessionEntry & { placeKey: string; connection: number | null; readAt: number | null };
/** Every read place's Sessions, most recently updated first, then by place and name for a stable order. */
export function merge(places: PlaceSessions[]): Row[] {
  return places.flatMap(p => p.entries.map(e => ({ ...e, placeKey: p.key, connection: p.connection, readAt: p.observedAt })))
    .sort((a, b) => b.updatedAt - a.updatedAt || a.placeKey.localeCompare(b.placeKey) || a.name.localeCompare(b.name));
}
const roles: Record<string, string> = { user: 'User', assistant: 'Agent', tool: 'Tool result', system: 'System' };
export const roleLabel = (role: string) => roles[role] ?? role;
