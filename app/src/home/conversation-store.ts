import { useSyncExternalStore } from 'react';
import type { AgentProposal, TaskProposal } from './cards';

// What the person did with each card in the current conversation, kept for this run of Bridge so Home can be left
// and reopened. The Navigator's turns themselves stay in native memory; a card stores only the id of what it created.
export type TaskOutcome = { phase: string; answer: string | null; tools: string[]; startedAt: number; finishedAt: number | null; stopped: boolean; reached: number };
export type TaskRun = { documentId: string; agent: string; message: string; requestedAt: number; outcome?: TaskOutcome };
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

let state: ConversationState = { startVersion: 0, cards: {} };
const listeners = new Set<() => void>();
export function conversationState() { return state; }
export function updateConversation(change: (current: ConversationState) => ConversationState) {
  state = change(state);
  for (const listener of listeners) listener();
}
export function updateCard(key: string, change: (current: CardRecord) => CardRecord) {
  updateConversation(current => ({ ...current, cards: { ...current.cards, [key]: change(current.cards[key] ?? { status: 'proposed' }) } }));
}
// Tests start each case from an empty conversation.
export function resetConversation() { state = { startVersion: 0, cards: {} }; for (const listener of listeners) listener(); }
export function useConversation() {
  return useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener); }; }, conversationState);
}
