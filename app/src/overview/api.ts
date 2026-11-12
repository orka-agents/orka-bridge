import { invoke, isTauri } from '@tauri-apps/api/core';
import { isDocumentId, isSourceId, type SourceId } from '../designer/agent-api';
export type SavedFile = { documentId: string; name: string; state: 'readable' | 'unreadable'; source: SourceId | null; summary: { name: string | null; description: string | null; model: string; tools: string[] } | null; reason: string | null };
export type SavedFiles = { state: 'complete' | 'unavailable'; files: SavedFile[]; runActive: boolean };
export type Agent = { name: string; namespace: string; uid: string };
export type Observation = { installation: string; documentId: string; source: SourceId; namespace: string; agentName: string; agentUid: string; observedAt: number; state: 'running' | 'complete' | 'unknown'; report: { verdict: 'passed' | 'failed' | 'unknown' } | null };
export type LiveAgents = { state: 'complete' | 'busy' | 'unavailable'; installation: string | null; agents: Agent[]; observedAt: number | null; observation: Observation | null; runActive: boolean; reason: 'image' | 'stopped' | 'read' | null };
const object = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const text = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && v.length <= 1048576;
const nullableText = (v: unknown) => v === null || typeof v === 'string' && v.length <= 1048576;
const time = (v: unknown) => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
function summary(v: unknown) {
  return v === null || object(v) && nullableText(v.name) && nullableText(v.description) && text(v.model) && Array.isArray(v.tools) && v.tools.every(text);
}
function observation(v: unknown) {
  return v === null || object(v) && isDocumentId(v.installation) && isDocumentId(v.documentId) && isSourceId(v.source) && text(v.namespace) && text(v.agentName) && text(v.agentUid) && time(v.observedAt)
    && ['running', 'complete', 'unknown'].includes(String(v.state)) && (v.report === null || object(v.report) && ['passed', 'failed', 'unknown'].includes(String(v.report.verdict)));
}
export function isSavedFiles(v: unknown): v is SavedFiles {
  return object(v) && ['complete', 'unavailable'].includes(String(v.state)) && typeof v.runActive === 'boolean' && Array.isArray(v.files) && v.files.every(f => object(f) && isDocumentId(f.documentId) && text(f.name) && ['readable', 'unreadable'].includes(String(f.state)) && (f.source === null || isSourceId(f.source)) && summary(f.summary) && nullableText(f.reason));
}
export function isLiveAgents(v: unknown): v is LiveAgents {
  return object(v) && ['complete', 'busy', 'unavailable'].includes(String(v.state)) && typeof v.runActive === 'boolean' && (v.installation === null || isDocumentId(v.installation)) && (v.observedAt === null || time(v.observedAt)) && (v.reason === null || v.reason === 'image' || v.reason === 'stopped' || v.reason === 'read') && observation(v.observation)
    && Array.isArray(v.agents) && v.agents.every(a => object(a) && text(a.name) && text(a.namespace) && text(a.uid))
    && (v.state !== 'complete' || v.installation !== null && v.observedAt !== null);
}
export const unavailableFiles: SavedFiles = { state: 'unavailable', files: [], runActive: false };
export const unavailableAgents: LiveAgents = { state: 'unavailable', installation: null, agents: [], observedAt: null, observation: null, runActive: false, reason: 'read' };
export async function readFiles(): Promise<SavedFiles> {
  if (isTauri()) { try { const v: unknown = await invoke('overview_files'); if (isSavedFiles(v)) return v; } catch { /* Do not project native diagnostics. */ } }
  return unavailableFiles;
}
export async function readAgents(): Promise<LiveAgents> {
  if (isTauri()) { try { const v: unknown = await invoke('overview_agents'); if (isLiveAgents(v)) return v; } catch { /* Do not project native diagnostics. */ } }
  return unavailableAgents;
}
