# Zellij SSH Manager

A standalone Zellij WebAssembly plugin that reads concrete `Host` aliases from `~/.ssh/config`, then opens a local tab running a reconnecting default SSH login shell or a remote tmux or Zellij session.

## Build

```sh
cd /path/to/zellij-remote
cargo build --target wasm32-wasip1 --release
```

The plugin is written to `target/wasm32-wasip1/release/zellij-ssh-manager.wasm`. Put `bin/zellij-ssh-connector` on the `PATH` seen by Zellij, or configure its absolute path.

Merge the alias and keybinding entries from `ssh-manager.kdl` into the existing `plugins` and `keybinds.session` blocks in `~/.config/zellij/config.kdl`. Replace both absolute paths, restart Zellij, then press `Ctrl-o`, followed by `r`.

The plugin requests access to the session `HOME`, host files, command execution, and Zellij application state. It reads only through Zellij's `/host` mount after mapping that mount to `HOME`.

The connector accepts `--host` with a `shell`, `tmux`, or `zellij` backend. The `shell` backend opens the host's default login shell and does not persist tasks; it takes no session. tmux and Zellij require a safe `--session` name. The connector reconnects indefinitely only when `ssh` exits with status 255, using delays of 1, 2, 4, 8, then 16 seconds.

## Test

```sh
host=$(rustc -vV | sed -n 's/^host: //p')
cargo fmt --check
cargo test --target "$host"
cargo build --target wasm32-wasip1
sh -n bin/zellij-ssh-connector tests/connector.sh
sh tests/connector.sh
```
