decree: durable state-machine workflows from plain files.

A message says what to do, a machine says in what order, scripts do the work.

Install or update:
       curl -fsSL https://raw.githubusercontent.com/jtmckay/decree/main/install.sh | sh

1. Set up a project, then see the official plugins and add one:
       decree init
       decree list
       decree install decree_develop_rust

2. Write a script (chmod +x it):
       `.decree/machines/hello/scripts/greet.sh`
   Exit 0 is the event `done`; anything else is `error`.

3. Write a machine that runs it:
       `.decree/machines/hello/machine.yml`

4. Write a message for that machine:
       `.decree/inbox/hello.md`

5. Some more commands:
       decree check          validate machines
       decree graph          create mermaid charts
       decree daemon --once  process the queue, then exit
       decree status         what ran, and what needs you
       decree tail           follow runs' output as they go
       decree help           or https://github.com/jtmckay/decree#readme
