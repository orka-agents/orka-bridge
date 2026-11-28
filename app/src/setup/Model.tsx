import { invoke } from '@tauri-apps/api/core';
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Button } from '../components/Button';
import { StatusPill } from '../components/DesignSystem';
import { Disclosure } from '../components/Disclosure';
import { checkCopilot, companies, connectKey, connectModel, type CopilotCatalog, isPreflightCategory, type KeyVendor, keyFailure, type ModelConnection, type ModelFailure, type ModelRecovery, removeModel, resetModel, validKey } from './modelState';
import type { HelpTopic } from './recovery-info';
import type { FreshRefusal } from './fresh';

// `checking` and `checked` are a Copilot choice checked on this computer while the workspace is still on its way.
export type ModelState = 'idle' | 'queued' | 'checking' | 'checked' | 'connecting' | 'removing' | 'resetting' | 'resetFailed' | 'removalFailed' | 'connected' | 'failed';
const ghLogin = 'gh auth login --hostname github.com --web';
// What to do next after a Copilot preflight refusal; nothing reached the workspace.
function preflightStep(error: ModelFailure, hasSignIn: boolean): { summary: string; step: string; command?: string } | undefined {
  const again = hasSignIn ? ' Bridge saved an earlier sign-in: remove it first, then connect again.' : ' Then press Check again.';
  switch (error.category) {
    case 'modelCopilotTokenType': return { summary: error.message, step: `Sign in to GitHub CLI again with your browser in your terminal.${again}`, command: ghLogin };
    case 'modelCopilotSignInRejected': return { summary: error.message, step: `Sign in again with GitHub CLI in your terminal.${again}`, command: ghLogin };
    case 'modelCopilotAccessDenied': return { summary: `GitHub Copilot isn't available to ${error.account ?? 'this GitHub account'}.`, step: 'Check the account has Copilot, or that your organisation allows Copilot in the command line. Or use your own API key.' };
    case 'modelCopilotUnreachable': return { summary: error.message, step: "Check this computer's internet connection, then press Check again." };
    case 'modelCopilotMalformed': return { summary: error.message, step: 'Press Check again. If it happens again, copy the details for help.' };
    case 'modelCopilotModelNotOffered': return { summary: `${error.wanted ?? 'The chosen model'} isn't offered to ${error.account ?? 'this GitHub account'} any more.`, step: 'Choose another model, then press Connect the AI.' };
  }
}
// `enabled` is native permission to touch credentials. `queueable` only permits remembering the
// creator's choice while the workspace is still arriving. `away` means another place was chosen.
// Each increase of changeRequest explicitly asks to change the AI provider.
// `setupCard` is first setup's own AI card beside the steps: a Copilot failure takes the card, as its own story.
// `diagnostic` is the workspace card's detail line (the guest's typed Connect diagnostic), copied for help.
// `helpLines` are further finite lines for help, such as a fresh workspace's outcome.
// `onFreshWorkspace` asks for Start a fresh workspace, the way out when the workspace refuses cleanup;
// `freshBlocked` is native's reason it can't start, and then the card keeps its own retry instead.
export default function Model({ title = 'Connect the AI', enabled, queueable = false, paused = false, away = false, primary = true, changeRequest = 0, sharedConnection = false, recoveryTarget, choiceTarget, inline = false, onPlaceCard = false, setupCard = false, diagnostic, helpLines, onState, onSettled, onConnectRequest, onFreshWorkspace, freshBlocked }: {
  sharedConnection?: boolean; title?: string; choiceTarget?: HTMLElement | null; inline?: boolean; setupCard?: boolean; diagnostic?: string | null; helpLines?: string[]; onFreshWorkspace?: () => void; freshBlocked?: FreshRefusal;
  enabled: boolean; queueable?: boolean; paused?: boolean; away?: boolean; primary?: boolean; changeRequest?: number; recoveryTarget?: HTMLElement | null; onPlaceCard?: boolean; onState?: (state: ModelState) => void; onSettled?: () => void; onConnectRequest?: () => void;
}) {
  // In-memory consent only. It never carries a credential and is consumed once.
  const [queued, setQueued] = useState(false);
  const [choice, setChoice] = useState<'copilot' | 'key'>('copilot');
  const [vendor, setVendor] = useState<KeyVendor>('anthropic');
  const [connection, setConnection] = useState<ModelConnection>();
  // The account's Copilot models, fetched live, and the person's choice among them.
  const [catalog, setCatalog] = useState<CopilotCatalog>();
  const [model, setModel] = useState<string>();
  const [picking, setPicking] = useState(false);
  const [checking, setChecking] = useState(false);
  const [reconnectFailures, setReconnectFailures] = useState(0);
  const checkRequest = useRef(0);
  const keyField = useRef<HTMLInputElement>(null);
  const [hasKey, setHasKey] = useState(false);
  const clearKey = useCallback(() => { if (keyField.current) keyField.current.value = ''; setHasKey(false); }, []);
  const [expanded, setExpanded] = useState(false);
  const [changing, setChanging] = useState(false);
  const changeButton = useRef<HTMLButtonElement>(null);
  function closeChange() { clearKey(); setExpanded(false); setChanging(false); setConfirmRemoval(false); changeFocus.current = changing || compact ? 'change' : 'panel'; }
  const company = companies[vendor];
  const root = useRef<HTMLElement>(null);
  const status = useRef<HTMLDivElement>(null);
  const focusWithin = useRef(false);
  useEffect(() => {
    // A portal move replaces DOM, not consent. Restore focus only when that DOM owned it.
    if (focusWithin.current && document.activeElement === document.body && !root.current?.closest('[hidden]')) {
      refocus.current = 'connect';
      (status.current ?? heading.current)?.focus();
    }
  }, [inline, choiceTarget]);
  const retry = useRef<HTMLButtonElement>(null);
  const removal = useRef<HTMLButtonElement>(null);
  const doneChange = useRef<HTMLButtonElement>(null);
  const cancelRemoval = useRef<HTMLButtonElement>(null);
  const [confirmRemoval, setConfirmRemoval] = useState(false);
  const heading = useRef<HTMLHeadingElement>(null);
  // An explicit change moves focus once rendered: to the confirmation, else the connection choice.
  const changeFocus = useRef<'confirm' | 'choice' | 'panel' | 'change'>(undefined);
  const handledChange = useRef(changeRequest);
  // Focus follows the keyboard user's operation, then its retry or next available action.
  const refocus = useRef<'connect' | 'remove'>(undefined);
  const [busy, setBusy] = useState<'connect' | 'remove' | 'reset'>();
  const [recovery, setRecovery] = useState<ModelRecovery>();
  const [confirmReset, setConfirmReset] = useState(false);
  const [anotherProvider, setAnotherProvider] = useState(false);
  const [resetDone, setResetDone] = useState(false);
  const [cardRecovery, setCardRecovery] = useState(false);
  const [resetAttempts, setResetAttempts] = useState(0);
  const [details, setDetails] = useState(false);
  // Set when the creator asks to see the provider choices again under a Copilot failure on the setup card.
  const [anotherChoice, setAnotherChoice] = useState(false);
  // Learned from native's refusal: this workspace can't use your own API key. A new workspace remounts the card.
  const [keysUnsupported, setKeysUnsupported] = useState(false);
  const [removed, setRemoved] = useState(false);
  const [connected, setConnected] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const ready = enabled && connected;
  const hasSignIn = !removed && attempted;
  const [error, setError] = useState<ModelFailure>();
  const [notice, setNotice] = useState('');
  const [failedAction, setFailedAction] = useState<'connect' | 'remove' | 'reset'>('connect');
  const removalFailed = !!error && (failedAction === 'remove' || error.category === 'modelKeyCleanup');
  const actionReason = paused ? 'Wait for the workspace action to finish.' : busy ? `Wait for the AI ${busy === 'connect' ? 'connection' : busy === 'remove' ? 'removal' : 'reset'} to finish.` : !enabled ? 'Start the workspace before changing the AI connection.' : undefined;
  const providerReason = busy ? actionReason : hasSignIn ? 'Remove the saved sign-in or key before choosing another provider.' : undefined;
  // Clearing an error advances the same card flow; it must not hide that flow's next state.
  useEffect(() => {
    if (!onPlaceCard) setCardRecovery(false);
    else if (error) setCardRecovery(true);
  }, [onPlaceCard, error]);
  const epoch = useRef(0);
  const active = useRef(false);
  useEffect(() => {
    epoch.current++;
    active.current = false;
    if (!enabled) { setBusy(undefined); setConnected(false); setRemoved(false); setError(undefined); setRecovery(undefined); setConfirmReset(false); }
    return () => { epoch.current++; };
  }, [enabled]);
  useEffect(() => {
    onState?.(!enabled ? error && failedAction === 'connect' ? 'failed' : queued ? checking ? 'checking' : catalog ? 'checked' : 'queued' : 'idle' : busy === 'connect' ? 'connecting' : busy === 'remove' ? 'removing' : busy === 'reset' ? 'resetting' : ready ? 'connected' : error ? failedAction === 'reset' ? 'resetFailed' : failedAction === 'remove' || error.category === 'modelKeyCleanup' ? 'removalFailed' : 'failed' : queued ? 'queued' : 'idle');
  }, [enabled, busy, ready, error, queued, failedAction, onState, checking, catalog]);
  useEffect(() => { if (away || !enabled) clearKey(); if (away) { setQueued(false); checkRequest.current++; setChecking(false); } }, [away, enabled, clearKey]);
  useEffect(() => {
    // Setup can stay mounted under a hidden tab or cluster page.
    const observer = new MutationObserver(() => { if (root.current?.closest('[hidden]')) clearKey(); });
    observer.observe(document.body, { subtree: true, attributes: true, attributeFilter: ['hidden'] });
    return () => observer.disconnect();
  }, [clearKey]);
  // Runs after the enabled reset above. An unreadable observation or workspace mutation holds the intent.
  useEffect(() => {
    if (queued && enabled && !paused && !away) { setQueued(false); void perform('connect'); }
  }, [queued, enabled, paused, away]);
  // Busy and queued states replace the action with status; keep keyboard focus with the operation.
  // Focus is restored only while the operation still owns it: on the page (not hidden), and on nothing,
  // the status or its own action. Anywhere else the creator moved it, so drop the intent and leave focus alone.
  useEffect(() => {
    if (!refocus.current) return;
    const at = document.activeElement;
    const owns = !root.current?.closest('[hidden]') && (!at || at === document.body || at === status.current || at === retry.current || at === removal.current || at === cancelRemoval.current);
    if (!owns) { refocus.current = undefined; return; }
    if (busy || queued) { status.current?.focus(); return; }
    if (confirmRemoval || confirmReset) { cancelRemoval.current?.focus(); return; }
    const next = refocus.current === 'remove' ? removal.current ?? retry.current : retry.current;
    if (next?.disabled) keyField.current?.focus(); else next?.focus();
    if (document.activeElement === document.body) status.current?.focus();
    refocus.current = undefined;
  }, [busy, queued, ready, error, confirmRemoval, confirmReset, recoveryTarget]);
  // Never replaces a saved sign-in directly: the creator confirms removal first, then chooses again.
  useEffect(() => {
    if (changeRequest === handledChange.current) return;
    handledChange.current = changeRequest;
    if (changeRequest <= 0) return;
    if (recovery && enabled && !paused && !busy) {
      if (sharedConnection) return;
      setAnotherProvider(true); setConfirmReset(true); refocus.current = 'connect';
      return;
    }
    const confirm = hasSignIn && enabled && !paused && !busy && !removalFailed;
    changeFocus.current = confirm ? 'confirm' : 'choice';
    if (confirm) { setChanging(true); changeFocus.current = 'panel'; }
    else setExpanded(true);
  }, [changeRequest, hasSignIn, enabled, paused, busy, removalFailed, recovery, sharedConnection]);
  useEffect(() => {
    // A confirmation focuses once it renders; a hidden card never takes focus.
    if (!changeFocus.current || (changeFocus.current === 'confirm' && !confirmRemoval)) return;
    const panelAction = sharedConnection ? doneChange.current : removal.current;
    if (changeFocus.current === 'panel' && !panelAction || changeFocus.current === 'change' && !changeButton.current) return;
    const intent = changeFocus.current;
    changeFocus.current = undefined;
    if (root.current?.closest('[hidden]')) return;
    const target = intent === 'change' ? changeButton.current : intent === 'panel' ? panelAction : confirmRemoval ? cancelRemoval.current
      : busy ? heading.current
      : changing || removalFailed ? removal.current
      : root.current?.querySelector<HTMLElement>('.model-choice input:checked:not(:disabled)');
    (target ?? heading.current)?.focus();
  });
  async function perform(action: 'connect' | 'remove' | 'reset', chosen = model) {
    if (!enabled || paused || active.current || (action === 'connect' && ready)) return;
    if ((action === 'remove' || action === 'reset') && sharedConnection) return;
    if (action === 'reset' && !recovery) return;
    const key = action === 'connect' && choice === 'key' ? keyField.current?.value ?? '' : '';
    if (action === 'connect' && choice === 'key' && (key || !hasSignIn) && !validKey(vendor, key)) {
      clearKey(); setFailedAction('connect'); setError({ category: 'modelKeyInvalid', codes: [], message: 'Paste the whole key.' }); return;
    }
    clearKey();
    active.current = true;
    const request = ++epoch.current;
    const before = { attempted, removed };
    setBusy(action); setError(undefined); setRemoved(false); setNotice(''); setFailedAction(action);
    setAttempted(true); setConfirmRemoval(false); setConfirmReset(false); setResetDone(false); setAnotherChoice(false);
    if (action === 'remove') setChanging(false);
    if (action === 'reset') setResetAttempts(value => value + 1);
    if (action !== 'connect') setConnected(false);
    const result = await (action === 'reset' ? resetModel() : action === 'remove' ? removeModel() : choice === 'key' && key ? connectKey(vendor, key) : choice === 'key' ? connectModel(vendor) : connectModel(undefined, chosen));
    if (request !== epoch.current) return;
    active.current = false; setBusy(undefined);
    if (!result.ok && result.catalog && (result.category === 'modelCopilotChooseModel' || result.category === 'modelCopilotModelNotOffered')) {
      // The sign-in is good: show the account's models. Nothing was saved or sent to the workspace.
      const list = result.catalog;
      const pick = chosen && list.models.some(m => m.id === chosen) ? chosen : list.preselected;
      setCatalog(list); setModel(pick); setAttempted(before.attempted); setRemoved(before.removed);
      if (result.category === 'modelCopilotChooseModel') {
        setError(undefined);
        // On the guided first run the preselected model, named on the card, is the person's confirmation.
        if (setupCard && !picking && pick && !chosen) { void perform('connect', pick); return; }
        setPicking(true);
      } else { setPicking(true); setError(result); }
      onSettled?.();
      return;
    }
    if (result.ok) {
      setConnected(action === 'connect'); setRemoved(action !== 'connect'); setChanging(false); setReconnectFailures(0);
      if (action === 'reset') { setResetDone(true); setRecovery(undefined); setAttempted(false); setDetails(false); if (anotherProvider) setChoice('key'); }
      if (action === 'connect') {
        const info: ModelConnection | undefined = result.connection ?? (choice === 'copilot' && chosen ? { kind: 'copilot', model: chosen } : undefined);
        setConnection(info); if (info) setChoice(info.kind); setPicking(false); setReconnectFailures(0);
        if (info?.kind === 'key') setVendor(info.vendor);
      } else { setConnection(undefined); setCatalog(undefined); setModel(undefined); setPicking(false); }
    } else {
      if (result.vendor) { setChoice('key'); setVendor(result.vendor); }
      if (action === 'connect' && result.category === 'modelBusy') { setAttempted(before.attempted); setRemoved(before.removed); }
      if (['modelCaptureUnavailable', 'modelCaptureFailed', 'modelCaptureInvalid', 'modelCaptureTimedOut'].includes(result.category)) setAttempted(false);
      // A refused preflight saved nothing: this attempt adds no sign-in to remove.
      if (isPreflightCategory(result.category)) { setAttempted(before.attempted); setRemoved(before.removed); }
      // The workspace's image predates API keys: native refused before saving or sending anything, so there is no
      // saved key to remove or check again, and this workspace stays without keys until it is replaced.
      if (result.category === 'modelKeyUnsupported') { setAttempted(before.attempted); setRemoved(before.removed); setKeysUnsupported(true); }
      setError(result);
      // Consecutive refusals: the workspace said no to removing the previous connection. Any other outcome starts over,
      // so an uncertain one still gets its Check again.
      const refusal = result.category === 'modelReconnect' && (!result.cleanup || result.cleanup[1] === 'refused');
      setReconnectFailures(count => refusal ? count + 1 : 0);
      if (result.recovery && (action === 'reset' || result.category === 'modelInstallationMismatch')) setRecovery(result.recovery);
      else if (action === 'reset') setRecovery(value => value && { ...value, resetStage: undefined });
      if ((choice === 'key' || result.vendor) && ['modelCredentialRejected', 'modelProviderUnreachable', 'modelModelUnavailable'].includes(result.category)) setRemoved(true);
    }
    onSettled?.();
  }
  async function help(topic: HelpTopic) {
    try { await invoke('setup_open_help', { topic }); setNotice('Help opened in your browser.'); }
    catch { setNotice(choice === 'key' ? `Couldn't open help. Open ${company.site} to create a key.` : "Couldn't open help. Go to cli.github.com for installation and sign-in instructions."); }
  }
  async function copy() {
    if (!error) return;
    try {
      // Finite categories, ids and the workspace's own diagnostic line only: never a token or an e-mail.
      await navigator.clipboard.writeText(['Bridge AI connection', `Operation: ${failedAction === 'reset' ? 'Resetting AI connection' : failedAction === 'remove' ? 'Removing sign-in' : checking || !enabled ? 'Checking the AI sign-in' : 'Connecting AI'}`, `Category: ${error.category}`,
        ...(isPreflightCategory(error.category) ? [`Copilot preflight: ${error.category.replace('modelCopilot', '')}`] : []),
        ...(error.wanted ? [`Chosen model: ${error.wanted}`] : []),
        ...(error.cleanup ? [`Cleanup operation: ${error.cleanup[0]}`, `Cleanup class: ${error.cleanup[1]}`] : []),
        ...(diagnostic ? [`Workspace detail: ${diagnostic}`] : []),
        ...(helpLines ?? []),
        ...error.codes.map(code => `OS code: ${code}`),
        ...(recovery ? [`Installation: ${recovery.installation}`, `Saved installation: ${recovery.savedInstallation}`, `Native credential slot: ${recovery.credentialSlot}`, `Reset stage: ${recovery.resetStage ?? (failedAction === 'reset' ? 'Unconfirmed' : 'Not started')}`] : [])].join('\n'));
      setNotice('Details copied. No keys or sign-in tokens are included.');
    } catch { setNotice("Couldn't copy. Share the visible error message for help."); }
  }
  function choose() {
    onConnectRequest?.();
    refocus.current = document.activeElement === retry.current ? 'connect' : undefined;
    if (enabled) void perform('connect'); else if (choice === 'copilot' && queueable && !paused) { setQueued(true); setExpanded(false); changeButton.current?.focus(); void precheck(); }
  }
  // While the workspace is on its way: check the GitHub CLI sign-in and list the account's models on
  // this computer. Nothing is saved or sent to the workspace; Connect checks again if this goes stale.
  async function precheck() {
    // The latest request wins; one the person cancelled or moved away from is dropped.
    const request = ++checkRequest.current;
    setChecking(true); setError(undefined); setFailedAction('connect'); setNotice('');
    const result = await checkCopilot();
    if (request !== checkRequest.current) return;
    setChecking(false);
    if (result.ok) {
      setCatalog(result.catalog);
      setModel(value => value && result.catalog.models.some(m => m.id === value) ? value : result.catalog.preselected);
      if (!result.catalog.models.length) setQueued(false);
    } else if (isPreflightCategory(result.category) || result.category.startsWith('modelCapture')) { setQueued(false); setError(result); }
    // Otherwise this build couldn't check early: the choice stays queued and Connect checks before the workspace is touched.
  }
  const cliUnavailable = !removalFailed && error?.category === 'modelCaptureUnavailable';
  const signInCheck = !removalFailed && error && ['modelCaptureFailed', 'modelCaptureInvalid', 'modelCaptureTimedOut'].includes(error.category);
  // Installed but not signed in, or its sign-in unreadable: signing in is the next step.
  const signedOut = !removalFailed && error?.category === 'modelCaptureFailed';
  // Bridge couldn't read a sign-in, so nothing was saved or sent.
  const nothingRead = !!cliUnavailable || !!signInCheck;
  // A route's own error and help notice belong to that route: changing route clears them.
  function chooseKind(kind: 'copilot' | 'key') { clearKey(); setQueued(false); setChoice(kind); setError(undefined); setNotice(''); setAnotherChoice(false); setPicking(false); }
  const keyMode = choice === 'key';
  const keyProblem = keyMode && error && !removalFailed;
  // A key the company refused, or one that isn't whole, is an error on the field itself.
  const fieldError = keyProblem && ['modelKeyInvalid', 'modelCredentialRejected'].includes(error.category);
  const callout = !!error && !fieldError;
  const sharedNotice = <p>To change provider, use Remove saved connection in the Workspace card. Its confirmation explains removal for all retained workspaces.</p>;
  const removalAction = sharedConnection ? sharedNotice : hasSignIn && !busy && <Button ref={removal} type="button" kind={removalFailed ? 'primary' : 'quiet'} className={removalFailed ? undefined : 'button-danger'} disabled={!enabled || paused || !!busy}
            onClick={() => {
              refocus.current = document.activeElement === removal.current ? 'remove' : undefined;
              if (removalFailed) void perform('remove'); else setConfirmRemoval(true);
            }}>{removalFailed ? 'Retry removal' : changing ? keyMode ? 'Remove saved key' : 'Remove saved sign-in' : keyMode ? 'Remove key' : 'Remove sign-in'}</Button>;
  if (recovery) {
    const resetFailed = failedAction === 'reset' && !!error;
    const custody = recovery.resetStage === 'recordRemovalUnconfirmed'
      ? 'Native credential removed; saved record removal not confirmed.'
      : recovery.resetStage === 'credentialRemovalUnconfirmed' ? 'Native credential removal not confirmed; saved record kept.' : resetFailed ? 'Bridge could not check what is still saved. The identifiers below are from the last confirmed record.' : 'Saved record: model-v1. Native credential removal has not been requested.';
    // On its own card the recovery has a head; what happened sits in the callout, the choices stay below it.
    const content = <section ref={root} className={recoveryTarget ? 'model-recovery' : 'card setup-model model-recovery'} aria-label="Saved AI connection" aria-busy={!!busy}>
      {!recoveryTarget && <div className="steps-head"><h2>Connect the AI</h2>{busy !== 'reset' && <StatusPill state="needs-you">Needs you</StatusPill>}</div>}
      {actionReason && <p className="muted">{actionReason}</p>}
      {busy === 'reset' ? <div ref={status} role="status" tabIndex={-1} className="model-state"><StatusPill state="running">Resetting</StatusPill><p>Removing Bridge's saved AI connection. Your workspace and agents aren't touched.</p></div>
        : <div className="workspace-failure" role="alert"><div className="workspace-failure-explanation">
          {resetFailed ? <><h2 className="failure-summary">Bridge couldn't reset the AI connection</h2><p>{resetAttempts > 1 ? 'Reset failed again. Copy the details for whoever helps you with Bridge; Show details lists what Bridge could confirm.' : "Bridge couldn't confirm the saved connection was removed. Try once more. If it fails again, copy the details for whoever helps you with Bridge."}</p></>
            : <><p className="failure-summary">Bridge's saved AI connection is from an earlier setup.</p><p>{sharedConnection ? 'Your agents are safe.' : 'This happens after a reinstall. Reset it and choose the AI again. Your agents are safe.'}</p></>}
          </div>
          {sharedConnection ? sharedNotice : <div className="workspace-step-actions">
            <Button ref={confirmReset ? undefined : retry} kind="primary" disabled={!enabled || paused || !!busy} onClick={() => { refocus.current = 'connect'; if (resetFailed) void perform('reset'); else { setAnotherProvider(false); setConfirmReset(true); } }}>{resetFailed ? 'Reset again' : 'Reset AI connection'}</Button>
            {!resetFailed && <Button kind="secondary" disabled={!enabled || paused || !!busy} onClick={() => { refocus.current = 'connect'; setAnotherProvider(true); setConfirmReset(true); }}>Use another provider</Button>}
            <Button kind="quiet" onClick={() => { void copy(); }}>Copy details for help</Button>
          </div>}
          {!sharedConnection && confirmReset && <div role="group" aria-label="Reset AI connection confirmation"><h3>Reset the AI connection?</h3><p>Bridge forgets its saved AI connection for this workspace. Your GitHub account and your agents don't change. Use another provider starts with this same reset.</p><div className="workspace-step-actions">
            <Button ref={retry} kind="primary" disabled={!enabled || paused || !!busy} onClick={() => { refocus.current = 'connect'; void perform('reset'); }}>Reset</Button>
            <Button ref={cancelRemoval} kind="secondary" onClick={() => { refocus.current = 'connect'; setConfirmReset(false); }}>Cancel</Button>
          </div></div>}
          <Disclosure label={details ? 'Hide details' : 'Show details'} open={details} onOpenChange={setDetails} controls="model-recovery-details">
            <div id="model-recovery-details" hidden={!details}><p>{custody}</p><dl className="model-recovery-details"><dt>Category</dt><dd>{error?.category ?? 'modelInstallationMismatch'}</dd><dt>This installation</dt><dd>{recovery.installation}</dd><dt>Saved installation</dt><dd>{recovery.savedInstallation}</dd><dt>Saved record</dt><dd>Bridge app data: model-v1</dd><dt>Native credential slot</dt><dd>{recovery.credentialSlot}</dd></dl></div>
          </Disclosure>
        </div>}
      {notice && <p role="status">{notice}</p>}
    </section>;
    return recoveryTarget ? createPortal(content, recoveryTarget) : content;
  }
  const portaled = !!recoveryTarget && (!!error || onPlaceCard && (resetDone || cardRecovery));
  const compact = inline && !portaled;
  // This computer's own AI card names its state in the head: connecting, connected, or couldn't connect.
  const ownCard = title === 'AI' && !compact && !portaled;
  const connectFailed = !!error && failedAction === 'connect' && !removalFailed;
  // On the setup card a Copilot failure is the card's whole story, as on the boards: the choices wait behind a quiet action.
  const setupFailure = setupCard && !compact && !portaled && connectFailed && callout && !keyMode;
  const choicesShown = !setupFailure || anotherChoice;
  // A queued choice on the setup card reads as the board draws it: the provider and model, Queued, and Cancel.
  const queuedCard = setupCard && queued && !compact && !portaled;
  const headTitle = removalFailed ? `Couldn't remove the saved ${keyMode ? 'key' : 'sign-in'}` : (ownCard || setupFailure) && !ready ? connectFailed ? "The AI couldn't connect" : 'Connect the AI' : title;
  const headBusy = ownCard && !!busy;
  const removalConfirm = <div role="group" aria-label={`Remove ${keyMode ? 'key' : 'sign-in'} confirmation`}>
    <p>{keyMode ? 'Disconnect the AI and remove Bridge’s saved key? This does not revoke the key with the company.' : 'Disconnect the AI and remove Bridge’s saved sign-in? GitHub CLI stays signed in.'}</p>
    <div className="workspace-step-actions">
      <Button ref={cancelRemoval} type="button" kind="primary" onClick={() => {
        refocus.current = document.activeElement === cancelRemoval.current ? 'remove' : undefined;
        setConfirmRemoval(false);
      }}>Cancel</Button>
      <Button ref={removal} type="button" kind="secondary" className="button-danger" disabled={!enabled || paused || !!busy} onClick={() => {
        refocus.current = document.activeElement === removal.current ? 'remove' : undefined;
        void perform('remove');
      }}>{keyMode ? 'Remove key' : 'Remove sign-in'}</Button>
    </div>
  </div>;
  const connecting = headBusy && busy === 'connect';
  const keyForm = <div className="model-key-form">
    <fieldset className="view-switch"><legend className="sr-only">Company</legend>
      {(['openai', 'anthropic'] as const).map(item => <button key={item} type="button" aria-pressed={vendor === item} disabled={!!busy || hasSignIn} onClick={() => { clearKey(); setVendor(item); setError(undefined); }}>{companies[item].name}</button>)}
    </fieldset>
    <label htmlFor="model-api-key">API key</label>
    <input ref={keyField} className="field" onChange={event => setHasKey(!!event.currentTarget.value.trim())} id="model-api-key" type="password" autoComplete="off" spellCheck={false} autoCapitalize="none" disabled={!enabled || paused || !!busy} aria-describedby={fieldError ? 'model-key-error model-key-help' : 'model-key-help'} aria-invalid={error?.category === 'modelKeyInvalid' || error?.category === 'modelCredentialRejected'} />
    {fieldError && <p id="model-key-error" role="alert" className="field-error">{keyFailure(error, vendor)}</p>}
    <p id="model-key-help" className="muted">Stored on this computer, never in your agent files.</p>
    <a href={`https://${company.site}`} onClick={event => { event.preventDefault(); void help(vendor === 'openai' ? 'openaiKey' : 'anthropicKey'); }}>Where do I get a key?</a>
  </div>;
  const noModels = !!catalog && !catalog.models.length;
  const picker = !!catalog?.models.length && <div className="model-picker">
    <label htmlFor="model-copilot-model">Model</label>
    <select id="model-copilot-model" className="field" value={model ?? ''} disabled={!!busy || checking} onChange={event => { setModel(event.currentTarget.value); if (error?.category === 'modelCopilotModelNotOffered') setError(undefined); }}>
      {catalog.models.map(m => <option key={m.id} value={m.id}>{m.name ? `${m.id} · ${m.name}` : m.id}</option>)}
    </select>
    <p className="muted">The models your Copilot plan offers for agents, as GitHub lists them.</p>
  </div>;
  // Copilot's model comes from the account's own list; before the check there is nothing to name.
  const copilotModelLine = noModels ? <p>GitHub Copilot doesn't offer {catalog.account ?? 'this GitHub account'} a model Bridge can use for agents. Check the account's Copilot plan, or use your own API key.</p>
    : catalog && model && setupCard && !picking ? <p className="model-chosen">Model <code>{model}</code> <Button type="button" kind="quiet" aria-label="Change model" disabled={!!busy || checking} onClick={() => setPicking(true)}>Change</Button></p>
    : catalog ? picker
    : <p className="muted">Bridge checks your GitHub sign-in, then lists the models your Copilot plan offers.</p>;
  // What the selected choice means: on first setup it sits inside the selected option; elsewhere it follows both options.
  const choiceDetail = <>
    {keyMode && keysUnsupported && !error && <p className="muted">The workspace on this computer can't use your own API key yet. Choose GitHub Copilot instead.</p>}
    {keyMode ? keyForm : !connecting && <p className="muted">Use GitHub Copilot. GitHub CLI must be installed and signed in to github.com.</p>}
    {keyMode ? <p>Model <code>{company.model}</code>, a good default.</p> : copilotModelLine}
    {!enabled && keyMode ? <p className="muted">Bridge can't queue a key. Enter it once the workspace is ready.</p>
      : !enabled && (queueable || queued) ? (!compact || expanded) && <p className="muted">Choose now to connect automatically when the workspace is ready.</p>
      : !enabled ? <p className="muted">Start the workspace before connecting the AI.</p>
      : !keyMode && <p className="muted">Your sign-in is shared only with your workspace.</p>}
  </>;
  const option = (kind: 'copilot' | 'key', label: ReactNode) => <div className={`model-option${(kind === 'key') === keyMode ? ' model-option-selected' : ''}`}>
    <label><input type="radio" name="model-kind" checked={(kind === 'key') === keyMode} disabled={!!busy || hasSignIn} onChange={() => chooseKind(kind)} /> {label}</label>
    {(kind === 'key') === keyMode && <div className="model-option-detail">{choiceDetail}</div>}
  </div>;
  // The actions that answer a failure: retry first; on the setup card, help at full size and copying on its own row.
  const failureHelp = <>
    {cliUnavailable && <Button type="button" kind={setupFailure ? 'secondary' : 'quiet'} onClick={() => { void help('githubCliInstall'); }}>Install GitHub CLI</Button>}
    {(cliUnavailable || signInCheck) && <Button type="button" kind="quiet" onClick={() => { void help('githubSignIn'); }}>Sign-in help</Button>}
  </>;
  const preflight = error && !removalFailed && !keyMode ? preflightStep(error, hasSignIn) : undefined;
  // Cleanup refused twice in a row, after one Check again or Retry removal: a host-only reset can't clear the workspace's
  // own record of the old connection, so the card offers a fresh workspace instead of another round.
  const stuck = error?.category === 'modelReconnect' && reconnectFailures >= 2;
  const freshAction = stuck && !!onFreshWorkspace && !sharedConnection && !freshBlocked;
  const summary = preflight?.summary ?? error?.message;
  const nextStep = preflight?.step ?? (stuck ? freshAction ? "Bridge can't remove it from here, and resetting Bridge's saved connection wouldn't clear the workspace's own record. A fresh workspace is the sure way back: Bridge replaces the workspace on this computer, keeps your agent files, and you connect the AI again."
    : freshBlocked === 'updateHistory' ? "Bridge can't remove it from here, and resetting Bridge's saved connection wouldn't clear the workspace's own record. A fresh workspace would, but this workspace came from a workspace update and Bridge keeps the earlier workspace so it can be restored, so it can't start one over that yet. Try again later, or copy the details for whoever helps you with Bridge. Your agent files are kept."
    : freshBlocked ? "Bridge can't remove it from here, and resetting Bridge's saved connection wouldn't clear the workspace's own record. A fresh workspace isn't available right now. Try again later, or copy the details for whoever helps you with Bridge. Your agent files are kept."
    : "Bridge can't remove it from here, and resetting Bridge's saved connection wouldn't clear the workspace's own record. A fresh workspace is the sure way back. Copy the details for whoever helps you with Bridge. Your agent files are kept."
    : cliUnavailable ? 'Install GitHub CLI if needed, then sign in to github.com in your terminal:'
    : signedOut ? 'Sign in to github.com with GitHub CLI in your terminal, then press Check again:'
    : signInCheck ? 'Check your GitHub CLI sign-in in your terminal:'
    : error?.category === 'modelBusy' ? 'Wait for the other operation to finish, then press Check again.'
    : 'Press Check again to reconnect. If it happens again, copy the details for help.');
  const command = preflight ? preflight.command : cliUnavailable || signedOut ? ghLogin : signInCheck ? 'gh auth status --hostname github.com' : undefined;
  const otherRoute = error?.category === 'modelCopilotAccessDenied' && !hasSignIn && !busy && <Button type="button" kind="secondary" onClick={() => chooseKind('key')}>Use your own API key</Button>;
  const copyAction = error && <Button type="button" kind={setupFailure ? 'secondary' : 'quiet'} onClick={() => { void copy(); }}>Copy details for help</Button>;
  const reconnectHint = !removalFailed && <p className="muted">Check again reconnects the AI using your saved sign-in when available.</p>;
  // A key that is cleared after a refusal waits for the next one: the action keeps its place and says what it needs.
  const awaitingKey = keyMode && !hasKey && !hasSignIn && !!enabled && !paused && !busy;
  const actions = <div className="workspace-step-actions">
    {changing && <fieldset className="workspace-change" aria-label="Change the AI">
      <legend>Change the AI</legend><p>Remove the saved {keyMode ? 'key' : 'sign-in'} before choosing another provider.</p>
      {confirmRemoval && !sharedConnection && removalConfirm}
      <div className="workspace-step-actions">{!confirmRemoval && removalAction}<Button ref={doneChange} type="button" kind="secondary" onClick={closeChange}>Done</Button></div>
    </fieldset>}
    {confirmRemoval && !sharedConnection ? !changing && removalConfirm : <>
    {queued || busy ? !headBusy && <div ref={status} role={queued ? 'status' : undefined} tabIndex={-1} className="model-state">
      {!queuedCard && <StatusPill state={ready ? 'passed' : queued ? 'not-started' : 'running'}>{busy === 'remove' ? 'Removing' : ready ? 'Connected' : queued ? 'Queued' : keyMode ? 'Checking' : 'Connecting'}</StatusPill>}
      {busy && (busy === 'remove' || !keyMode) && <span className="status-description">{busy === 'remove' ? keyMode ? 'Removing key…' : 'Removing sign-in…' : 'Connecting the AI…'}</span>}
      {queued && <><span className="status-description">{checking ? 'Checking your GitHub sign-in. Connects when the workspace is ready.' : catalog && model ? `GitHub Copilot is ready: ${catalog.account ? `signed in as ${catalog.account}, ` : ''}model ${model}. Connects when the workspace is ready.` : 'Connects when the workspace is ready.'}</span>{catalog && model && (!compact || expanded) && <Button type="button" kind="quiet" aria-label="Change model" onClick={() => { setQueued(false); setPicking(true); }}>Change</Button>}{(!compact || expanded) && <Button type="button" kind="secondary" aria-label={compact ? undefined : 'Cancel queued connection'} onClick={() => { setQueued(false); checkRequest.current++; setChecking(false); if (compact) { setExpanded(false); changeButton.current?.focus(); } }}>{compact ? "Don't connect automatically" : 'Cancel'}</Button>}</>}
    </div>
    : freshAction ? <Button key="model-fresh" ref={retry} type="button" kind="primary" disabled={paused || !!busy} onClick={onFreshWorkspace}>Start a fresh workspace</Button>
    // Without keys on this workspace, the way forward is the other route, not another try with a key.
    : keyMode && keysUnsupported && !hasSignIn && !ready ? <Button key="model-copilot" ref={retry} type="button" kind="primary" disabled={paused || !!busy} onClick={() => chooseKind('copilot')}>Use GitHub Copilot</Button>
    : !ready && !removalFailed && <Button key="model-connect" ref={retry} type="button" kind={primary ? 'primary' : 'secondary'} className={awaitingKey && primary ? 'model-connect-waiting' : undefined} aria-describedby={awaitingKey ? fieldError ? 'model-key-error' : 'model-key-help' : undefined} disabled={(!enabled && (!queueable || keyMode)) || paused || !!busy || (keyMode && !hasKey && !hasSignIn) || (!keyMode && noModels && !error)} onClick={choose}>
      {error && failedAction === 'connect' && error.category !== 'modelCopilotModelNotOffered' ? 'Check again' : 'Connect the AI'}
    </Button>}
    {ready && !changing && <Button ref={changeButton} type="button" kind="quiet" title={actionReason} disabled={paused || !!busy} aria-expanded={false} onClick={event => { if (document.activeElement === event.currentTarget) changeFocus.current = 'panel'; setChanging(true); }}>Change</Button>}
    {removalFailed && !freshAction && removalAction}
    {setupFailure ? failureHelp : <>{copyAction}{failureHelp}</>}
    {!ready && !removalFailed && !freshAction && removalAction}
    </>}
  </div>;
  const content = <section ref={root} className={portaled || compact && choiceTarget ? 'setup-model' : `card setup-model${compact ? ' model-compact' : setupCard && !ready ? ' model-connect' : ''}`} aria-labelledby="model-title" aria-busy={!!busy} onFocusCapture={() => { focusWithin.current = true; }} onBlurCapture={event => { if (event.relatedTarget) focusWithin.current = !!root.current?.contains(event.relatedTarget); }} onKeyDown={event => { if (event.key === 'Escape' && !busy && (changing || confirmRemoval || compact && expanded)) { event.stopPropagation(); closeChange(); } }}>
    <div className={compact ? 'model-inline-head' : 'steps-head'}>
      <h2 id="model-title" ref={heading} tabIndex={-1} className={compact ? 'sr-only' : undefined}>{headTitle}</h2>
      {compact && <span role="status" aria-atomic="true"><b>{keyMode ? 'Your own API key' : 'GitHub Copilot'}</b>{queued ? ', connects when the workspace is ready' : ', connect at step 3'}</span>}
      {ready && <div ref={status} role="status" tabIndex={-1} className="model-state"><StatusPill state="passed">Connected</StatusPill></div>}
      {headBusy && !ready && <div ref={status} tabIndex={-1} className="model-state"><StatusPill state="running">{busy === 'remove' ? 'Removing' : keyMode ? 'Checking' : 'Connecting'}</StatusPill></div>}
      {ownCard && connectFailed && !busy && <StatusPill state="needs-you">Needs you</StatusPill>}
      {queuedCard && <StatusPill state="not-started">Queued</StatusPill>}
      {compact && <Button ref={changeButton} type="button" kind="quiet" disabled={paused || !!busy} aria-expanded={expanded} onClick={() => setExpanded(value => !value)}>Change</Button>}
    </div>
    {compact && !expanded && !queued && !choiceTarget && !keyMode && !enabled && queueable && <p className="muted model-compact-hint">Choose now to connect automatically when the workspace is ready.</p>}
    <div hidden={compact && !expanded}>
    {actionReason && (paused || busy && !headBusy || removalFailed) && <p className="muted">{actionReason}</p>}
    {!ready && busy !== 'remove' && !removalFailed && <>
      {queuedCard ? <p>{keyMode ? 'Your own API key' : 'GitHub Copilot'}{(keyMode || model) && <> · <code>{keyMode ? company.model : model}</code></>}</p>
        : !connecting && choicesShown ? <><fieldset className="model-choice" title={providerReason}>
        <legend className="sr-only">AI connection</legend>
        {option('copilot', 'GitHub Copilot')}
        {option('key', <>Your own API key <span className="muted">(OpenAI, Anthropic)</span></>)}
      </fieldset>
      {providerReason && providerReason !== actionReason && <p className="muted">{providerReason}</p>}</>
        : !setupFailure && choiceDetail}
    </>}
    {ready && connection && <>
      <p>{connection.kind === 'key' ? `${companies[connection.vendor].name} with your API key` : 'GitHub Copilot'} · <code>{connection.model}</code></p>
      {connection.kind === 'copilot' && <p className="muted">Saved GitHub sign-in: {connection.account ?? 'account unavailable'}</p>}
    </>}
    {setupFailure ? <>
      <div className="workspace-failure" role="alert"><div className="workspace-failure-explanation">
        <p className="failure-summary">{summary}</p>
        <p>{nextStep}</p>
        {command && <pre>{command}</pre>}
        {error.category === 'modelCopilotModelNotOffered' && picker}
        <p className="muted">{preflight || nothingRead ? 'Nothing was sent to your workspace.' : 'Your workspace and agent files are kept. A sign-in may already have been saved.'}</p>
      </div></div>
      {actions}
      {otherRoute && <div className="workspace-step-actions">{otherRoute}</div>}
      <div className="workspace-step-actions model-copy-row">{copyAction}</div>
      {!freshAction && reconnectHint}
      {/* Choosing again leaves this route's failure behind: its error and help notice go with it. */}
      {!anotherChoice && !hasSignIn && !busy && <Button type="button" kind="quiet" className="model-another" onClick={() => { setError(undefined); setNotice(''); setAnotherChoice(true); }}>Choose another provider</Button>}
    </> : <div className={callout ? 'workspace-failure' : undefined} role={callout ? 'alert' : undefined}>
      <div className={callout ? 'workspace-failure-explanation' : undefined}>
        {callout && <><p className="failure-summary">{removalFailed && error.category === 'unknown' ? `Bridge could not confirm ${keyMode ? 'key' : 'sign-in'} removal.` : keyMode ? keyFailure(error, vendor) : summary}</p>
        {!keyProblem && <><p>{removalFailed && !stuck ? 'Press Retry removal to retry cleanup. If it happens again, copy the details for help.' : nextStep}</p>
        {command && <pre>{command}</pre>}
        <p>{preflight || nothingRead ? 'Nothing was sent to your workspace.' : `Your workspace and agent files are kept. ${removalFailed ? 'Removal has not been confirmed.' : 'A sign-in may already have been saved.'}`}</p>
        {otherRoute}
        {!preflight && !freshAction && reconnectHint}</>}
        </>}
        {actions}
      </div>
    </div>}
    {busy === 'connect' && <p role="status">{keyMode ? `Asking ${company.name} to confirm the key…` : 'Connecting with your saved sign-in. This may take a few seconds…'}</p>}
    {headBusy && busy === 'connect' && <div className="infra-bar indeterminate" aria-hidden="true"><span /></div>}
    {busy === 'remove' && <p role="status">Disconnecting the AI and removing Bridge’s saved {keyMode ? 'key' : 'sign-in'}…</p>}
    {resetDone && <p role="status">Connection reset. Choose the AI again.</p>}
    {removed && !resetDone && !fieldError && <p role="status">{keyMode ? 'Key removed.' : 'Saved sign-in removed.'}</p>}
    {notice && <p role="status">{notice}</p>}
    {compact && expanded && <Button type="button" kind="quiet" onClick={closeChange}>Done</Button>}
    </div>
  </section>;
  return portaled && recoveryTarget ? createPortal(content, recoveryTarget) : compact && choiceTarget ? createPortal(content, choiceTarget) : content;
}
