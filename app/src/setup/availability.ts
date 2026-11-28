import type { SetupStatus } from './state';

// A build without the local cluster (a Linux build, for one) reports this computer as not integrated. Nothing can
// start there, so every page names the cluster route instead of waiting for, stopping or starting a workspace.
export const localUnavailable = (status: SetupStatus | undefined) => status?.localVm === 'notIntegrated';

export const unavailableCopy = {
  line: 'Not available on this build',
  // Said wherever This computer would otherwise wait, be stopped or start.
  detail: "This computer can't run agents on this build. Use a cluster you already have.",
  // Home's card where the Navigator would wait for this computer.
  title: 'Not available on this build.',
  // Under Welcome's Get started, which leads to Clusters.
  lede: "Build an agent on this computer, then test it and run it on your team's Orka cluster.",
  welcome: "This build can't run agents here. Next, you connect a cluster you already have.",
  // Why the choice can't be made, beside it.
  reason: 'Workspace setup is not included in this build. Nothing will be downloaded or started.',
};
