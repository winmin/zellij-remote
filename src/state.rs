#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Tmux,
    TmuxWorker,
    Zellij,
    Shell,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
            Self::TmuxWorker => "tmux-worker",
            Self::Zellij => "zellij",
            Self::Shell => "shell",
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Tmux => Self::Shell,
            Self::TmuxWorker => Self::Tmux,
            Self::Zellij => Self::TmuxWorker,
            Self::Shell => Self::Zellij,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Tmux => Self::TmuxWorker,
            Self::TmuxWorker => Self::Zellij,
            Self::Zellij => Self::Shell,
            Self::Shell => Self::Tmux,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    HostSelection,
    BackendSelection,
    Loading,
    TmuxSessionSelection,
    SessionInput,
    WorkspaceInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Character(char),
    Backspace,
    Up,
    Down,
    Enter,
    Escape,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Render,
    Close,
    DiscoverTmuxSessions { host: String },
    Connect(CommandSpec),
    ConnectWorkspace(CommandSpec),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub tab_name: String,
}

#[derive(Debug)]
pub struct AppState {
    pub stage: Stage,
    pub hosts: Vec<String>,
    pub filter: String,
    pub selected: usize,
    pub host: Option<String>,
    pub backend: Backend,
    pub session: String,
    pub workspace: String,
    pub tmux_sessions: Vec<String>,
    pub connector: String,
    pub error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            stage: Stage::HostSelection,
            hosts: Vec::new(),
            filter: String::new(),
            selected: 0,
            host: None,
            backend: Backend::Tmux,
            session: "work".to_owned(),
            workspace: "work".to_owned(),
            tmux_sessions: Vec::new(),
            connector: "zellij-ssh-connector".to_owned(),
            error: None,
        }
    }
}

impl AppState {
    pub fn set_hosts(&mut self, hosts: Vec<String>) {
        self.hosts = hosts;
        self.selected = 0;
    }

    pub fn filtered_hosts(&self) -> Vec<&str> {
        let filter = self.filter.to_ascii_lowercase();
        self.hosts
            .iter()
            .filter(|host| host.to_ascii_lowercase().contains(&filter))
            .map(String::as_str)
            .collect()
    }

    pub fn handle(&mut self, input: Input) -> Effect {
        self.error = None;
        match self.stage {
            Stage::HostSelection => self.handle_host_input(input),
            Stage::BackendSelection => self.handle_backend_input(input),
            Stage::Loading => self.handle_loading_input(input),
            Stage::TmuxSessionSelection => self.handle_tmux_session_input(input),
            Stage::SessionInput => self.handle_session_input(input),
            Stage::WorkspaceInput => self.handle_workspace_input(input),
        }
    }

    fn handle_host_input(&mut self, input: Input) -> Effect {
        match input {
            Input::Character(character) if !character.is_control() => {
                self.filter.push(character);
                self.selected = 0;
            }
            Input::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            Input::Up => self.move_selection(-1),
            Input::Down => self.move_selection(1),
            Input::Enter => {
                let host = self
                    .filtered_hosts()
                    .get(self.selected)
                    .map(|host| (*host).to_owned());
                if let Some(host) = host {
                    self.host = Some(host);
                    self.stage = Stage::BackendSelection;
                } else {
                    self.error = Some("No matching SSH host".to_owned());
                }
            }
            Input::Escape | Input::Cancel => return Effect::Close,
            _ => {}
        }
        Effect::Render
    }

    fn handle_backend_input(&mut self, input: Input) -> Effect {
        match input {
            Input::Up => self.backend = self.backend.previous(),
            Input::Down => self.backend = self.backend.next(),
            Input::Character('s') | Input::Character('S') => self.backend = Backend::Shell,
            Input::Character('t') | Input::Character('T') => self.backend = Backend::Tmux,
            Input::Character('w') | Input::Character('W') => self.backend = Backend::TmuxWorker,
            Input::Character('z') | Input::Character('Z') => self.backend = Backend::Zellij,
            Input::Enter if self.backend == Backend::Shell => match self.command_spec() {
                Ok(spec) => return Effect::Connect(spec),
                Err(error) => self.error = Some(error),
            },
            Input::Enter if matches!(self.backend, Backend::Tmux | Backend::TmuxWorker) => {
                if let Some(host) = self.host.clone() {
                    self.tmux_sessions.clear();
                    self.selected = 0;
                    self.stage = Stage::Loading;
                    return Effect::DiscoverTmuxSessions { host };
                }
                self.error = Some("No SSH host selected".to_owned());
            }
            Input::Enter => self.stage = Stage::SessionInput,
            Input::Escape => self.stage = Stage::HostSelection,
            Input::Cancel => return Effect::Close,
            _ => {}
        }
        Effect::Render
    }

