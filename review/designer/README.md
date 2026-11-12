# Designer: native desktop captures

Product revision: `370625f57ed53730ea0cacd0941a39c9d802c1c6`.
Source: [macOS bundle job, App run 43277281518](https://github.com/orka-agents/orka-bridge/actions/runs/43277281518).
These are native macOS app windows, not browser previews. Desktop captures are 1280 px wide; compact captures are 540 px wide.

The walk opens the owned sample file before workspace readiness, edits it through Agent and File, verifies the exact saved bytes and Saved acknowledgement, opens More settings, and switches views at minimum width. UUID markers in the instructions are test edits, not product copy. The debug lifecycle walk separately checks retained edits through close/reopen and second launches in both themes.

| State | Light | Dark |
| --- | --- | --- |
| Agent: instructions, tools and model guidance | [![Agent, light](light-agent-view.png)](light-agent-view.png) | [![Agent, dark](dark-agent-view.png)](dark-agent-view.png) |
| File: path controls, syntax colouring and line numbers | [![File, light](light-file-view.png)](light-file-view.png) | [![File, dark](dark-file-view.png)](dark-file-view.png) |
| File: focused editor after a setup-time save | [![Saved file, light](light-file-edited.png)](light-file-edited.png) | [![Saved file, dark](dark-file-edited.png)](dark-file-edited.png) |
| More settings: Description, Personality and file location | [![Settings, light](light-designer-settings.png)](light-designer-settings.png) | [![Settings, dark](dark-designer-settings.png)](dark-designer-settings.png) |
| Compact Agent | [![Compact Agent, light](light-designer-compact.png)](light-designer-compact.png) | [![Compact Agent, dark](dark-designer-compact.png)](dark-designer-compact.png) |
| Compact File: wrapped lines without horizontal overflow | [![Compact File, light](light-file-compact.png)](light-file-compact.png) | [![Compact File, dark](dark-file-compact.png)](dark-file-compact.png) |

Limits: these captures do not prove a running VM, a real provider or cluster, a live catalog, native clipboard transfer, or Finder/Explorer navigation. The walk uses no real credentials and submits no workload.
