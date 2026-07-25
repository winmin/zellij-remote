mod parser;
mod state;

use parser::parse_ssh_config;
use state::{AppState, Backend, CommandSpec, Effect, Input, Stage};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use zellij_tile::prelude::*;

#[derive(Default)]
struct PluginState {
    app: AppState,
    status: String,
    home_ready: bool,
}

register_plugin!(PluginState);

impl ZellijPlugin for PluginState {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        if let Some(connector) = configuration.get("connector") {
            if !connector.is_empty() {
                self.app.connector = connector.clone();
            }
        }
        if let Some(default_backend) = configuration.get("default_backend") {
            self.app.backend = match default_backend.as_str() {
                "shell" => Backend::Shell,
                "zellij" => Backend::Zellij,
                _ => Backend::Tmux,
            };
        }
        self.status = "Waiting for permissions...".to_owned();
        subscribe(&[
            EventType::Key,
            EventType::HostFolderChanged,
            EventType::FailedToChangeHostFolder,
            EventType::PermissionRequestResult,
            EventType::RunCommandResult,
        ]);
        request_permission(&[
            PermissionType::ReadSessionEnvironmentVariables,
            PermissionType::FullHdAccess,
            PermissionType::RunCommands,
            PermissionType::ChangeApplicationState,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                let environment = get_session_environment_variables();
                match environment.get("HOME").filter(|home| !home.is_empty()) {
                    Some(home) => {
                        self.status = "Loading ~/.ssh/config...".to_owned();
                        self.home_ready = true;
                        change_host_folder(PathBuf::from(home));
                    }
                    None => {
                        self.status = "HOME is not available in the Zellij session".to_owned();
                    }
                }
                true
            }
            Event::PermissionRequestResult(PermissionStatus::Denied) => {
                self.status = "Required permissions were denied".to_owned();
                true
            }
            Event::HostFolderChanged(_) if self.home_ready => {
                let parsed = parse_ssh_config(Path::new("/host/.ssh/config"), Path::new("/host"));
                self.app.set_hosts(parsed.hosts);
                self.status = if self.app.hosts.is_empty() {
                    parsed
                        .warnings
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "No concrete Host aliases found".to_owned())
                } else if parsed.warnings.is_empty() {
                    format!("Loaded {} SSH hosts", self.app.hosts.len())
                } else {
                    format!(
                        "Loaded {} SSH hosts ({} warning{})",
                        self.app.hosts.len(),
                        parsed.warnings.len(),
                        if parsed.warnings.len() == 1 { "" } else { "s" }
                    )
                };
                true
            }
            Event::FailedToChangeHostFolder(error) => {
                self.status = error.unwrap_or_else(|| "Failed to access HOME".to_owned());
                true
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                if context.get("kind").map(String::as_str) != Some("tmux_sessions") {
                    return false;
                }
                context
                    .get("host")
                    .map(|host| {
                        self.app
                            .apply_tmux_sessions_result(host, exit_code, &stdout, &stderr)
                    })
                    .unwrap_or(false)
            }
            Event::Key(key) => {
                if let Some(input) = map_key(key) {
                    match self.app.handle(input) {
                        Effect::Close => close_self(),
                        Effect::DiscoverTmuxSessions { host } => self.discover_tmux_sessions(host),
                        Effect::Connect(spec) => self.connect(spec),
                        Effect::Render => return true,
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        let mut lines = vec!["SSH Manager".to_owned(), String::new()];
        match self.app.stage {
            Stage::HostSelection => {
                lines.push(format!("Host filter: {}", self.app.filter));
                lines.push(String::new());
                let hosts = self.app.filtered_hosts();
                if hosts.is_empty() {
                    lines.push("  (no matching hosts)".to_owned());
                } else {
                    let available = rows.saturating_sub(8).max(1);
                    let start = self
                        .app
                        .selected
                        .saturating_sub(available.saturating_sub(1));
                    for (index, host) in hosts.iter().enumerate().skip(start).take(available) {
                        let marker = if index == self.app.selected { ">" } else { " " };
                        lines.push(format!("{marker} {host}"));
                    }
                }
                lines.push(String::new());
                lines.push("Type to filter | ↑/↓ or Ctrl-p/Ctrl-n | Enter | Esc".to_owned());
            }
            Stage::BackendSelection => {
                lines.push(format!(
                    "Host: {}",
                    self.app.host.as_deref().unwrap_or("(none)")
                ));
                lines.push("Choose backend:".to_owned());
                for backend in [Backend::Tmux, Backend::Zellij, Backend::Shell] {
                    let marker = if backend == self.app.backend {
                        ">"
                    } else {
                        " "
                    };
                    let label = match backend {
                        Backend::Tmux => "tmux",
                        Backend::Zellij => "zellij",
                        Backend::Shell => "shell (default login shell)",
                    };
                    lines.push(format!("{marker} {label}"));
                }
                lines.push(String::new());
                lines.push("↑/↓ or t/z/s | Enter | Esc back".to_owned());
            }
            Stage::Loading => {
                lines.push(format!(
                    "Host: {}  Backend: tmux",
                    self.app.host.as_deref().unwrap_or("(none)")
                ));
                lines.push(String::new());
                lines.push("Loading tmux sessions...".to_owned());
                lines.push(String::new());
                lines.push("Esc back".to_owned());
            }
            Stage::TmuxSessionSelection => {
                lines.push(format!(
                    "Host: {}  Backend: tmux",
                    self.app.host.as_deref().unwrap_or("(none)")
                ));
                lines.push("Choose a tmux session:".to_owned());
                let items: Vec<String> = self
                    .app
                    .tmux_sessions
                    .iter()
                    .cloned()
                    .chain(std::iter::once("+ New session".to_owned()))
                    .collect();
                let available = rows.saturating_sub(8).max(1);
                let start = self
                    .app
                    .selected
                    .saturating_sub(available.saturating_sub(1));
                for (index, item) in items.iter().enumerate().skip(start).take(available) {
                    let marker = if index == self.app.selected { ">" } else { " " };
                    lines.push(format!("{marker} {item}"));
                }
                lines.push(String::new());
                lines.push("↑/↓ or Ctrl-p/Ctrl-n | Enter | Esc back".to_owned());
            }
            Stage::SessionInput => {
                lines.push(format!(
                    "Host: {}  Backend: {}",
                    self.app.host.as_deref().unwrap_or("(none)"),
                    self.app.backend.as_str()
                ));
                lines.push(format!("Session: {}", self.app.session));
                lines.push(String::new());
                lines.push("Allowed: A-Z a-z 0-9 _ . -".to_owned());
                lines.push("Enter connect | Esc back".to_owned());
            }
        }
        if let Some(error) = &self.app.error {
            lines.push(format!("Error: {error}"));
        } else if !self.status.is_empty() {
            lines.push(self.status.clone());
        }

        for (row, line) in lines.into_iter().take(rows).enumerate() {
            let clipped: String = line.chars().take(cols).collect();
            print_text_with_coordinates(Text::new(clipped), 0, row, None, None);
        }
    }
}

impl PluginState {
    fn discover_tmux_sessions(&self, host: String) {
        let mut context = BTreeMap::new();
        context.insert("kind".to_owned(), "tmux_sessions".to_owned());
        context.insert("host".to_owned(), host.clone());
        run_command(
            &[
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "--",
                &host,
                "tmux list-sessions -F '#{session_name}'",
            ],
            context,
        );
    }

    fn connect(&mut self, spec: CommandSpec) {
        let command = CommandToRun::new_with_args(&spec.program, spec.args);
        let (tab_id, _pane_id) = open_command_pane_in_new_tab(command, BTreeMap::new());
        if let Some(tab_id) = tab_id {
            rename_tab_with_id(tab_id as u64, spec.tab_name);
            close_self();
        } else {
            self.app.error = Some("Zellij did not create the connector tab".to_owned());
        }
    }
}

fn map_key(key: KeyWithModifier) -> Option<Input> {
    if key.bare_key == BareKey::Char('c') && key.has_modifiers(&[KeyModifier::Ctrl]) {
        return Some(Input::Cancel);
    }
    if key.bare_key == BareKey::Char('p') && key.has_modifiers(&[KeyModifier::Ctrl]) {
        return Some(Input::Up);
    }
    if key.bare_key == BareKey::Char('n') && key.has_modifiers(&[KeyModifier::Ctrl]) {
        return Some(Input::Down);
    }
    if !key.has_no_modifiers() {
        return None;
    }
    match key.bare_key {
        BareKey::Char(character) => Some(Input::Character(character)),
        BareKey::Backspace => Some(Input::Backspace),
        BareKey::Up => Some(Input::Up),
        BareKey::Down => Some(Input::Down),
        BareKey::Enter => Some(Input::Enter),
        BareKey::Esc => Some(Input::Escape),
        _ => None,
    }
}