    fn handle_loading_input(&mut self, input: Input) -> Effect {
        match input {
            Input::Escape => self.stage = Stage::BackendSelection,
            Input::Cancel => return Effect::Close,
            _ => {}
        }
        Effect::Render
    }

    fn handle_tmux_session_input(&mut self, input: Input) -> Effect {
        match input {
            Input::Up => self.move_tmux_selection(-1),
            Input::Down => self.move_tmux_selection(1),
            Input::Enter => {
                if let Some(session) = self.tmux_sessions.get(self.selected).cloned() {
                    self.session = session;
                    return match self.command_spec() {
                        Ok(spec) => Effect::Connect(spec),
                        Err(error) => {
                            self.error = Some(error);
                            Effect::Render
                        }
                    };
                }
                if self.backend == Backend::TmuxWorker {
                    self.workspace = "work".to_owned();
                    self.stage = Stage::WorkspaceInput;
                } else {
                    self.session = "work".to_owned();
                    self.stage = Stage::SessionInput;
                }
            }
            Input::Escape => self.stage = Stage::BackendSelection,
            Input::Cancel => return Effect::Close,
            _ => {}
        }
        Effect::Render
    }

    fn handle_session_input(&mut self, input: Input) -> Effect {
        match input {
            Input::Character(character) if is_session_character(character) => {
                self.session.push(character);
            }
            Input::Character(_) => {
                self.error = Some("Session may only contain A-Z, a-z, 0-9, _, . and -".to_owned());
            }
            Input::Backspace => {
                self.session.pop();
            }
            Input::Enter => match self.command_spec() {
                Ok(spec) => return Effect::Connect(spec),
                Err(error) => self.error = Some(error),
            },
            Input::Escape => self.stage = Stage::BackendSelection,
            Input::Cancel => return Effect::Close,
            _ => {}
        }
        Effect::Render
    }

    fn handle_workspace_input(&mut self, input: Input) -> Effect {
        match input {
            Input::Character(character) if is_session_character(character) => {
                self.workspace.push(character);
            }
            Input::Character(_) => {
                self.error =
                    Some("Workspace may only contain A-Z, a-z, 0-9, _, . and -".to_owned());
            }
            Input::Backspace => {
                self.workspace.pop();
            }
            Input::Enter => match self.workspace_command_spec() {
                Ok(spec) => return Effect::ConnectWorkspace(spec),
                Err(error) => self.error = Some(error),
            },
            Input::Escape => self.stage = Stage::BackendSelection,
            Input::Cancel => return Effect::Close,
            _ => {}
        }
        Effect::Render
    }

    fn move_selection(&mut self, amount: isize) {
        let count = self.filtered_hosts().len();
        if count == 0 {
            self.selected = 0;
        } else {
            self.selected = (self.selected as isize + amount).rem_euclid(count as isize) as usize;
        }
    }

    fn move_tmux_selection(&mut self, amount: isize) {
        let count = self.tmux_sessions.len() + 1;
        self.selected = (self.selected as isize + amount).rem_euclid(count as isize) as usize;
    }

    pub fn apply_tmux_sessions_result(
        &mut self,
        host: &str,
        exit_code: Option<i32>,
        stdout: &[u8],
        stderr: &[u8],
    ) -> bool {
        if self.stage != Stage::Loading || self.host.as_deref() != Some(host) {
            return false;
        }

        let (mut sessions, error) = parse_tmux_sessions_result(exit_code, stdout, stderr);
        if self.backend == Backend::TmuxWorker {
            sessions.retain(|session| worker_workspace_root(session).is_some());
        }
        self.tmux_sessions = sessions;
        self.selected = 0;
        self.error = error;
        self.stage = Stage::TmuxSessionSelection;
        true
    }

