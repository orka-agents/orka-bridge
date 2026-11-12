import type { ClusterFailure } from './api';

const checks: Record<ClusterFailure['check'], string> = {
  configuration: 'Kubeconfig reading', credentials: 'Credential resolution', clusterApi: 'Cluster API reachability',
  compatibility: 'Orka compatibility and watch namespace', serviceAccount: 'ServiceAccount token issuance',
  permissions: 'Cluster permission review', orkaIdentity: 'Orka identity and namespace', orkaConnection: 'Orka connection',
  agents: 'Agent reading', providers: 'Provider reading',
};
const results: Record<ClusterFailure['result'], string> = {
  unknown: 'The cause could not be established.', connectionFailed: 'The connection could not be opened.',
  authenticationRejected: 'The sign-in was rejected.', permissionDenied: 'Access was denied.',
  unsupportedCredential: 'This credential method could not be used.', incompatible: 'The Orka schema or watch namespace is incompatible.',
  identityRejected: 'Orka rejected the identity or namespace.', noProviders: 'No Providers are configured in this Orka.',
};
export function clusterRecovery(failure: ClusterFailure) {
  const { check, result } = failure;
  const next = check === 'configuration' ? 'Correct your kubeconfig, then press Reload kubeconfig.'
    : check === 'credentials' || result === 'unsupportedCredential' ? 'Check its sign-in method and credential plugin outside Bridge, then check again.'
    : result === 'connectionFailed' ? 'Check connectivity and any VPN you use, then check again.'
    : result === 'authenticationRejected' ? 'Refresh the sign-in outside Bridge, then press Reload kubeconfig, select the context and use the cluster again.'
    : result === 'permissionDenied' ? 'Ask your cluster administrator to check access for this check, then check again.'
    : check === 'compatibility' ? 'Ask your cluster administrator to check the Orka schema and watched namespace, then check again.'
    : check === 'serviceAccount' ? 'Check the existing ServiceAccount name and token-request access with your cluster administrator, then check again.'
    : check === 'orkaIdentity' ? 'Ask your cluster administrator to check this Orka identity and watched namespace, then check again.'
    : check === 'providers' && result === 'noProviders' ? 'Ask your cluster administrator to configure a Provider, then check again.'
    : 'Check again. If it still fails, copy the details for help.';
  return { cause: check === 'configuration' ? 'Your kubeconfig could not be read.' : `${checks[check]} failed. ${results[result]}`, next };
}
export function clusterDetails(failure: ClusterFailure, operation: string) {
  return ['Bridge cluster connection', `Operation: ${operation}`, `Check: ${failure.check}`, `Result: ${failure.result}`].join('\n');
}
