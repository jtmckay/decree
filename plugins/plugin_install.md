---
name: plugin_install
machine:
  description: Install the project's plugins as their files say, each time the daemon starts.
  initial: install
  states:
    install:
      description: Run `decree plugin install`, which installs new and changed plugins, removes deleted ones and keeps edited copies.
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
    # plugin_install's only script. Installs every plugin in .decree/plugins/ that is new
    # or changed, and removes the installed copy of each deleted one. An installed copy
    # edited in place is never overwritten: a plugin that changed over one is a conflict,
    # which fails this run, so a person merges it (`decree plugin export <name>` keeps the
    # edit, `decree plugin install <name>` takes the plugin's version).
    set -euo pipefail
    decree plugin install
cron:
  boot.md: |
    ---
    cron: '@reboot'
    machine: plugin_install
    ---
    Install the plugins as their files say.
---
# plugin_install

decree installs a plugin only when asked: `decree plugin install`, `add` or `import`. `process`, `daemon` and `check` read `.decree/plugins/` and tell you what is not installed, but write nothing, so a pull that changes a plugin changes nothing until someone installs it.

This plugin asks for you. Its cron file, `.decree/machines/plugin_install/cron/boot.md`, fires once each time `decree daemon` starts (`@reboot`), and its run runs `decree plugin install`. A daemon restarted after `git pull` then runs the plugins as pulled.

- **Needs:** `decree` on the `PATH` of the daemon, and `.decree/machines/` writable. A container that mounts them read-only should not add this plugin; install on the host instead.
- **Conflicts:** a plugin that changed while its installed copy was edited in place is not installed, and the run fails, naming it. Its attention file tells a person to merge it.
- **More often:** add a cron file beside `boot.md` in the installed copy (`.decree/machines/plugin_install/cron/hourly.md` with `cron: 0 * * * *` and `machine: plugin_install`), then `decree plugin export plugin_install`.