    pub fn command_spec(&self) -> Result<CommandSpec, String> {
        let host = self
            .host
            .as_ref()
            .ok_or_else(|| "No SSH host selected".to_owned())?;
        if self.backend == Backend::Shell {
            return Ok(CommandSpec {
                program: self.connector.clone(),
                args: vec![
                    "--host".to_owned(),
                    host.clone(),
                    "--backend".to_owned(),
                    self.backend.as_str().to_owned(),
                ],
                tab_name: format!("{host}/shell"),
            });
        }
        if !valid_session(&self.session) {
            return Err("Session must match [A-Za-z0-9_.-]+".to_owned());
        }
        Ok(CommandSpec {
            program: self.connector.clone(),
            args: vec![
                "--host".to_owned(),
                host.clone(),
                "--backend".to_owned(),
                self.backend.as_str().to_owned(),
                "--session".to_owned(),
                self.session.clone(),
            ],
            tab_name: format!("{host}/{}", self.session),
        })
    }

    pub fn workspace_command_spec(&self) -> Result<CommandSpec, String> {
        let host = self
            .host
            .as_ref()
            .ok_or_else(|| "No SSH host selected".to_owned())?;
        if !valid_workspace(&self.workspace) {
            return Err("Workspace must match [A-Za-z0-9_.-]+".to_owned());
        }
        let worker = worker_session_name(&self.workspace, 1);
        Ok(CommandSpec {
            program: self.connector.clone(),
            args: vec![
                "--host".to_owned(),
                host.clone(),
                "--backend".to_owned(),
                Backend::TmuxWorker.as_str().to_owned(),
                "--session".to_owned(),
                worker,
            ],
            tab_name: format!("{host}/{}", self.workspace),
        })
    }
}

pub fn worker_session_name(workspace: &str, pane_id: u32) -> String {
    format!("zr-{workspace}-p{pane_id:04}")
}

pub fn valid_workspace(workspace: &str) -> bool {
    valid_session(workspace)
}

pub fn worker_workspace_root(session: &str) -> Option<&str> {
    let workspace = session.strip_prefix("zr-")?.strip_suffix("-p0001")?;
    valid_workspace(workspace).then_some(workspace)
}

pub fn parse_tmux_sessions_result(
    exit_code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> (Vec<String>, Option<String>) {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);

    if exit_code == Some(0) {
        let mut sessions: Vec<String> = stdout
            .lines()
            .map(str::trim)
            .filter(|session| valid_session(session))
            .map(str::to_owned)
            .collect();
        sessions.sort();
        sessions.dedup();
        return (sessions, None);
    }

    let output = format!("{stdout}\n{stderr}");
    if exit_code == Some(1) && output.to_ascii_lowercase().contains("no server running") {
        return (Vec::new(), None);
    }

    let detail = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(100).collect::<String>())
        .unwrap_or_else(|| match exit_code {
            Some(code) => format!("ssh exited with status {code}"),
            None => "ssh did not report an exit status".to_owned(),
        });
    (
        Vec::new(),
        Some(format!("Could not list tmux sessions: {detail}")),
    )
}

pub fn valid_session(session: &str) -> bool {
    !session.is_empty() && session.chars().all(is_session_character)
}

