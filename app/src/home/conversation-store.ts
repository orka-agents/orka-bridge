import { useSyncExternalStore } from 'react';
import type { AgentProposal, TaskProposal } from './cards';

// What the person did with each card in the current conversation, kept for this run of Bridge so Home can be left
// and reopened. The Navigator's turns themselves stay in native memory; a card stores only the id of what it created.
export type TaskOutcome = { phase: string; answer: string | null; tools: string[]; startedAt: number; finishedAt: number | null; stopped: boolean; reached: number };
// `where` names the cluster a task was created on; absent means this computer.
export type TaskRun = { documentId: string; agent: string; message: string; requestedAt: number; where?: string; outcome?: TaskOutcome };
export type CardRecord = {
  status: 'proposed' | 'editing' | 'dismissed' | 'created';
  edited?: TaskProposal | AgentProposal;
  task?: TaskRun;
  agent?: { documentId: string; name: string };
};
export type ConversationState = {
  // Versions of the native conversation before this index belong to earlier conversations.
  startVersion: number;
  cards: Record<string, CardRecord>;
};

// One conversation per place, like the native ones: '' is this computer, otherwise the cluster target and AI connection.
const empty: ConversationState = { startVersion: 0, cards: {} };
let places: Record<string, ConversationState> = {};
let selected = '';
const listeners = new Set<() => void>();
const notify = () => { for (const listener of listeners) listener(); };
export function conversationState() { return places[selected] ?? empty; }
// Home shows the conversation of the selected place; switching never changes another place's record.
export function selectConversationPlace(key: string) {
  if (key === selected) return;
  selected = key; notify();
}
export function updateConversation(change: (current: ConversationState) => ConversationState) {
  places = { ...places, [selected]: change(conversationState()) };
  notify();
}
export function updateCard(key: string, change: (current: CardRecord) => CardRecord) {
  updateConversation(current => ({ ...current, cards: { ...current.cards, [key]: change(current.cards[key] ?? { status: 'proposed' }) } }));
}
// Tests start each case from an empty conversation on this computer.
export function resetConversation() { places = {}; selected = ''; notify(); }
export function useConversation() {
  return useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener); }; }, conversationState);
}
