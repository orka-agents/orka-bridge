# Test result screens

Native macOS captures for PR #145 at `172e8b1784aa478b6bddc59f9365184068f7a6f6`, from [the successful native build and walks](https://github.com/orka-agents/orka-bridge/actions/runs/43345318709). Wide captures are 1280 pixels across (800 or 828 pixels high, depending on the native titlebar). Compact views use 540×900 content, captured with their titlebars. Click an image for its full size.

Light and dark run in separate bounded CI jobs, with the same assertions and 30-minute cap. Each verifies a fresh profile and retained choice after its own restart.

The local screens embed the real compiled frontend in the separate synthetic-IPC native fixture, without a VM or credentials. Its catalog is intentionally unavailable. The Send screens use the main desktop app and native IPC against loopback Kubernetes/Orka fixtures. Neither walk proves real inference, website fetching, or execution cancellation. Existing cluster Stop stops watching; no local Stop was added.

## Local results — native UI fixture

| State | Light | Dark |
| --- | --- | --- |
| Result and verdict, wide | ![](light/local-result-wide.png) | ![](dark/local-result-wide.png) |
| Question, answer and failure reason, wide | ![](light/local-answer-wide.png) | ![](dark/local-answer-wide.png) |
| Question, answer and failure reason, compact | ![](light/local-answer.png) | ![](dark/local-answer.png) |
| Saved edit makes the earlier result stale, compact | ![](light/local-stale.png) | ![](dark/local-stale.png) |
| Running stage and observed time, compact | ![](light/local-running.png) | ![](dark/local-running.png) |
| Running stage and observed time, wide | ![](light/local-running-wide.png) | ![](dark/local-running-wide.png) |

## Send results — main native app, loopback services

| State | Light | Dark |
| --- | --- | --- |
| Passing aggregate, wide | ![](light/send-result-wide.png) | ![](dark/send-result-wide.png) |
| Disclosed passing question and answer, wide | ![](light/send-answer-wide.png) | ![](dark/send-answer-wide.png) |
| Disclosed passing question and answer, compact | ![](light/send-answer.png) | ![](dark/send-answer.png) |
| External test-file change detected, before Reload | ![](light/send-disk-stale.png) | ![](dark/send-disk-stale.png) |
| Earlier result after Reload, wide | ![](light/send-stale-wide.png) | ![](dark/send-stale-wide.png) |
| Mixed result aggregate, wide | ![](light/send-mixed-wide.png) | ![](dark/send-mixed-wide.png) |
| Failure reason, wide | ![](light/send-failed-wide.png) | ![](dark/send-failed-wide.png) |
| Failure reason, compact | ![](light/send-failed.png) | ![](dark/send-failed.png) |
| Running stage, compact | ![](light/send-running.png) | ![](dark/send-running.png) |
| Running stage, wide | ![](light/send-running-wide.png) | ![](dark/send-running-wide.png) |
| Disclosed resource identity and Copy, compact | ![](light/send-result-details.png) | ![](dark/send-result-details.png) |
| Existing Stop behavior: stopped watching | ![](light/send-stopped.png) | ![](dark/send-stopped.png) |
| Saved summaries after restart | ![](light/send-history-restarted.png) | ![](dark/send-history-restarted.png) |
| Earlier passing summary after restart | ![](light/send-history-passed.png) | ![](dark/send-history-passed.png) |