fn is_session_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtering_is_case_insensitive_and_preserves_order() {
        let mut state = AppState::default();
        state.set_hosts(vec!["Prod-One".into(), "dev".into(), "prod-two".into()]);
        state.handle(Input::Character('P'));
        state.handle(Input::Character('r'));
        state.handle(Input::Character('o'));

        assert_eq!(state.filtered_hosts(), vec!["Prod-One", "prod-two"]);
        state.handle(Input::Down);
        state.handle(Input::Enter);
        assert_eq!(state.host.as_deref(), Some("prod-two"));
        assert_eq!(state.stage, Stage::BackendSelection);
    }

    #[test]
    fn host_selection_wraps_and_escape_closes() {
        let mut state = AppState::default();
        state.set_hosts(vec!["one".into(), "two".into()]);

        state.handle(Input::Up);
        assert_eq!(state.selected, 1);
        assert_eq!(state.handle(Input::Escape), Effect::Close);
    }

    #[test]
    fn escape_moves_back_through_later_stages() {
        let mut state = AppState::default();
        state.set_hosts(vec!["host".into()]);
        state.handle(Input::Enter);
        state.backend = Backend::Zellij;
        state.handle(Input::Enter);
        assert_eq!(state.stage, Stage::SessionInput);

        state.handle(Input::Escape);
        assert_eq!(state.stage, Stage::BackendSelection);
        state.handle(Input::Escape);
        assert_eq!(state.stage, Stage::HostSelection);
    }

    #[test]
    fn backend_can_be_selected_with_keys_and_cycles_in_display_order() {
        let mut state = AppState::default();
        state.stage = Stage::BackendSelection;

        assert_eq!(state.backend, Backend::Tmux);
        state.handle(Input::Down);
        assert_eq!(state.backend, Backend::TmuxWorker);
        state.handle(Input::Down);
        assert_eq!(state.backend, Backend::Zellij);
        state.handle(Input::Down);
        assert_eq!(state.backend, Backend::Shell);
        state.handle(Input::Down);
        assert_eq!(state.backend, Backend::Tmux);
        state.handle(Input::Up);
        assert_eq!(state.backend, Backend::Shell);
        state.handle(Input::Up);
        assert_eq!(state.backend, Backend::Zellij);
        state.handle(Input::Up);
        assert_eq!(state.backend, Backend::TmuxWorker);
        state.handle(Input::Character('s'));
        assert_eq!(state.backend, Backend::Shell);
        state.handle(Input::Character('T'));
        assert_eq!(state.backend, Backend::Tmux);
        state.handle(Input::Character('w'));
        assert_eq!(state.backend, Backend::TmuxWorker);
        state.handle(Input::Character('z'));
        assert_eq!(state.backend, Backend::Zellij);
        state.handle(Input::Character('S'));
        assert_eq!(state.backend, Backend::Shell);
    }

    #[test]
    fn shell_backend_connects_immediately_without_validating_session() {
        let mut state = AppState {
            stage: Stage::BackendSelection,
            host: Some("prod".to_owned()),
            backend: Backend::Shell,
            session: "not a valid session".to_owned(),
            connector: "/opt/bin/connector".to_owned(),
            ..AppState::default()
        };

        assert_eq!(
            state.handle(Input::Enter),
            Effect::Connect(CommandSpec {
                program: "/opt/bin/connector".to_owned(),
                args: vec!["--host", "prod", "--backend", "shell"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                tab_name: "prod/shell".to_owned(),
            })
        );
        assert_eq!(state.stage, Stage::BackendSelection);
    }

    #[test]
    fn tmux_backend_starts_session_discovery() {
        let mut state = AppState {
            stage: Stage::BackendSelection,
            host: Some("prod".to_owned()),
            ..AppState::default()
        };

        assert_eq!(
            state.handle(Input::Enter),
            Effect::DiscoverTmuxSessions {
                host: "prod".to_owned()
            }
        );
        assert_eq!(state.stage, Stage::Loading);
    }

    #[test]
    fn discovered_session_can_be_selected_and_connected() {
        let mut state = AppState {
            stage: Stage::Loading,
            host: Some("prod".to_owned()),
            ..AppState::default()
        };

        assert!(state.apply_tmux_sessions_result(
            "prod",
            Some(0),
            b" zeta\nalpha\nzeta\nbad name\n",
            b"",
        ));
        assert_eq!(state.tmux_sessions, vec!["alpha", "zeta"]);
        state.handle(Input::Up);
        assert_eq!(state.selected, 2);
        state.handle(Input::Down);
        assert_eq!(state.selected, 0);
        state.handle(Input::Down);
        let effect = state.handle(Input::Enter);
        assert!(matches!(effect, Effect::Connect(_)));
        assert_eq!(state.session, "zeta");
    }

    #[test]
    fn new_session_entry_opens_session_input() {
        let mut state = AppState {
            stage: Stage::Loading,
            host: Some("prod".to_owned()),
            ..AppState::default()
        };
        state.apply_tmux_sessions_result("prod", Some(0), b"existing\n", b"");

        state.handle(Input::Down);
        assert_eq!(state.handle(Input::Enter), Effect::Render);
        assert_eq!(state.stage, Stage::SessionInput);
        assert_eq!(state.session, "work");
    }

    #[test]
    fn empty_discovery_result_still_offers_new_session() {
        let mut state = AppState {
            stage: Stage::Loading,
            host: Some("prod".to_owned()),
            ..AppState::default()
        };

        state.apply_tmux_sessions_result("prod", Some(0), b"", b"");
        assert!(state.tmux_sessions.is_empty());
        assert_eq!(state.handle(Input::Enter), Effect::Render);
        assert_eq!(state.stage, Stage::SessionInput);
    }

    #[test]
    fn discovery_error_still_allows_a_new_session() {
        let mut state = AppState {
            stage: Stage::Loading,
            host: Some("prod".to_owned()),
            ..AppState::default()
        };

        assert!(state.apply_tmux_sessions_result(
            "prod",
            Some(255),
            b"",
            b"ssh: connect timed out\n",
        ));
        assert_eq!(state.stage, Stage::TmuxSessionSelection);
        assert!(state.error.is_some());
        assert_eq!(state.handle(Input::Enter), Effect::Render);
        assert_eq!(state.stage, Stage::SessionInput);
    }

    #[test]
    fn escape_cancels_loading_and_session_selection() {
        let mut state = AppState {
            stage: Stage::Loading,
            host: Some("prod".to_owned()),
            ..AppState::default()
        };

        state.handle(Input::Escape);
        assert_eq!(state.stage, Stage::BackendSelection);
        assert!(!state.apply_tmux_sessions_result("prod", Some(0), b"old\n", b""));
        assert!(state.tmux_sessions.is_empty());

        state.stage = Stage::TmuxSessionSelection;
        state.handle(Input::Escape);
        assert_eq!(state.stage, Stage::BackendSelection);
    }

    #[test]
    fn stale_host_is_ignored_and_tmux_output_is_normalized() {
        let mut state = AppState {
            stage: Stage::Loading,
            host: Some("new-host".to_owned()),
            ..AppState::default()
        };
        assert!(!state.apply_tmux_sessions_result("old-host", Some(0), b"old\n", b""));
        assert_eq!(state.stage, Stage::Loading);

        let (sessions, error) = parse_tmux_sessions_result(Some(0), b" b\na\nb\nbad/name\n\n", b"");
        assert_eq!(sessions, vec!["a", "b"]);
        assert_eq!(error, None);

        let (sessions, error) = parse_tmux_sessions_result(
            Some(1),
            b"",
            b"no server running on /tmp/tmux-1000/default\n",
        );
        assert!(sessions.is_empty());
        assert_eq!(error, None);

        let (_, error) = parse_tmux_sessions_result(Some(255), b"", b"ssh: timed out\n");
        assert_eq!(
            error.as_deref(),
            Some("Could not list tmux sessions: ssh: timed out")
        );
    }

    #[test]
    fn session_validation_rejects_empty_and_disallowed_characters() {
        assert!(valid_session("work.2-prod_test"));
        assert!(!valid_session(""));
        assert!(!valid_session("two words"));
        assert!(!valid_session("shell;rm"));

        let mut state = AppState::default();
        state.stage = Stage::SessionInput;
        state.session.clear();
        state.handle(Input::Character('/'));
        assert!(state.session.is_empty());
        assert!(state.error.is_some());
    }

    #[test]
    fn command_spec_has_separate_arguments_and_tab_name() {
        let state = AppState {
            host: Some("prod".to_owned()),
            backend: Backend::Zellij,
            session: "work-2".to_owned(),
            connector: "/opt/bin/connector".to_owned(),
            ..AppState::default()
        };

        assert_eq!(
            state.command_spec().unwrap(),
            CommandSpec {
                program: "/opt/bin/connector".to_owned(),
                args: vec![
                    "--host",
                    "prod",
                    "--backend",
                    "zellij",
                    "--session",
                    "work-2"
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                tab_name: "prod/work-2".to_owned(),
            }
        );
    }

    #[test]
    fn workspace_validation_and_worker_names_are_safe_and_deterministic() {
        assert!(valid_workspace("project.2-prod_test"));
        assert!(!valid_workspace(""));
        assert!(!valid_workspace("project/work"));
        assert_eq!(worker_session_name("project", 1), "zr-project-p0001");
        assert_eq!(worker_session_name("project", 42), "zr-project-p0042");
    }

    #[test]
    fn tmux_worker_discovers_and_connects_to_existing_workspace_root() {
        let mut state = AppState {
            stage: Stage::BackendSelection,
            host: Some("prod".to_owned()),
            backend: Backend::TmuxWorker,
            connector: "/opt/bin/connector".to_owned(),
            ..AppState::default()
        };
        assert_eq!(
            state.handle(Input::Enter),
            Effect::DiscoverTmuxSessions {
                host: "prod".to_owned()
            }
        );
        state.apply_tmux_sessions_result(
            "prod",
            Some(0),
            b"zr-project-p0001\nzr-project-p0002\nzr-other-p0001\nordinary\n",
            b"",
        );
        assert_eq!(
            state.tmux_sessions,
            vec!["zr-other-p0001", "zr-project-p0001"]
        );
        assert_eq!(
            state.handle(Input::Enter),
            Effect::Connect(CommandSpec {
                program: "/opt/bin/connector".to_owned(),
                args: [
                    "--host",
                    "prod",
                    "--backend",
                    "tmux-worker",
                    "--session",
                    "zr-other-p0001",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                tab_name: "prod/zr-other-p0001".to_owned(),
            })
        );
    }

    #[test]
    fn tmux_worker_selector_offers_new_workspace_and_root_parser_is_strict() {
        assert_eq!(worker_workspace_root("zr-project-p0001"), Some("project"));
        assert_eq!(worker_workspace_root("zr-project-p001"), None);
        assert_eq!(worker_workspace_root("zr-project-p0002"), None);
        assert_eq!(worker_workspace_root("project"), None);

        let mut state = AppState {
            stage: Stage::Loading,
            host: Some("prod".to_owned()),
            backend: Backend::TmuxWorker,
            ..AppState::default()
        };
        state.apply_tmux_sessions_result("prod", Some(0), b"zr-project-p0002\n", b"");
        assert!(state.tmux_sessions.is_empty());
        assert_eq!(state.handle(Input::Enter), Effect::Render);
        assert_eq!(state.stage, Stage::WorkspaceInput);
        assert_eq!(state.workspace, "work");

        let mut failed_discovery = AppState {
            stage: Stage::Loading,
            host: Some("prod".to_owned()),
            backend: Backend::TmuxWorker,
            ..AppState::default()
        };
        failed_discovery.apply_tmux_sessions_result("prod", Some(255), b"", b"ssh: timed out\n");
        assert_eq!(failed_discovery.stage, Stage::TmuxSessionSelection);
        assert!(failed_discovery.error.is_some());
        assert_eq!(failed_discovery.handle(Input::Enter), Effect::Render);
        assert_eq!(failed_discovery.stage, Stage::WorkspaceInput);
    }

    #[test]
    fn tmux_worker_workspace_connects_with_first_worker() {
        let mut state = AppState {
            stage: Stage::WorkspaceInput,
            host: Some("prod".to_owned()),
            backend: Backend::TmuxWorker,
            connector: "/opt/bin/connector".to_owned(),
            workspace: "project".to_owned(),
            ..AppState::default()
        };
        assert_eq!(
            state.handle(Input::Enter),
            Effect::ConnectWorkspace(CommandSpec {
                program: "/opt/bin/connector".to_owned(),
                args: [
                    "--host",
                    "prod",
                    "--backend",
                    "tmux-worker",
                    "--session",
                    "zr-project-p0001",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                tab_name: "prod/project".to_owned(),
            })
        );
    }

    #[test]
    fn shell_command_spec_omits_session_and_uses_backend_tab_name() {
        let state = AppState {
            host: Some("prod".to_owned()),
            backend: Backend::Shell,
            session: "not a valid session".to_owned(),
            connector: "/opt/bin/connector".to_owned(),
            ..AppState::default()
        };

        assert_eq!(
            state.command_spec().unwrap(),
            CommandSpec {
                program: "/opt/bin/connector".to_owned(),
                args: vec!["--host", "prod", "--backend", "shell"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                tab_name: "prod/shell".to_owned(),
            }
        );
    }
}
