# What Código does today

The full list of what runs, kept in step with the code. The README has the short version; what is
planned is in [`ROADMAP.md`](../ROADMAP.md). Where something has not been tried against the real
service or tool, it says so.

- **A text editor built for speed.** Only the rows on screen are laid out, so a file of a million
  lines (45 MB) opens and scrolls without trouble. Multiple cursors and selections, word wrap, find and replace with a
  match count, line operations (move, duplicate, delete, join, comment, indent), unlimited undo and
  Unicode that behaves: grapheme-aware movement, wide characters, and the file's encoding and line
  endings kept when you save.
- **Syntax highlighting for 15 languages** through tree-sitter: Rust, Python, JavaScript,
  TypeScript, JSON, TOML, Markdown, Bash, C, C++, Go, Java, HTML, CSS and YAML. Other files open as
  plain text, and the status bar lets you pick a language by hand.
- **Errors as you type.** With a language server installed (rust-analyzer, pyright, gopls,
  clangd, typescript-language-server and others), Código underlines what it reports, shows the
  message under the pointer and counts errors and warnings in the status bar. The Problems panel
  (`Ctrl+Shift+M`, or a click on the counts) lists them for every file, errors first, and a click
  opens the place. It never installs a
  server for you, and the setting can be turned off in Preferences. Hover (`Ctrl+Shift+Space`),
  completion (`Ctrl+Space`, and as you type) and go to definition (`F12`) work too, and so do
  find references (`Shift+F12`), rename symbol (`F2`, changing every file the server names) and format document (`Alt+Shift+F`).
- **Snippets with places to fill in.** Type a word such as `fn` or `for` and press `Tab`: the
  snippet opens with its placeholders, `Tab` and `Shift+Tab` move between them and every copy of a
  repeated one changes together. Código ships snippets for its languages and reads your own from
  `snippets.json` in its configuration folder (*Edit snippets* opens it).
- **A minimap and a split editor.** A scaled picture of the file sits beside the editor, with the
  part on screen, the lines changed against git and the problems marked; click or drag to scroll
  (it can be turned off in Preferences). `Ctrl+\` splits the editor in two, to read one file
  while editing another or to see two places of the same file.
- **Files that open at once.** Opening a file no longer waits for anything: big files are read
  and parsed off the window, the text shows first and the colours arrive a moment later, and only
  the part of a very long line that is on screen is laid out.
- **A terminal and a run console.** The top bar has Run, Stop and Terminal buttons next to the
  palette. `Ctrl+\`` opens a real terminal under the editor, with tabs,
  colours, history and the project folder as its working directory. `F5` runs the project
  (`cargo run`, `npm start`, `go run .`, `python main.py`, Gradle, Maven, CMake, Make targets,
  Deno, Zig, Swift, Elixir, Flutter, Ruby, PHP and more, chosen from what the folder contains,
  or from the file you have open) in a tab of its own, and Build, Test and Check are in the Run
  menu. If nothing is recognised, Código asks once how to run it and remembers the answer in the
  project's `.codigo` folder. A
  `path:line:column` in the output is a link: click it to open that place.
- **A debugger.** The bug button next to Run (`F6`) builds the project and runs it under a
  debugger through the Debug Adapter Protocol: `lldb-dap` or GDB 14+ for Rust, C and C++, `dlv` for
  Go and `debugpy` for Python (the last two are not tried against a real session yet). Click the
  margin of the editor (or `F9`) to set breakpoints; when the program stops, the line is
  highlighted and the Debug panel shows the call stack, the variables (click to open them), an
  expression field and the program's output. Continue (`F8`), step over (`F10`), into (`F11`),
  out (`Shift+F11`) and Stop (`Shift+F6`) are in the top bar and the Run menu. A lone file can be
  debugged too, and when the program to debug is not recognised Código asks once and remembers.
  Código installs no debugger: when one is missing it says which to install. Conditional breakpoints and persistent
  watches are not there yet.
- **Git, in the window.** The Source control panel (`Ctrl+Shift+G`) shows the changes of the
  open repository, opens the diff of a file, stages and unstages, discards (with a second click to
  confirm), commits (`Ctrl+Enter` in the message box), switches and creates branches, and
  fetches, pulls and pushes. A diff can be staged by hunk (a button on each hunk stages,
  unstages or discards just that part), changes can be stashed and the latest stash applied, a branch can be
  merged into the current one, and when a merge leaves conflicts a window lists each block of a
  conflicted file and keeps *current*, *incoming* or *both* one block at a time before you mark
  the file resolved. *Show who changed each line* (`Ctrl+Alt+B`) writes the author, date and
  subject of the last change at the end of the cursor's line. Git is run as the `git` program on
  your computer, so your SSH keys and credential helper work as always.
- **GitHub, in the window.** The GitHub panel (`Ctrl+Shift+H`) lists the open pull requests and
  issues of the repository you are in, the pull requests you opened or that wait for your review,
  and your repositories. A pull request opens in its own window with its description, checks,
  files and diffs and conversation, and you can comment, approve or request changes, or check its
  branch out. *Clone repository…* (on the welcome page and in the GitHub menu) takes an address or
  `owner/name` and opens the clone. Sign in with the GitHub CLI session or a personal access
  token; the token is kept in the system's keyring and nowhere else. Signing in from the browser
  (device flow) is built in but needs a GitHub OAuth app, so it only shows when
  `CODIGO_GITHUB_CLIENT_ID` is set. Código ships no one else's client. Status, diffs, staging,
  committing, cloning and reads of public GitHub data have been tried; reviews, saving to the
  keyring and the browser sign-in have not been run against a real account. *Create a pull
  request…* pushes the current branch and opens a pull request from it, and in a pull request's
  diff a click on a line's number opens a comment on that line (shown as a dot, with its thread
  and a box to reply). Both go through the GitHub API and have not been run against a real
  repository either.
