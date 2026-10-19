decree: durable state-machine workflows from plain files.

A message says what to do, a machine says in what order, scripts do the work.

Install or update

       curl -fsSL https://raw.githubusercontent.com/jtmckay/decree/v0.5/install.sh | sh

1. Set up a project
       decree init
2. Write a script (chmod +x it):
       `.decree/scripts/greet.sh`
   Exit 0 is the event `done`; anything else is `error`.
3. Write a machine that runs it:
       `.decree/machines/hello.yml`
4. Write a message for that machine: 
       `.decree/migrations/01-hello.md`
5. Some commands
       decree check         validate machines
       decree graph         create mermaid charts
       decree process       process all messages
       decree status        what ran, and what needs you
       decree tail          follow runs' output as they go
       decree help          or https://github.com/jtmckay/decree#readme
