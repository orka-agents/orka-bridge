use bridge_runtime_protocol::*;

const MODEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
pub enum ModelCommand {
    Connect {
        generation: Uuid,
        selection: ModelSelection,
    },
    Disconnect {
        connection: ModelConnection,
    },
}
pub struct Outcome {
    pub connection: Option<ModelConnection>,
    pub success: bool,
}
pub trait Runner {
    fn execute(&mut self, command: &ModelCommand) -> Outcome;
}
pub struct Kmx {
    installation: Uuid,
}
impl Kmx {
    pub fn new(installation: Uuid) -> Self {
        Self { installation }
    }
    fn command(&self, command: &ModelCommand) -> std::process::Command {
        use std::{
            os::unix::process::CommandExt,
            process::{Command, Stdio},
        };
        let cluster = format!("bridge-{}", self.installation.simple());
        let mut child = Command::new("/usr/local/bin/kmx");
        child
            .args(argv(command, &cluster))
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
            .env("HOME", "/root")
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C")
            .env("KUBECONFIG", "/var/lib/bridge/kubeconfig")
            .env("KMX_HOME", "/var/lib/bridge/kmx")
            .env("KIND_CLUSTER", cluster)
            .env("CONTAINER_ENGINE", "docker")
            .env("KIND_EXPERIMENTAL_PROVIDER", "docker")
            .env("DOCKER_HOST", "unix:///var/run/docker.sock")
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        child
    }
}
pub struct ModelLifecycle<R = Kmx> {
    runner: R,
    phase: ModelPhase,
    connection: Option<ModelConnection>,
}
impl Runner for Kmx {
    fn execute(&mut self, command: &ModelCommand) -> Outcome {
        let run = || {
            let child = self.command(command).spawn()?;
            crate::engine::collect(child, MODEL_TIMEOUT)
        };
        match run() {
            Ok(output) => parse_reply(command, output.exit, &output.stdout),
            Err(_) => Outcome {
                connection: None,
                success: false,
            },
        }
    }
}
fn argv(command: &ModelCommand, cluster: &str) -> Vec<String> {
    let mut args: Vec<String> = ["--container-engine=docker", "cluster", "orka", "model"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    args.insert(0, format!("--context=kind-{cluster}"));
    match command {
        ModelCommand::Connect {
            generation,
            selection,
        } => args.extend([
            "connect".into(),
            "--namespace=orka-system".into(),
            "--route=copilot".into(),
            format!("--provider={}", selection.provider),
            format!("--model={}", selection.model),
            format!("--credential-generation={generation}"),
        ]),
        ModelCommand::Disconnect { connection } => args.extend([
            "disconnect".into(),
            "--namespace=orka-system".into(),
            format!("--provider={}", connection.selection.provider),
            format!("--connection={}", connection.id),
        ]),
    }
    args.push("-o=jsonl".into());
    args
}
impl<R: Runner> ModelLifecycle<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            phase: ModelPhase::Idle,
            connection: None,
        }
    }
    pub fn allows_credential_mutation(&self) -> bool {
        self.phase == ModelPhase::Idle
    }
    // The serving loop supplies actual store status and guards physical-root dispatch.
    pub fn handle(
        &mut self,
        credential: CredentialState,
        request: &RequestBody,
    ) -> Option<ResultBody> {
        match request {
            RequestBody::ModelStatus {} => {}
            RequestBody::ModelConnect {
                generation,
                selection,
            } => {
                if self.phase == ModelPhase::Idle
                    && credential.installed
                    && credential.generation == Some(*generation)
                    && selection.valid()
                {
                    // An attempted external mutation is not implicitly safe to retry.
                    self.phase = ModelPhase::Unknown;
                    let result = self.runner.execute(&ModelCommand::Connect {
                        generation: *generation,
                        selection: selection.clone(),
                    });
                    self.connection = result.connection;
                    if result.success && self.connection.is_some() {
                        self.phase = ModelPhase::Ready;
                    }
                }
            }
            RequestBody::ModelDisconnect { connection } => {
                if self.phase != ModelPhase::Idle {
                    if let Some(known) = self.connection.clone().filter(|c| c.id == *connection) {
                        self.phase = ModelPhase::Unknown;
                        let result = self.runner.execute(&ModelCommand::Disconnect {
                            connection: known.clone(),
                        });
                        if result.success && result.connection == Some(known) {
                            self.phase = ModelPhase::Idle;
                            self.connection = None;
                        }
                    }
                }
            }
            _ => return None,
        }
        Some(ResultBody::ModelState(ModelState {
            phase: self.phase,
            connection: self.connection.clone(),
            credential,
        }))
    }
}

