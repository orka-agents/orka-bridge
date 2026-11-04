# Bridge

**Write an agent on your laptop, watch it use a real [Orka](https://github.com/orka-agents/orka) tool and pass its
tests, then send the same file to your team's cluster.**

Writing an agent for Orka today starts with a cluster. Before the first prompt runs, someone has to create one,
install Orka, connect a model and learn the YAML. Most people who want to write an agent do not want to do any of
that. Bridge is a desktop app that does it for them.

This is the hour it is built for. None of it runs yet; the repo holds this plan, and the app arrives as pull
requests, one at a time. You open Bridge. It starts an Orka on your laptop without asking you to install anything.
An example agent is already open, with a prompt, the `web_search` tool ticked and one test the agent can only pass by
searching. You click Run. The test runs on your local Orka and passes, with the tool call shown under it. You change
the prompt, add a test of your own, run again. When it is right, you pick your team's cluster and click Send. The
same file and the same tests run there, and the agent is deployed. The local Orka is a convenience; the tool, the
file and the tests are the ones the real cluster sees.

## The first hour

The app has three tabs: Overview, Designer and Clusters. Here they are in the order you meet them.

**Setup** is what Clusters shows on first run. The app downloads and starts a virtual machine it ships with, which
has an Orka already running inside. It asks where your model comes from, then runs one test task to prove the path
works. One screen with a progress list; the only choice is the model. If something is missing, the screen says what
and waits.

**Designer** is where Setup ends, with the example agent open. It has Prompt, Tools, Tests and Chat. Tools lists
Orka's built-in tools and the Tool resources on the cluster you are targeting; a Tool resource is Orka's object for a
tool you define yourself, such as an HTTP endpoint. Run executes the tests as real tasks on your local Orka and shows
pass or fail, the tool calls, time and tokens. Chat is a session with the same agent. The agent is a Markdown file in
a folder the app makes a git repository, or in a repository of yours.

**Clusters** is where you send the agent: any kubectl context that has Orka on it, or a new AKS cluster the app
creates for you. Send first checks the destination against the file: Orka version, the model, and each tool the agent
uses, present or missing. Then it runs the same tests there, publishes the agent as a package in a container registry
and deploys it. After that, one card per agent per place: where it lives, whether it was tested there, what it can
reach, its last runs. **Overview** is those cards for every agent you can reach, local and on each cluster.

## How it is built

- **A desktop app** in Tauri 2, for Windows, Mac and Linux. The app owns the files, the setup and the screens.
- **kmx as the engine.** [kmx](https://github.com/kaimahi-agents/kaimahi) already creates a local cluster with Orka
  on it (`kmx cluster up`), runs an agent's tests on Orka (`kmx package test`), and publishes and deploys
  (`kmx package deploy`). Bridge will run kmx as a sidecar and read its JSON output instead of talking to Orka's API
  or running cluster tools itself.
- **A VM the app ships with.** The plan is an image that carries a container runtime,
  [kind](https://kind.sigs.k8s.io/), an Orka cluster already created, and kmx, so nothing is installed on the machine
  first. On Mac it would run on Apple's Virtualization framework (Apple silicon), on Windows as its own WSL2 distro,
  on Linux under QEMU with KVM. The image and its build pipeline do not exist yet; see the issues.
- **Your model.** The plan is to use your existing GitHub Copilot sign-in through a model gateway inside the VM. A
  pasted key for OpenAI, Anthropic or Azure OpenAI is the other choice on the same screen.

Two public projects shaped this: Hermes Agent, for an install that ends in a working agent with no terminal, and
Try Omarchy, for shipping a whole Linux VM inside a Mac app.

## What comes after

Everything past the first hour is an issue, one each, written so a person can pick it up and own it: running an agent
on a schedule, a Messaging panel for a bot in Teams or Telegram, skills in the designer, a list of sessions across
clusters, MCP tools in the designer, the VM image build and its update path, a shared team cluster, and the second
kind of developer, one who points a coding agent at a repository. See the issues labelled
[help wanted](https://github.com/orka-agents/orka-bridge/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22).
The first-hour screens themselves land as pull requests from the maintainers, in the order above; if you want to help
with one of those, say so on an issue first so two people do not build the same screen.

## Develop

Build and run instructions arrive with the app shell.

## Contributing

This project welcomes contributions and suggestions.  Most contributions require you to agree to a
Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us
the rights to use your contribution. For details, visit [Contributor License Agreements](https://cla.opensource.microsoft.com).

When you submit a pull request, a CLA bot will automatically determine whether you need to provide
a CLA and decorate the PR appropriately (e.g., status check, comment). Simply follow the instructions
provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/).
For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or
contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.

## Trademarks

This project may contain trademarks or logos for projects, products, or services. Authorized use of Microsoft
trademarks or logos is subject to and must follow
[Microsoft's Trademark & Brand Guidelines](https://www.microsoft.com/legal/intellectualproperty/trademarks/usage/general).
Use of Microsoft trademarks or logos in modified versions of this project must not cause confusion or imply Microsoft sponsorship.
Any use of third-party trademarks or logos are subject to those third-party's policies.