- **GitLab, in the window.** The GitLab panel (`Ctrl+Alt+L`, GitLab menu) works like the GitHub one
  for gitlab.com or your own GitLab server (set in Preferences ▸ Accounts): the open merge requests
  and issues of the project you are in, the merge requests you opened or that wait for your review,
  and your projects. A merge request opens in its own window with its description, pipeline status,
  changed files with their diffs and the conversation, and you can comment on it, approve it or
  open it on GitLab; *Create a merge request…* pushes the current branch and opens one from it, and
  *Clone a GitLab project…* takes an address or `group/project`. Sign in with a personal access
  token (the `api` scope; the panel links to the page that creates it): the token is kept in the
  system's keyring, one entry per server, is sent only to that server over HTTPS and is never
  written to a file. Pushing, pulling and cloning a project on the server you are signed in to use
  the same token. The GitHub and GitLab tabs and menus carry the services' own marks, drawn in the
  colour of the interface. Tried: signing in with a wrong token against the real gitlab.com (it says
  so), reading public projects, the address and remote parsing, and the token never appearing in an
  error. Not run against a real account: the lists, comments, approvals, creating a merge request,
  the keyring and cloning a private project.
- **Code quality and security, built in.** The Quality tab (`Ctrl+Shift+Y`) analyses the folder
  with no server to run and nothing leaving your computer: hard-coded secrets (private keys,
  cloud tokens, passwords), unsafe patterns (`eval`, shell commands from strings, SQL built from
  text, disabled TLS checks, weak hashes, unsafe C functions, `innerHTML`), Rust shortcuts that
  can panic, functions that are too long, too branching or take too many parameters, and blocks
  of code that were copied. Findings are listed worst first under a quality gate, and a click
  opens the exact place. Put `codigo-ignore` on a line to silence it. Fixes from the editor and
  dependency advisories are not there yet.
- **Tools for building Nodalia apps.** The Nodalia menu has *New Nodalia app*: it creates a Rust
  egui app in the Nodalia Design Language with the shared theme, an About window, Spanish and
  English with the test that checks every string, the lints and the CI the other apps use, and
  opens it. In a folder that depends on the Nodalia theme, the Quality tab also checks Nodalia's
  engineering rules: colours written in the code instead of taken from the tokens, interface text
  that skips `tr()`, strings missing from the `SPANISH` table, and files without a licence header.
  A theme and token preview and helpers for icons are not there yet.
- **AI assistants, with your own key.** The Assistant panel (`Ctrl+Alt+I`) sees what you are
  working on: the open file (a window of up to 40,000 characters around the caret), the selection,
  the open tabs and the project's file list, and `@name` in a question attaches that file. It
  answers about your code and, when a change is needed, proposes it as a diff card per file
  (*Change src/main.rs*, *New file src/util.rs*) with *Apply*, *Reject* and *Apply all*. A change to
  an open file goes through the editor, so *Undo* brings the old code back; a new file is written
  and opened. *Explain*, *Fix*, *Refactor*, *Document* and *Write tests for this code* in the
  Assistant menu send the selection (or the file) with that instruction. A switch in Preferences
  turns the context off. Pick Anthropic, OpenAI, Ollama or any OpenAI-compatible address in
  *Assistant settings…*. Código ships no key and no account: the key is kept in the system's
  keyring, is sent only to the provider's own host (over https, or to your own computer) and never
  appears in an error, and nothing is sent until you ask. The proposals are plain text blocks the
  model writes (it has no tools): a block whose old lines are not in the file, or appear twice, is
  shown as failed and is not applied. Tried end to end against a local server that replies with
  such blocks; the real Anthropic and OpenAI services and the keyring have not been tried.
- **Files you can tell apart.** Every file type has its own drawn symbol in the explorer and in
  the tabs, so you find the Rust, the Python or the Dockerfile at a glance.
- **Projects.** Open a folder to get a file explorer that respects `.gitignore` and shows git status
  in colour, quick open by fuzzy file name (`Ctrl+P`), search across the project (`Ctrl+Shift+F`)
  and a command palette (`Ctrl+Shift+P`) that reaches every command.
- **Pick up where you left off.** Tabs, the folder, the sidebar and recent folders come back at the
  next launch.
- **Nodalia Design.** Light, dark and high-contrast themes, round window controls on Linux (native
  ones on macOS and Windows), a global menu over D-Bus on Linux, a reduced-motion option and an
  in-app update check.
- **Make it look the way you want.** Preferences is a large window with categories and a search
  that looks through every setting. Pick a colour theme (Nodalia, which follows the window, or
  Dracula, One Dark, Monokai, Solarized, GitHub, Nord, Gruvbox, Tokyo Night and Catppuccin), then
  change any single colour of the editor or of each kind of code; the code font (any font installed
  on the computer), its size and line height; the cursor (line, block or underline, blinking or
  not); rulers and indent guides; the interface zoom; the terminal's shell; auto save and what is
  cleaned on save. A live preview shows each change as you make it. Not done yet: importing a
  VS Code theme, per-language settings and a keybindings editor.
- **Spanish and English**, following your system language or your choice in Preferences.
