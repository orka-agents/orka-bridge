export type HelpTopic = 'virtualization' | 'hypervisor' | 'workspace' | 'githubCliInstall' | 'githubSignIn';
export type RecoveryInfo = { summary: string; action: string; topic?: HelpTopic; stage: string; retryBlocked?: boolean; heading?: string; preservation?: string; category?: string };
const operations = ['requirements', 'download', 'verify', 'prepare', 'start', 'services', 'stop'] as const;
const causes = ['permission', 'noSpace', 'integrity', 'inUse', 'network', 'preparation', 'guestConnection', 'services', 'stopUnconfirmed', 'unknown'] as const;
export type WorkspaceFailure = { operation: typeof operations[number]; cause: typeof causes[number]; codes: number[] };
export const isOsCode = (value: unknown): value is number => typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 0xffffffff;
export function isWorkspaceFailure(value: unknown): value is WorkspaceFailure {
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).sort().join(',') !== 'cause,codes,operation') return false;
  const item = value as Record<string, unknown>;
  return operations.some(operation => operation === item.operation) && causes.some(cause => cause === item.cause)
    && Array.isArray(item.codes) && item.codes.length <= 4 && item.codes.every(isOsCode);
}
const operationLabels: Record<WorkspaceFailure['operation'], string> = {
  requirements: 'Checking requirements', download: 'Getting workspace files', verify: 'Checking workspace files',
  prepare: 'Preparing workspace copies', start: 'Starting workspace', services: 'Starting workspace services', stop: 'Stopping workspace',
};
// Only native finite metadata selects a cause. Private diagnostics are never input.
export function workspaceRecovery(failure?: WorkspaceFailure): RecoveryInfo {
  const operation = failure?.operation ?? 'start';
  const cause = failure?.cause ?? 'unknown';
  const common = { stage: operationLabels[operation], category: cause,
    heading: operation === 'download' ? "Your workspace files couldn't be downloaded" : operation === 'prepare' || operation === 'verify' ? "Your workspace files couldn't be prepared" : "Your workspace couldn't start",
    preservation: operation === 'requirements' ? 'No workspace files were changed by this check.'
      : operation === 'download' ? 'Reusable verified content is kept. Rejected content is not kept.'
      : operation === 'prepare' || operation === 'verify' ? 'Bridge prepares local copies, not downloads. Existing workspace and agent files are kept.'
      : 'Your workspace and agent files are kept.',
  };
  switch (cause) {
    case 'permission': return { ...common, summary: operation === 'prepare' || operation === 'verify' ? 'The operating system denied preparing the workspace files.' : 'The operating system denied starting the workspace.', action: 'Check the permissions for Bridge and its workspace files, then press Try again.', topic: 'workspace' };
    case 'noSpace': return { ...common, summary: "There isn't enough free space for the workspace.", action: 'Free some space on this computer, then press Try again.', topic: 'workspace' };
    case 'integrity': return { ...common, summary: "The workspace files couldn't be verified.", action: 'Check that you have the matching workspace files for this Bridge build, then press Try again.', topic: 'workspace' };
    case 'inUse': return { ...common, summary: 'The workspace files are still in use or need checking.', action: 'Close any other Bridge window, then press Try again.', topic: 'workspace' };
    case 'network': return { ...common, summary: "The workspace files couldn't be downloaded.", action: 'Check your internet connection, then press Try again.' };
    case 'preparation': return { ...common, summary: "The workspace files couldn't be prepared.", action: 'Press Try again. If it happens again, copy the details for help.' };
    case 'guestConnection': return { ...common, summary: "The workspace didn't finish starting.", action: 'Press Try again to restart it. If it happens again, copy the details for help.' };
    case 'services': return { ...common, summary: "The workspace's services couldn't start.", action: 'Press Try again to restart the workspace. If it happens again, copy the details for help.' };
    case 'stopUnconfirmed': return { ...common, summary: "The workspace couldn't be stopped safely.", action: 'Quit Bridge and reopen it before trying again. Your workspace may still be stopping.', topic: 'workspace', retryBlocked: true };
    default: return { ...common, summary: "Bridge couldn't establish why the workspace failed.", action: 'Press Try again. If it happens again, copy the details for help.' };
  }
}

export function helpDetails(info: RecoveryInfo, codes: number[] = []): string {
  return ['Bridge workspace setup', `Operation: ${info.stage}`, `Category: ${info.category ?? 'requirements'}`,
    ...[...new Set(codes.filter(isOsCode))].slice(0, 4).map(code => `OS code: ${code}`), 'Private diagnostic text is not included.'].join('\n');
}

export function modelDetails(detail: string): string {
  const marker = detail.indexOf(' Guest Connect:');
  return marker >= 0 ? detail.slice(marker + 1) : detail;
}
