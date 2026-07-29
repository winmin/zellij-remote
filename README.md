# Zellij SSH Manager

A standalone Zellij WebAssembly plugin that reads concrete `Host` aliases from `~/.ssh/config`, then opens a local tab running a reconnecting default SSH login shell, remote tmux/Zellij session, or a pane-oriented tmux workspace.

## Build

```sh
cd /path/to/zellij-remote
cargo build --target wasm32-wasip1 --release
```

The plugin is written to `target/wasm32-wasip1/release/zellij-ssh-manager.wasm`. Put `bin/zellij-ssh-connector` on the `PATH` seen by Zellij, or configure its absolute path.

Merge the alias and keybinding entries from `ssh-manager.kdl` into the existing `plugins` and `keybinds.session` blocks in `~/.config/zellij/config.kdl`. Replace both absolute paths, restart Zellij, then press `Ctrl-o`, followed by `r`.

The plugin requests access to the session `HOME`, host files, command execution, and Zellij application state. It reads only through Zellij's `/host` mount after mapping that mount to `HOME`.

The connector accepts `--host` with a `shell`, `tmux`, `tmux-worker`, or `zellij` backend. The `shell` backend opens the host's default login shell and does not persist tasks; it takes no session. tmux, tmux-worker, and Zellij require a safe `--session` name. `tmux-worker` creates a detached session, turns its status line off, and attaches it. The connector reconnects indefinitely only when `ssh` exits with status 255, using delays of 1, 2, 4, 8, then 16 seconds.

## Pane workspace backend

Select **tmux workspace panes** to list existing remote workspace roots (`zr-<workspace>-p0001`) or choose **+ New workspace** and enter a safe name. The first local tab attaches to remote worker `zr-<workspace>-p0001`; subsequent optional `Shift-R` / `Shift-D` bindings send `split-right` / `split-down` to the plugin. The plugin reads the focused tab's connector commands to recover its host and workspace, then asynchronously lists remote tmux sessions before allocating the next independent worker index. In-flight worker names are reserved so rapid splits cannot duplicate a session; the local `zellij` CLI (override with the `zellij_cli` plugin setting) then creates the pane and launches the connector. This works after the plugin reloads or restarts and needs no custom plugin manifest. On ordinary tabs, or when the focused pane is not a workspace worker, these bindings show the picker with an error rather than splitting.

## Test

```sh
host=$(rustc -vV | sed -n 's/^host: //p')
cargo fmt --check
cargo test --target "$host"
cargo build --target wasm32-wasip1
sh -n bin/zellij-ssh-connector tests/connector.sh
sh tests/connector.sh
```