fn canonical_uuid(value: &serde_json::Value) -> Option<Uuid> {
    let text = value.as_str()?;
    let id = Uuid::parse_str(text).ok()?;
    (id.to_string() == text && id.get_version_num() == 4 && id.as_bytes()[8] & 0xc0 == 0x80)
        .then_some(id)
}
fn parse_reply(command: &ModelCommand, exit: i32, bytes: &[u8]) -> Outcome {
    use serde_json::Value;
    let (operation, expected, selection, generation) = match command {
        ModelCommand::Connect {
            generation,
            selection,
        } => (
            "cluster.orka.model.connect",
            "model_ready",
            selection,
            *generation,
        ),
        ModelCommand::Disconnect { connection } => (
            "cluster.orka.model.disconnect",
            "gateway_stopped",
            &connection.selection,
            connection.generation,
        ),
    };
    let mut out = Outcome {
        connection: None,
        success: false,
    };
    let valid = (|| {
        if bytes.len() > MAX_FILE {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let (mut invocation, mut terminal, mut success) = (None, false, false);
        for (index, line) in text.lines().enumerate() {
            let row: Value = serde_json::from_str(line).ok()?;
            let id = canonical_uuid(&row["operationId"])?;
            if terminal
                || row["schemaVersion"] != 1
                || row["operation"] != operation
                || row["seq"].as_u64() != Some(index as u64 + 1)
                || *invocation.get_or_insert(id) != id
            {
                return None;
            }
            match row["kind"].as_str()? {
                "kmx.event" => {}
                "kmx.result" => {
                    terminal = true;
                    if !row["connection"].is_null() {
                        let c = &row["connection"];
                        let id = canonical_uuid(&c["id"])?;
                        if c["route"] != "copilot"
                            || c["provider"] != selection.provider
                            || c["model"] != selection.model
                            || c["credentialGeneration"] != generation.to_string()
                        {
                            return None;
                        }
                        out.connection = Some(ModelConnection {
                            id,
                            selection: selection.clone(),
                            generation,
                        });
                        if matches!(command, ModelCommand::Disconnect { connection } if connection.id != id)
                        {
                            return None;
                        }
                    }
                    let outcome = row["outcome"].as_str()?;
                    if !matches!(outcome, "complete" | "failed" | "cancelled") {
                        return None;
                    }
                    success = exit == 0
                        && row["exitCode"].as_i64() == Some(0)
                        && outcome == "complete"
                        && out.connection.is_some()
                        && row["connection"]["state"] == expected;
                }
                _ => return None,
            }
        }
        Some(terminal && success)
    })();
    out.success = valid.unwrap_or(false);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        calls: Vec<ModelCommand>,
        outcomes: VecDeque<Outcome>,
    }
    impl Runner for Fake {
        fn execute(&mut self, command: &ModelCommand) -> Outcome {
            self.calls.push(command.clone());
            self.outcomes
                .pop_front()
                .expect("unexpected external command")
        }
    }
    fn id(n: u8) -> Uuid {
        Uuid::parse_str(&format!("00000000-0000-4000-8000-{n:012}")).unwrap()
    }
    fn selection() -> ModelSelection {
        ModelSelection {
            provider: "copilot".into(),
            model: "gpt-4.1".into(),
        }
    }
    fn credential() -> CredentialState {
        CredentialState {
            profile: CREDENTIAL_PROFILE,
            installed: true,
            generation: Some(id(1)),
        }
    }
    fn connection() -> ModelConnection {
        ModelConnection {
            id: id(2),
            selection: selection(),
            generation: id(1),
        }
    }
    fn connect() -> RequestBody {
        RequestBody::ModelConnect {
            generation: id(1),
            selection: selection(),
        }
    }
    fn lifecycle(outcomes: Vec<Outcome>) -> ModelLifecycle<Fake> {
        ModelLifecycle::new(Fake {
            calls: vec![],
            outcomes: outcomes.into(),
        })
    }
    fn state(m: &mut ModelLifecycle<Fake>, request: &RequestBody) -> ModelState {
        match m.handle(credential(), request).unwrap() {
            ResultBody::ModelState(s) => s,
            _ => panic!("expected model state"),
        }
    }
    // fc811: machine_output.go envelope + machine_setup.go flattened OrkaModelResult.
    const READY: &str = r#"{"schemaVersion":1,"kind":"kmx.event","operation":"cluster.orka.model.connect","operationId":"00000000-0000-4000-8000-000000000004","seq":1,"elapsedMs":0,"event":"stage.started","stage":"gateway"}
{"schemaVersion":1,"kind":"kmx.result","operation":"cluster.orka.model.connect","operationId":"00000000-0000-4000-8000-000000000004","seq":2,"elapsedMs":2,"outcome":"complete","verdict":"not_applicable","exitCode":0,"mode":"apply","effects":[],"connection":{"id":"00000000-0000-4000-8000-000000000002","route":"copilot","provider":"copilot","model":"gpt-4.1","credentialGeneration":"00000000-0000-4000-8000-000000000001","state":"model_ready"}}
"#;
    fn command() -> ModelCommand {
        ModelCommand::Connect {
            generation: id(1),
            selection: selection(),
        }
    }
    #[test]
    fn wrong_generation_absent_credentials_and_invalid_selection_never_run() {
        let cases = [
            (
                credential(),
                RequestBody::ModelConnect {
                    generation: id(3),
                    selection: selection(),
                },
            ),
            (
                CredentialState {
                    installed: false,
                    ..credential()
                },
                connect(),
            ),
            (
                credential(),
                RequestBody::ModelConnect {
                    generation: id(1),
                    selection: ModelSelection {
                        provider: "".into(),
                        model: "x".into(),
                    },
                },
            ),
            (
                credential(),
                RequestBody::ModelConnect {
                    generation: id(1),
                    selection: ModelSelection {
                        provider: "x".into(),
                        model: "x\0".into(),
                    },
                },
            ),
            (
                credential(),
                RequestBody::ModelConnect {
                    generation: id(1),
                    selection: ModelSelection {
                        provider: "x".into(),
                        model: "x".repeat(4097),
                    },
                },
            ),
        ];
        for (credential, request) in cases {
            let mut m = lifecycle(vec![]);
            let ResultBody::ModelState(s) = m.handle(credential.clone(), &request).unwrap() else {
                panic!()
            };
            assert_eq!(s.phase, ModelPhase::Idle);
            assert_eq!(s.credential, credential);
            assert!(m.runner.calls.is_empty());
        }
    }
    #[test]
    fn unknown_without_an_id_cannot_guess_cleanup_or_retry() {
        let mut m = lifecycle(vec![Outcome {
            connection: None,
            success: false,
        }]);
        assert_eq!(state(&mut m, &connect()).phase, ModelPhase::Unknown);
        state(&mut m, &connect());
        state(&mut m, &RequestBody::ModelDisconnect { connection: id(2) });
        assert!(!m.allows_credential_mutation());
        assert_eq!(m.runner.calls.len(), 1);
    }
    #[test]
    fn failed_exit_keeps_the_validated_id_without_becoming_ready() {
        for (outcome, exit) in [("failed", 1), ("cancelled", 1), ("complete", -1)] {
            let reply = READY
                .replace(
                    r#""outcome":"complete""#,
                    &format!(r#""outcome":"{outcome}""#),
                )
                .replace(r#""exitCode":0"#, r#""exitCode":1"#)
                .replace("model_ready", "unknown");
            let parsed = parse_reply(&command(), exit, reply.as_bytes());
            assert!(!parsed.success);
            assert_eq!(parsed.connection, Some(connection()));
            let mut m = lifecycle(vec![parsed]);
            assert_eq!(state(&mut m, &connect()).connection, Some(connection()));
            assert_eq!(state(&mut m, &connect()).phase, ModelPhase::Unknown);
            assert!(!m.allows_credential_mutation());
            assert_eq!(m.runner.calls.len(), 1);
        }
    }
    #[test]
    fn malformed_or_cross_selection_replies_never_become_ready() {
        let replies = vec![
            String::new(),
            "not json".into(),
            READY.lines().next().unwrap().into(),
            format!("{READY}{{"),
            format!("{READY}{READY}"),
            READY.replace(r#""schemaVersion":1"#, r#""schemaVersion":2"#),
            READY.replace(
                "cluster.orka.model.connect",
                "cluster.orka.model.disconnect",
            ),
            READY.replace(r#""seq":2"#, r#""seq":3"#),
            READY.replacen("000000000004", "000000000005", 1),
            READY.replace("0000-4000-8000", "0000-1000-8000"),
            READY.replace(r#""kind":"kmx.result""#, r#""kind":"other""#),
            READY.replace(r#""route":"copilot""#, r#""route":"other""#),
            READY.replace(r#""provider":"copilot""#, r#""provider":"other""#),
            READY.replace("gpt-4.1", "other"),
            READY.replace("000000000001", "000000000003"),
            READY.replace("model_ready", "gateway_stopped"),
            READY.replace(r#""exitCode":0"#, r#""exitCode":1"#),
            READY.replace(r#""outcome":"complete""#, r#""outcome":"failed""#),
        ];
        for reply in replies {
            let parsed = parse_reply(&command(), 0, reply.as_bytes());
            assert!(!parsed.success);
            let mut m = lifecycle(vec![parsed]);
            assert_eq!(state(&mut m, &connect()).phase, ModelPhase::Unknown);
            state(&mut m, &connect());
            assert_eq!(m.runner.calls.len(), 1);
        }
        // A valid observed ID survives a later malformed row, but never grants readiness.
        let trailing = parse_reply(&command(), 0, format!("{READY}{{").as_bytes());
        assert_eq!(trailing.connection, Some(connection()));
        assert!(!trailing.success);
        let oversized = vec![b' '; MAX_FILE + 1];
        assert!(!parse_reply(&command(), 0, &oversized).success);
        assert!(!parse_reply(&command(), 0, &[0xff]).success);
    }
    #[test]
    fn production_command_targets_the_installation_cluster_and_restricted_guest_env() {
        let installation = Uuid::parse_str("12345678-1234-4abc-8def-123456789abc").unwrap();
        let kmx = Kmx::new(installation);
        for action in [
            command(),
            ModelCommand::Disconnect {
                connection: connection(),
            },
        ] {
            let command = kmx.command(&action);
            assert_eq!(command.get_program(), "/usr/local/bin/kmx");
            assert_eq!(command.get_current_dir(), Some(std::path::Path::new("/")));
            assert_eq!(
                command.get_args().next().unwrap(),
                "--context=kind-bridge-1234567812344abc8def123456789abc"
            );
            let env: std::collections::BTreeMap<_, _> = command
                .get_envs()
                .map(|(key, value)| (key.to_str().unwrap(), value.unwrap().to_str().unwrap()))
                .collect();
            assert_eq!(
                env,
                std::collections::BTreeMap::from([
                    ("PATH", "/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"),
                    ("HOME", "/root"),
                    ("LANG", "C.UTF-8"),
                    ("LC_ALL", "C"),
                    ("KUBECONFIG", "/var/lib/bridge/kubeconfig"),
                    ("KMX_HOME", "/var/lib/bridge/kmx"),
                    ("KIND_CLUSTER", "bridge-1234567812344abc8def123456789abc"),
                    ("CONTAINER_ENGINE", "docker"),
                    ("KIND_EXPERIMENTAL_PROVIDER", "docker"),
                    ("DOCKER_HOST", "unix:///var/run/docker.sock"),
                ])
            );
        }
    }
    #[test]
    fn argv_is_fixed_and_values_are_single_arguments_not_options() {
        let mut selection = selection();
        selection.model = "--odd model/with:punctuation".into();
        assert!(selection.valid());
        let kmx = Kmx::new(Uuid::parse_str("12345678-1234-4abc-8def-123456789abc").unwrap());
        let cmd = kmx.command(&ModelCommand::Connect {
            generation: id(1),
            selection,
        });
        let args: Vec<_> = cmd.get_args().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "--context=kind-bridge-1234567812344abc8def123456789abc",
                "--container-engine=docker",
                "cluster",
                "orka",
                "model",
                "connect",
                "--namespace=orka-system",
                "--route=copilot",
                "--provider=copilot",
                "--model=--odd model/with:punctuation",
                "--credential-generation=00000000-0000-4000-8000-000000000001",
                "-o=jsonl",
            ]
        );
        assert_eq!(
            kmx.command(&ModelCommand::Disconnect {
                connection: connection()
            })
            .get_args()
            .map(|s| s.to_str().unwrap())
            .collect::<Vec<_>>(),
            [
                "--context=kind-bridge-1234567812344abc8def123456789abc",
                "--container-engine=docker",
                "cluster",
                "orka",
                "model",
                "disconnect",
                "--namespace=orka-system",
                "--provider=copilot",
                "--connection=00000000-0000-4000-8000-000000000002",
                "-o=jsonl",
            ]
        );
    }
    #[test]
    fn ready_disconnect_keeps_credentials_installed_and_idle_disconnect_is_a_noop() {
        let mut m = lifecycle(vec![
            Outcome {
                connection: Some(connection()),
                success: true,
            },
            Outcome {
                connection: Some(connection()),
                success: true,
            },
        ]);
        state(&mut m, &RequestBody::ModelDisconnect { connection: id(2) });
        assert!(m.runner.calls.is_empty());
        state(&mut m, &connect());
        let s = state(&mut m, &RequestBody::ModelDisconnect { connection: id(2) });
        assert_eq!(s.phase, ModelPhase::Idle);
        assert!(s.connection.is_none());
        assert_eq!(s.credential, credential());
        assert!(m.allows_credential_mutation());
        assert_eq!(m.runner.calls.len(), 2);
        assert!(m
            .handle(credential(), &RequestBody::CredentialStatus {})
            .is_none());
    }
    #[test]
    fn disconnect_requires_the_actual_stopped_id() {
        let stopped = READY
            .replace(
                "cluster.orka.model.connect",
                "cluster.orka.model.disconnect",
            )
            .replace("model_ready", "gateway_stopped");
        let command = ModelCommand::Disconnect {
            connection: connection(),
        };
        assert!(parse_reply(&command, 0, stopped.as_bytes()).success);
        let wrong_id = stopped.replace("000000000002", "000000000003");
        assert!(!parse_reply(&command, 0, wrong_id.as_bytes()).success);
        assert!(!parse_reply(&command, 0, READY.as_bytes()).success);
    }
    #[test]
    fn actual_flat_jsonl_projection_keeps_only_native_connection() {
        let out = parse_reply(&command(), 0, READY.as_bytes());
        assert!(out.success);
        assert_eq!(out.connection, Some(connection()));
    }
    #[test]
    fn only_a_matching_disconnect_unlocks_credentials_after_failed_connect() {
        let mut m = lifecycle(vec![
            Outcome {
                connection: Some(connection()),
                success: false,
            },
            Outcome {
                connection: Some(connection()),
                success: false,
            },
            Outcome {
                connection: Some(connection()),
                success: true,
            },
        ]);
        assert_eq!(state(&mut m, &connect()).phase, ModelPhase::Unknown);
        assert_eq!(state(&mut m, &connect()).connection, Some(connection()));
        assert!(!m.allows_credential_mutation());
        state(&mut m, &RequestBody::ModelDisconnect { connection: id(3) });
        assert_eq!(m.runner.calls.len(), 1);
        assert_eq!(
            state(&mut m, &RequestBody::ModelDisconnect { connection: id(2) }).phase,
            ModelPhase::Unknown
        );
        assert_eq!(m.runner.calls.len(), 2);
        assert!(!m.allows_credential_mutation());
        assert_eq!(
            state(&mut m, &RequestBody::ModelDisconnect { connection: id(2) }).phase,
            ModelPhase::Idle
        );
        assert!(m.allows_credential_mutation());
        assert_eq!(
            m.runner.calls[1],
            ModelCommand::Disconnect {
                connection: connection()
            }
        );
        assert!(state(&mut m, &RequestBody::ModelStatus {})
            .connection
            .is_none());
    }
    #[test]
    fn connects_once_and_status_never_runs_a_command() {
        let mut m = lifecycle(vec![Outcome {
            connection: Some(connection()),
            success: true,
        }]);
        assert_eq!(
            state(&mut m, &RequestBody::ModelStatus {}).phase,
            ModelPhase::Idle
        );
        assert!(m.allows_credential_mutation());
        assert_eq!(state(&mut m, &connect()).phase, ModelPhase::Ready);
        assert_eq!(state(&mut m, &connect()).connection, Some(connection()));
        assert_eq!(
            state(&mut m, &RequestBody::ModelStatus {}).phase,
            ModelPhase::Ready
        );
        assert!(!m.allows_credential_mutation());
        assert_eq!(
            m.runner.calls,
            vec![ModelCommand::Connect {
                generation: id(1),
                selection: selection()
            }]
        );
    }
}
