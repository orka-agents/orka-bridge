# Native setup and first-test screenshots

Bridge revision [`2d59a7be6f88956cbe6c401e4fcdb89cf6929c4b`](https://github.com/orka-agents/orka-bridge/commit/2d59a7be6f88956cbe6c401e4fcdb89cf6929c4b), captured by [App CI](https://github.com/orka-agents/orka-bridge/actions/runs/43432018721). All three native artifact head files match that full revision.

These are **synthetic IPC fixtures rendered by the native apps**, not a browser dry run and not proof of real sign-in, model inference, VM or cluster execution. Click any image for full size.

Windows compact captures resized the actual native window: WebView/client **540×900 CSS px**, devicePixelRatio 1. Mac compact content is 540×900; its screenshots also include the native title bar. Wide captures retain their native viewport. Layout assertions check no horizontal overflow, top-aligned wide columns, compact test-first order, and Send after the editor. Change asserts Keep is the only rendered primary.

<details>
<summary>Windows — wide, light and dark (18 captures)</summary>

| State | Light | Dark |
|---|---|---|
| Place chooser | ![Place chooser, light](windows/light-wide-chooser.png) | ![Place chooser, dark](windows/dark-wide-chooser.png) |
| AI prerequisite / ready workspace | ![AI prerequisite / ready workspace, light](windows/light-wide-ai-idle.png) | ![AI prerequisite / ready workspace, dark](windows/dark-wide-ai-idle.png) |
| AI connecting | ![AI connecting, light](windows/light-wide-ai-connecting.png) | ![AI connecting, dark](windows/dark-wide-ai-connecting.png) |
| Connected / first test complete | ![Connected / first test complete, light](windows/light-wide-ai-connected.png) | ![Connected / first test complete, dark](windows/dark-wide-ai-connected.png) |
| Automatic first test running | ![Automatic first test running, light](windows/light-wide-first-test-running.png) | ![Automatic first test running, dark](windows/dark-wide-first-test-running.png) |
| Automatic result in Designer | ![Automatic result in Designer, light](windows/light-wide-first-test-result.png) | ![Automatic result in Designer, dark](windows/dark-wide-first-test-result.png) |
| Change confirmation at its invoker | ![Change confirmation at its invoker, light](windows/light-wide-change.png) | ![Change confirmation at its invoker, dark](windows/dark-wide-change.png) |
| Designer before a local test | ![Designer before a local test, light](windows/light-wide-designer.png) | ![Designer before a local test, dark](windows/dark-wide-designer.png) |
| Install progress and Meet | ![Install progress and Meet, light](windows/light-wide-setup-transfer.png) | ![Install progress and Meet, dark](windows/dark-wide-setup-transfer.png) |

</details>

<details>
<summary>Windows — compact, light and dark (22 captures)</summary>

| State | Light | Dark |
|---|---|---|
| Place chooser | ![Place chooser, light](windows/light-compact-chooser.png) | ![Place chooser, dark](windows/dark-compact-chooser.png) |
| AI prerequisite / ready workspace | ![AI prerequisite / ready workspace, light](windows/light-compact-ai-idle.png) | ![AI prerequisite / ready workspace, dark](windows/dark-compact-ai-idle.png) |
| AI connecting | ![AI connecting, light](windows/light-compact-ai-connecting.png) | ![AI connecting, dark](windows/dark-compact-ai-connecting.png) |
| Connected / first test complete | ![Connected / first test complete, light](windows/light-compact-ai-connected.png) | ![Connected / first test complete, dark](windows/dark-compact-ai-connected.png) |
| Automatic first test running | ![Automatic first test running, light](windows/light-compact-first-test-running.png) | ![Automatic first test running, dark](windows/dark-compact-first-test-running.png) |
| Automatic result in Designer | ![Automatic result in Designer, light](windows/light-compact-first-test-result.png) | ![Automatic result in Designer, dark](windows/dark-compact-first-test-result.png) |
| Change confirmation at its invoker | ![Change confirmation at its invoker, light](windows/light-compact-change.png) | ![Change confirmation at its invoker, dark](windows/dark-compact-change.png) |
| Designer before a local test | ![Designer before a local test, light](windows/light-compact-designer.png) | ![Designer before a local test, dark](windows/dark-compact-designer.png) |
| Install progress and Meet | ![Install progress and Meet, light](windows/light-compact-setup-transfer.png) | ![Install progress and Meet, dark](windows/dark-compact-setup-transfer.png) |
| Focused Change confirmation | ![Focused Change confirmation, light](windows/light-compact-change-confirmation.png) | ![Focused Change confirmation, dark](windows/dark-compact-change-confirmation.png) |
| Meet / end of install screen | ![Meet / end of install screen, light](windows/light-compact-setup-transfer-end.png) | ![Meet / end of install screen, dark](windows/dark-compact-setup-transfer-end.png) |

</details>

<details>
<summary>Mac — connection, removal and native editing/result walk (20 captures)</summary>

| State | Light | Dark |
|---|---|---|
| Before Connect | ![Before Connect, light](mac/light-before-connect.png) | ![Before Connect, dark](mac/dark-before-connect.png) |
| Connected | ![Connected, light](mac/light-connected.png) | ![Connected, dark](mac/dark-connected.png) |
| Remove failure / recovery | ![Remove failure / recovery, light](mac/light-remove-failed.png) | ![Remove failure / recovery, dark](mac/dark-remove-failed.png) |
| Removed | ![Removed, light](mac/light-removed.png) | ![Removed, dark](mac/dark-removed.png) |
| Reconnected | ![Reconnected, light](mac/light-reconnected.png) | ![Reconnected, dark](mac/dark-reconnected.png) |
| Result — wide | ![Result — wide, light](mac/light-local-result-wide.png) | ![Result — wide, dark](mac/dark-local-result-wide.png) |
| Result — compact | ![Result — compact, light](mac/light-local-result.png) | ![Result — compact, dark](mac/dark-local-result.png) |
| Saved native edit — stale result | ![Saved native edit — stale result, light](mac/light-local-stale.png) | ![Saved native edit — stale result, dark](mac/dark-local-stale.png) |
| Running after edit — compact | ![Running after edit — compact, light](mac/light-local-running.png) | ![Running after edit — compact, dark](mac/dark-local-running.png) |
| Running after edit — wide | ![Running after edit — wide, light](mac/light-local-running-wide.png) | ![Running after edit — wide, dark](mac/dark-local-running-wide.png) |

</details>
