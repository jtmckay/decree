---
name: plugin_install
machine:
  description: Install the project's plugins as their files say, each time the daemon starts.
  initial: install
  states:
    install:
      description: Run `decree install`, which installs every plugin, removes deleted ones and archives edited copies first.
      invoke: install
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  install.sh: |
    #!/usr/bin/env bash
    # plugin_install's only script. Installs every plugin in .decree/plugins/, and removes
    # the installed copy of each deleted one. An installed copy edited in place is first
    # archived as a plugin file, .decree/archive/<name>/<time>.md, so nothing is lost
    # (`decree install <that file>` restores it); `decree export <name>` before a restart
    # keeps an edit in the plugin file instead.
    set -euo pipefail
    decree install
cron:
  boot.md: |
    ---
    cron: '@reboot'
    machine: plugin_install
    ---
    Install the plugins as their files say.
---
# plugin_install

decree installs a plugin only when asked: `decree install`. `process`, `daemon` and `check` read `.decree/plugins/` and tell you what is not installed, but write nothing, so a pull that changes a plugin changes nothing until someone installs it.

This plugin asks for you. Its cron file, `.decree/machines/plugin_install/cron/boot.md`, fires once each time `decree daemon` starts (`@reboot`), and its run runs `decree install`. A daemon restarted after `git pull` then runs the plugins as pulled.

- **Needs:** `decree` on the `PATH` of the daemon, and `.decree/machines/` writable. A container that mounts them read-only should not add this plugin; install on the host instead.
- **Edits in place:** an installed copy edited in place, and not exported, is archived as a plugin file, `.decree/archive/<name>/<time>.md`, and the plugin installed over it, so export an edit you want to keep before restarting the daemon. That includes the edits a failed `develop` or `infra` run left for `decree retry`: `decree install` the archived file to put them back. Cron files are the exception: installed with a plugin the first time, they are then the project's, so a schedule changed or a cron file deleted in place stays (`decree install --cron` takes the plugin's). Only a plugin file that fails its schema fails the run.
- **More often:** add a cron file beside `boot.md` in the installed copy (`.decree/machines/plugin_install/cron/hourly.md` with `cron: 0 * * * *` and `machine: plugin_install`), then `decree export plugin_install`.
