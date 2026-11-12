import { expect, test } from 'vitest';
import { clusterChecks, clusterResults } from './api';
import { clusterDetails, clusterRecovery } from './recovery';

test.each(clusterChecks)('unknown at %s admits uncertainty without inventing a cause', check => {
  const recovery = clusterRecovery({ check, result: 'unknown' });
  expect(recovery.cause).toMatch(check === 'configuration' ? /kubeconfig could not be read/ : /cause could not be established/);
  expect(recovery.cause).not.toMatch(/was denied|was rejected|VPN/);
  expect(recovery.next.length).toBeGreaterThan(10);
});
test.each(clusterResults)('finite %s result produces only fixed copied labels', result => {
  const failure = { check: 'providers' as const, result };
  expect(clusterDetails(failure, 'Use cluster')).toBe(`Bridge cluster connection\nOperation: Use cluster\nCheck: providers\nResult: ${result}`);
  expect(clusterRecovery(failure).next.length).toBeGreaterThan(10);
});
test.each([
  ['authenticationRejected', /sign-in was rejected/, /Refresh the sign-in/],
  ['permissionDenied', /Access was denied/, /administrator/],
  ['identityRejected', /identity or namespace/, /identity and watched namespace/],
  ['connectionFailed', /connection could not be opened/, /connectivity and any VPN/],
] as const)('Orka identity %s has evidence-based cause and next step', (result, cause, next) => {
  const recovery = clusterRecovery({ check: 'orkaIdentity', result });
  expect(recovery.cause).toMatch(cause); expect(recovery.next).toMatch(next);
});
test.each([
  { check: 'clusterApi', result: 'authenticationRejected' },
  { check: 'credentials', result: 'unsupportedCredential' },
] as const)('kubeconfig-changing recovery for $result explains reload before retry', failure => {
  expect(clusterRecovery(failure).next).toContain('Reload kubeconfig');
});
test('no Providers is distinct from an unknown Provider read', () => {
  expect(clusterRecovery({ check: 'providers', result: 'noProviders' }).cause).toContain('No Providers are configured');
  expect(clusterRecovery({ check: 'providers', result: 'noProviders' }).next).toContain('configure a Provider');
});
