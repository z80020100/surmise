# surmise

Completion for `cd` directories, Git subcommands, and any command with a
committed specification.

> This file also provides guidance to [Claude Code](https://claude.ai/code) when
> working with code in this repository. `README.md`, `AGENTS.md` and `GEMINI.md`
> are symbolic links to it.

**surmise installs, runs and has its reference.** The licence is settled. The
platform is not.

## Prerequisites

- Rust 1.98.0. `rust-toolchain.toml` pins it and rustup installs it on demand
- Target platforms: macOS and Linux. CI builds and checks both. Nothing else
  has been built and nothing else is claimed

## Install

```sh
cargo install --git https://github.com/z80020100/surmise
```

Then in `~/.zshrc`, after zsh-autosuggestions and zsh-syntax-highlighting and
after `bindkey -e` or `bindkey -v`:

```sh
eval "$(surmise init zsh)"
```

`cargo install` places a binary and has no mechanism for anything beside it.
`shell/surmise.zsh` is therefore compiled into that binary and `init` prints
it. The two cannot fall out of step, because they are one artifact. The
`make shell` gate reads the same bytes the command emits.

One shell can still hold an older copy. The `eval` above runs once and the
function it defines lives as long as that shell does, so a session open
when the binary is replaced keeps sending the widget it already has. The
record the widget writes therefore leads with a tag naming its own shape,
and a binary reading a record without that tag reads the shape that came
before it rather than the fields behind it. A shell in that state completes
on everything but what the newer field carries. Starting a shell, or
running the `eval` again in the one you have, is what pairs the two.

zsh-autosuggestions asks for a new suggestion after a widget it wrapped
changes the line. It wraps the widgets `zle -la` lists when it binds. Under
`ZSH_AUTOSUGGEST_MANUAL_REBIND` that binding happens once and the first prompt
is the one that gets it. A surmise sourced at the prompt after that is on no
list of its own. surmise asks for the suggestion itself whenever it writes the
line and the ghost text follows the line either way.

## Demo

```sh
surmise demo
```

That starts an interactive zsh in your own home and in the directory you ran
it from. Nothing is installed and your own `.zshrc` is neither read nor
changed.

**The files are your own and so is the home.** The menu completes on the
directory you started in, so `cd `, `git add `, `git switch ` and `make ` all
answer out of the project in front of you, and a line you run in that shell
runs. `$HOME` is yours as well, so `ssh ` reads the hosts you have, the `~`
row is your own home and the `$HISTFILE` tie-break reads the history file
the paragraph below names. This is a shell of your own rather than a sandbox.

**So is everything surmise writes.** The config, the state file and the
directory history are the ones an installed surmise reads and writes. A
`surmise settings set` typed in the demo is therefore the setting you keep
afterwards and a `cd` you run there is a visit the menus you open tomorrow
rank by. That is the point rather than a cost. A setting that only ever
reached a copy would answer nothing about the setting and trying surmise is
the whole of what this command is for.

**`$ZDOTDIR` is the one thing that moves.** Leaving that shell takes the
directory it names away. The demo needs a `.zshrc` of its own: a person
trying surmise before they install it has no `eval "$(surmise init zsh)"`
line in theirs and writing one into it is the one thing a demo may not do.
`-d` drops the machine's own `/etc` files beside that. What is loaded in
that shell is therefore surmise and nothing else, and a menu that misbehaves
there has nowhere to hide: no framework, no plugin and no keybinding of
anybody else's sits in front of it.

`$HISTFILE` is the one file read and never written. `SAVEHIST=0` in that
`.zshrc` is what has zsh read the file you keep and write nothing back to
it, so the demo leaves not a line of its own in it.

Which file that is, is a guess. zsh exports no `HISTFILE` and the `.zshrc`
naming it is the one a clean shell does not read, so the demo falls back to
`~/.zsh_history` and its opening text says so where nothing is there. A
person whose history lives somewhere else points the demo at it with
`HISTFILE=... surmise demo`. Reading their own `.zshrc` for the name would
load everything else in it as well, which is the one thing this shell is for
not doing.

`shell/demo.zsh` is the `.zshrc` it writes and the binary carries that file
the way it carries the widget. The file holds no path of its own and reads
`$SURMISE_BIN` out of the environment. The bytes `make shell` checks are
therefore the bytes that run. The demo installs the widget through the same
`eval "$(surmise init zsh)"` line this file gives a person.

The opening text also offers the glyph set, to a person whose own `icons`
does not already ask for it. Nothing can detect the font those glyphs want,
so the demo draws the set in front of a person and names the command that
turns it on. Boxes rather than shapes is a terminal without such a font.
That command writes the config you keep and the line beside it names the
`unset` that puts it back. A demo that hid a real write behind an offer
would be worse than not offering at all. A person whose config already says
`nerd` is told the menu below is drawing it rather than told to switch on
what they switched on themselves, and the demo reads `surmise settings show`
to tell the two apart.

`surmise` in that shell is the binary that started the demo rather than
whichever one the PATH holds. A person reading a branch back out of a clone
has no installed one and the command has to be that build's.

A demo that is killed rather than left leaves that directory behind under the
temporary one, with the one file in it. It is owner-only and so is the
per-user temporary directory above it. Nothing reads the directory again and
the next demo makes one of its own.

## Use

Tab asks surmise about the line you already have and falls through to the
shell's own completion when surmise has nothing to offer. The paragraphs
under the table say what narrows that. A bare `cd ` also opens surmise on its
own, and so does a space after any other command surmise has a specification
for; "Other commands" below says what that opens and what it costs. A menu
of directories opens below the line. The mark on the first
name sits under the cursor and the menu follows the cursor along the line. The
menu is one width whatever it holds. Near the right edge it keeps that width
and gives the alignment up. A terminal too narrow for that width is the one
thing that shrinks it. Six rows show at a time and a shorter list or a
shorter terminal shows fewer. A line above the first of them closes the
panel and carries the position in the list at its right end. A line under
the last of them separates the list from the word below it. Each name has
one character in front of it that says what sort of row it is, in a shape
and a colour of that sort's own. Every one of those
characters is one a terminal draws as plain text in the colour it is given.
`icons = "nerd"` in the configuration swaps that set for one a patched font
draws instead. A file row then reads its own name: 76 extensions and 16 whole
names such as `Makefile` and `.gitignore` reach 33 shapes of their own, under
one of five colours for code, for data, for a document, for an image or a
sound, and for an archive. The shape says which file it is and the colour
says what sort of file that is. A name neither table answers for keeps the
plain file shape and the colour it already wore. So does every row that names
something other than a file. The set moves the shapes and leaves the palette
where the eye last found it. Nothing detects such a font and the set
is therefore a person's to turn on. A terminal without one draws a box in
place of every shape. A terminal set for CJK widths draws each one two cells
wide where surmise's own table calls it one and slides every name a column
right.
At the bottom of the screen the terminal
scrolls to make the room rather than the menu moving above the line. What is
above the line is the shell's own output and surmise cannot read it back to
put it there again. The menu holds still while the highlight has
somewhere to go inside it and follows the highlight one row at a time past
that. Up on the first name and Down on the last wrap to the other end of the
list and the menu follows in one step.

A selected name that does not fit its row also appears below the list, under
the line that separates the two. It keeps the list's own column in either
layout: the row it came from is up there and the cut it took is that column's
doing. Where the word sits under the list it remains below that. The
wrapped name carries no marks and no underline. The row in the list is what
says how the name got in and what Tab would take. A short terminal
limits the extra rows and an ellipsis marks any text that still does not fit.
Without spare rows the menu shows only the list and its label.

| Key | Inside the menu |
| --- | --- |
| Up and Down | Move the highlight |
| Shift-Tab | Move the highlight back |
| Tab | Take a highlighted `../` whole or the prefix the directories share |
| Right | Take what the highlighted directory adds. At the end of the line |
| Enter | Go into the highlighted directory or run the line |
| Esc | Leave the menu and keep what you typed |
| Ctrl-O | Open the whole of the word, beside the list where it fits |
| Ctrl-C and Ctrl-G | Leave and restore the line you started with |

A cursor that sits inside a word narrows Tab before surmise reads anything
else on the line. When the character to its right is not a space or a tab,
Tab hands the key to the shell's own completion instead, because completing
there would split a word still being typed.

A line that already names a directory gets a row of its own at the top of the
menu. That row runs the line rather than growing it. A `↵` in a colour of its
own is the whole row: the line is on the screen already and a name there would
be that same text a second time. The word under the list is what says what the
row does. Enter on any other row takes that directory and leaves the menu open
on what is inside it. Two presses therefore go one level down and run the line.
Enter also runs the line when there is nothing left to take. That is what
keeps the key working once the cursor has moved off the argument.

That row is the one the highlight starts on. Any line whose argument already
names a directory therefore opens a menu rather than falling through to the
shell's own completion. A bare `cd ..` or `cd .` is such a line and the row
that runs it arrives with whatever else the dots in it match. A dot in front
of the last part of the argument also turns the hidden names on. That is the
trade the row costs.

Each directory also offers `../` when the last part of the argument matches
it. An empty last part offers it after the children. Typing `cd ..` starts
on the row that runs the line. Enter there goes up and Tab leaves the line
alone. Select `../` and Enter or Tab instead adds the slash and opens the
parent's children. That menu also offers `../` so you can continue up.
The parent row remains available when the children fill the menu's limit.

Tab takes a highlighted `../` row whole. Otherwise it reads the child
directories and skips the parent and home shortcuts. Except for a `..`
argument it also looks past the row that runs the line. One directory that
leads with what you typed goes in whole, inside quotes as well as outside
them. Past that it takes the prefix they all share. It leaves the line alone
when that prefix adds nothing, when the shell would not read it as a single
literal word, when what they share is the whole of one of the names, when they
spell that shared part differently, when the argument is inside quotes and
when the argument ends in a space. The quote is because half a name cannot
carry the one that closes it. The space is you saying the word is finished.
Right leaves it alone for that same reason and Enter runs the line, because a
finished word leaves nothing to take.

Tab rings the terminal's bell whenever it leaves the line alone. The line is
the one that was already there and nothing on the screen would say the key
had been read at all. Whether that bell is a sound or a flash or nothing is
the terminal's own setting rather than surmise's.

Under the row that runs the line the names come in an order of their own. The
name that is what you typed leads and the case either is in makes no
difference. Names leading with what you typed come next and the rest follow.
Inside each group the history of successful directory changes comes first.
The closer text match follows and names that tie keep their alphabetical order.
Each source directory has preferences of its own. The menu reads those
preferences once when it opens and keeps them while you type.

The names themselves are read the same way. Each directory the argument
reaches is walked once and the menu answers every later key from what that
walk found. A directory made while the menu is open therefore arrives with the
next menu rather than with the next key.

The zsh directory hook records successful changes from manual commands and
from surmise. Selecting a name does not record a visit. Executing the change
does. Returning to the same physical directory records nothing. Quiet changes
such as `cd -q` suppress the hook. An earlier hook that fails can prevent it
from running too. This history reads no shell command text. A separate
reader does, for Git's own menu and the one "Other commands" describes below.

The records stay in `$XDG_DATA_HOME/surmise/history.sqlite3`. Without an
absolute `XDG_DATA_HOME` the path is `~/.local/share/surmise/history.sqlite3`.
Without either an absolute data directory or an absolute home directory the
history is unavailable. Opening the menu does not create a database.
The first recorded change creates it. Each record holds physical source and
target paths plus a visit count, the last visit time and a decaying weight.
Symbolic links to the same directory share that record. No command text is stored.

Each visit adds one to the weight. Existing weight halves every 30 days.
Records unused for more than 180 days no longer affect sorting.
The next write removes those records. History only reorders directories the
menu already found. It does not add destinations or expand the scan limit.
The parent and home rows do not use history weights.

SQLite serializes writes from concurrent shells. A write waits up to 50 ms
for a lock before giving up. A read does not wait for a lock.
A failed write loses that visit. Missing, locked or unreadable history leaves
the text order in place. Storage errors do not print at the prompt.
SQLite is compiled into the binary through `rusqlite`.
No SQLite command or service needs to be installed.

A second reader reads `$HISTFILE`, the shell's own command history, for
Git's own menu and "Other commands" below. The widget passes its path in the
same stdin record that already carries `RBUFFER` and the alias table. It
reads the last 5000 entries and at most 64 KiB of the file, the cap a Git
query's own output already carries, from the file's tail rather than its
start, so a long history costs what a short one does. It reads both zsh
history formats, the plain one and the extended `: <epoch>:<elapsed>;<command>`
one, and joins a line a trailing backslash continues before it reads that
line's words.

An entry reads the way a shell reads it rather than as a naive split on
whitespace: `cd sample && git status` teaches a pair for `cd` and a pair for
`git`, a leading `FOO=bar` is never mistaken for a command's name, and
`alias g='git -C sample'` expands `g`'s whole value before the words are
counted, so a person who types `g` still teaches `git`'s own menu. Only the
first two words of each command matter: how often its own second word
followed its first. A command of one word teaches nothing. A missing,
unreadable or empty file teaches nothing and prints nothing. So does a path
that is not a regular file: a named pipe would hold the shell's own line
editor open until somebody wrote to it and a device would answer a length
of nothing and then read for as long as it was asked. What it reads lives
only for the one menu that asked for it. Nothing here is written to a file
and no command text is stored, the same promise the directory history above
already keeps.

The file is read once when the menu opens and only for a line one of those
two menus answers. That reading then stands for as long as the menu does,
the same way the directory preferences above are read once and kept. A menu
opened on a line neither of them answers therefore has no reading at all,
and editing that line into one they do answer does not go back for one. The
line has to empty and close the menu before the next one reads the file.

What that reading may order is therefore a second word and nothing else. A
Git subcommand is one and so is the word a specification's menu offers
straight after the command name. A branch name, a file name, an option's
value and every row further along a specification's own walk sit at the
third word or later, where this reading has counted nothing about them, and
they keep the order they would have had without it. That is not only a
lookup saved. A branch named for a subcommand somebody types often would
otherwise climb a list it has no business leading.

A row that answers to several names is the one thing this misses. "Other
commands" below shows such a row once, under the first of its names, and
this reading counted whatever was actually typed. `npm install` also
answers to `i` and to `add`, so a person who types `npm i` teaches the pair
`("npm", "i")` and the row on screen says `install`, which that pair never
reaches. Sixteen of `npm`'s own subcommands answer to more than one name.
The row is ordered as though it had never been typed.

A match need not lead with what you typed and Tab ignores the rows that do
not. `cd wk` reaching `work/` is a match Enter takes. Right leaves it: that
key wants a name leading with what you typed, whatever case either is in. It
is not a match Tab can build a prefix from either.

Every name marks the characters what you typed reached. The `w` and the `k`
carry a ground and a brighter name of their own in that row and the `or`
between them does not. That is what says how a name got into the menu.

What Tab would add is underlined in every row it would add it to. The rows
Tab passes over carry no underline and neither does a line Tab would leave
alone. The key and the underline read one answer and the menu therefore
cannot promise what the key will not do.

Everything else is ordinary line editing. Left, Home, End, Backspace, Delete
and Ctrl-A, Ctrl-E, Ctrl-U and Ctrl-K all do what they do in the shell. Ctrl-W
takes back one path segment where the shell would take the whole path.
Emptying the line leaves the menu as well and keeps the empty line.

`bindkey ' ' $_surmise_space` after the `eval` gives the space key back and
keeps the Tab route.

`SURMISE_BIN` names the binary. It defaults to the `surmise` on the PATH.
That is the one that printed the widget when `init` was run from there.

## Git

Typing a bare `git ` opens a subcommand menu. Tab also opens it on a partial
subcommand such as `git stat`. Exact names lead. Prefixes follow and fuzzy
matches come last. A tie inside one of those three goes to the name
`$HISTFILE` says was typed after `git` more often. A further tie keeps the
closer text match and then the alphabetical order. The menu includes
high-level Git commands and configured alias names. It reads command names
from the installed Git once per menu.
It does not run aliases. That query prints a name and nothing else, so the
word under the list comes from the command's own completion specification
instead: it is the highlighted subcommand's description, read once per menu
the way every other menu reads one.

That specification is frozen at the release `specs/` was read from and the
installed Git is not. Git 2.50 lists 74 names here. 34 of them reach an entry
with a sentence, `blame` reaches the one entry that carries no sentence and
the remaining 39 reach no entry at all. 40 of the 74 rows therefore carried
the word `command` and nothing else. What fills them is Git itself.
`git help -a` writes a one-line summary for every command the installed Git
ships. The menu reads that answer once, the way it reads everything else it
needs. The sentence a row shows is therefore the Git on this machine rather
than a corpus that stopped moving.

The specification still answers first. A subcommand it describes keeps its
own sentence and everything said below about that sentence is what it always
was. The second query is what a row still on the fallback asks for and
nothing else asks for it: a word whose every remaining row already reached a
sentence spawns Git no second time. What it answers is the word under the
list and nothing besides. The arguments beside a name stay the
specification's. 39 of those 40 rows have no entry there to take one from and
what they gain is therefore a sentence and nothing more.

That answer describes Git's own guides and file formats as well. Those are
nobody's subcommand. Nothing in it decides what one is. The names in the list
are the name query's own and a row looks itself up by the name it already
has.

A name neither source answers for keeps the word `command`. `git help -a`
writes an external command as a name with nothing beside it and that is one
such name. A Git old enough to print no summaries at all leaves every row
exactly where the specification left it. A configured alias is Git's own to
describe and where a person wrote it is what decides whether Git does.
`git help` reads no repository at all. The aliases this answer carries are
therefore the ones written outside a repository. Each sits under a heading of
its own with the command line it stands for beside it. That line is what the
row then says. An alias written into one repository's own configuration is a
row the name query still prints — that query does read the repository — and
this answer never reaches it. Such a row keeps the word `command`.

Every row also carries the arguments that still fit beside its own name,
drawn dim from that same specification: `<name>` for a mandatory one,
`[name]` for an optional one, and `...` inside either for a variadic one.
They show in order and stop at the first that does not fit rather than
skipping it for a shorter one further along. `checkout` carries two:
`[branch, file, tag or commit]` is 29 cells beside an 8-cell name and one
space, exactly the panel's 38-cell budget for the two together, so
`[pathspec...]` after it has no room and neither it nor anything past it
shows. A name the specification does not carry, or one whose own arguments
carry no name, shows none.

Those descriptions are the upstream's own sentences rather than labels
written for this panel, and so are Git's own. Half of them are longer than
the panel is wide: of the 38 the committed `git` specification carries, 18
are cut short at the footer's edge. The position counter used to take its
own cells off the end of that word and 22 of the 38 left it none. It sits
on the panel's top edge now and the word under the list has the panel's
whole width whatever it holds.

That word takes one row unasked. Every row below the list comes out of what
the list and a wrapped name have left and the highlighted name has first
claim on them: it says which row the keys would act on and the sentence only
says what that row does.

A sentence that row cannot hold loses its first parenthetical and everything
past its first full stop. What is left is shown whole rather than cut short.
`Use TCP/IP device (error if multiple TCP/IP devices are available)` is 66
cells and `Use TCP/IP device` is 17. A full stop closes a sentence only where
two letters or digits run into it. `e.g. ` and an initial therefore cut
nothing. 66.8% of the corpus's 371 943 descriptions are wider than one row
and 53.7% still are once the trim has run. The ellipsis is what those get.

Ctrl-O opens the whole of that word and closes it again. The rule above the
word carries the key the way the top edge carries the position in the list.
Both sit on an edge because that is the one place either costs the word
nothing. An open word breaks at a space rather than wherever the cells run
out. A word split over two rows has to be read twice and 37 of git's own 38
fit in two rows. Nothing is trimmed while it is open. The key asked for the
whole of it and a clause is not that. Q binds this to Ctrl-K and surmise
keeps Ctrl-K as the shell's own kill-line.

A terminal 75 cells wide or wider opens the word beside the list, in a panel
of its own 30 cells wide. That leaves the five cells an eighty-cell terminal
has over for the menu to go on following the cursor in. Eighty is what a
terminal has before anybody widens one and a pair that filled one would stand
at the left edge there whatever the cursor was doing. Beside the list the
word takes no row off it. The line under the list and the word's own row
under that are gone and the list spends one of the two on a seventh name.
The word has the rows the list spends and every row the terminal spared under
them. Its panel has a line above it level with the list's own top edge and
that line carries the key. Each panel is as tall as what it holds. The two
open on one row and the shorter of them ends where its own last row does.

A narrower terminal opens the word under the list instead, to every row the
list and a wrapped name left. There is nothing between those two: the same
sentence in a third shape is one the eye has to find again every time a
terminal changes size. A terminal with no row to spare leaves the word on the
rows it already had, down to the clause they show.

The answer outlives the menu it was given in. It is kept in
`$XDG_DATA_HOME/surmise/state.toml`, beside the directory history and
resolved the same way. Every later menu opens the way the key last left it.
That file is surmise's own note to itself rather than a setting. That is why
it is not `config.toml`: this is written because a key was pressed and nobody
asked for it to be kept, and `config.toml` is only ever written by a person
naming the change themselves. "Configuration" below is where that is written
down. Nothing here reaches the prompt either. A file that will not read
leaves the word on its one row and a write that will not land loses one
menu's answer.

Enter accepts the highlighted subcommand and adds a space. Right does the same
when the name starts with what you typed. Tab accepts the shared prefix or a
single prefix match. Accepting `switch` or `checkout` opens the branch menu
when branches are available. Accepting `add` opens the file menu when files
are available. Other complete subcommands return the line to the shell for
further editing. A word no subcommand matches returns the line as well. Tab
on such a word opens no menu and the shell completes it instead. Accepting a
subcommand does not execute it. Esc keeps the edited line. Ctrl-C and Ctrl-G
restore the line that opened the menu.

What comes after a returned subcommand is the menu "Other commands" below
describes and Tab there is what opens it. `git blame ` opens on the files
beside the line and `git stash ` on the eleven children of its own. The
acceptance does not open that menu itself. Enter there takes the first row,
so the second press that runs `git status` would write `--ahead-behind` onto
the line instead. Behind a branch it would be worse. `git checkout <branch> `
offers the paths beside the line first, and running that line writes the
branch's copy of a file over the one on disk.

Typing `git switch ` or `git checkout ` also opens the branch menu.
Tab opens it on a partial branch name. The menu includes local branches and
remote branch names that Git can infer from locally stored remote refs.
Remote names have no remote prefix. A name shared by several remotes needs
`checkout.defaultRemote` to select one of them. `checkout.guess=false` disables
remote candidates. The menu excludes symbolic remote refs such as the remote's
`HEAD`. It does not fetch branches or contact a remote. The branch the
repository is on wears a mark of its own. The row is a branch row like any
other and the name on it says nothing about where the repository stands.

Branch matching uses the whole name including every `/`. Enter accepts the
highlighted branch. Right accepts a prefix match. Tab accepts a shared prefix
or a single prefix match. Accepting a whole branch adds a space and returns
the line to the shell for editing. It does not execute the command. Tab
behind the branch opens what the subcommand's own specification asks for
next, in the menu "Other commands" below describes: `switch`'s own options,
and `checkout`'s beside the paths its second argument takes. No second
branch is offered there. The branch reader answered the word it was asked
about and the walk answers the next one.
Esc keeps the edited line. Ctrl-C and Ctrl-G restore the line that opened the
menu. The menu reads branches and their settings once when it first needs
them. Branches created later appear in the next menu.

Typing `git add ` opens a menu of files, folders and options.
Tab opens it on a partial argument. Files and folders precede options when
the argument is empty. The default file list includes unstaged
modifications, unstaged deletions and untracked files below the current
directory. It excludes ignored untracked files and files with no changes
since their last staging. Git reads its standard ignore rules. Each candidate
shows its full path relative to the current directory. Nested files appear
individually. A leading `./` stays on the completed path.

Folder candidates come from the filesystem. They include empty, unchanged
and ignored folders. Hidden folders appear when their name starts with a
typed dot. The `.git` directory is excluded. So is a symbolic link to a
folder and so is every path through one: Git refuses a pathspec beyond a
symbolic link. Accepting a folder adds its trailing slash and opens its
children. The folder itself remains the first
row. Enter on that row accepts the whole folder and adds a space.
Files below an accepted folder are excluded from later candidates.
An argument of `.` offers the current directory as a whole.

File matching uses the whole path. Enter accepts the highlighted file.
Right accepts a prefix match. Tab accepts a shared prefix or a single prefix
match. Accepting a whole file adds a space and offers the remaining files.
Files already on the line are excluded. Tab can reopen completion after an
earlier file argument. Single quotes, literal double quotes and escaped
characters are supported. Shell expansions stay with shell completion.
Acceptance does not stage files or execute the command. Esc keeps the edited line.
Ctrl-C and Ctrl-G restore the line that opened the menu. Options can remain
after all files are selected. Esc returns to the shell before Enter runs
the command. Each file query and directory listing stays fixed until the
next menu.

The option names and aliases follow the [Q Git completion specification](https://github.com/withfig/autocomplete/blob/aef52acff84c45edde61ae610cc2c964802b9a38/src/git.ts).
The menu shows a short description of the highlighted option in its footer.
It excludes options already on the left of the cursor and their aliases.
Short option groups such as `-nv` are supported. Options can appear before
or after paths. Accepting `--` ends option completion and allows paths that
start with `-`.

Accepting `--chmod` adds `=` and offers `+x` and `-x`.
`--pathspec-from-file` completes a file argument after a space or `=`.
That argument uses filesystem names including unchanged files and absolute
paths. A `-` value reads standard input. These names receive shell quoting
without Git pathspec prefixes. After this argument only unused options are
offered. The path file is not read by completion.

`--force` includes ignored untracked files. `--update`, `--patch` and `--edit`
limit file candidates to tracked changes. `--renormalize` and `--refresh`
also include unchanged tracked files. `--no-all` and `--ignore-removal`
exclude deleted files. Each combination has its own cached file query.
Folder candidates remain available with these options.

Accepted file names receive shell quoting when needed. Git wildcard
characters and a leading `:` also receive a literal pathspec prefix. A
leading `-` receives `./` so Git reads a path. Names with control characters
are omitted and so are names that are not UTF-8.

Only the first branch argument without preceding options uses the branch
menu. File completion accepts multiple arguments and the documented `add`
options. Everything else on a `git` line falls to the menu "Other commands"
below describes, which walks `specs/git.json` rather than asking the
installed Git anything: another subcommand's own options and arguments, a
subcommand behind a `git` global option such as `git -C sample status `, an
`add` argument that reader refuses such as an absolute path or one with a
`..` in it, and a `git` that is not the first word of the line. What that
menu finds there is the specification's own and no more. An argument the
corpus marks `dyn` offers only what the specification itself lists for it:
its own suggestions and the files or folders a template names. The script
the corpus kept beside those is never run. One the specification names and
hands nothing to fill offers nothing at all. The new branch name after
`switch -c` or `checkout -b` is such an argument and that line still shows
nothing. Git candidates rank by that text, and a subcommand row by
`$HISTFILE` as well. A branch row and a file row rank by
the text alone. Directory history does not affect any of them. Branch
queries outside a repository leave completion to the shell. The `add`
options and filesystem candidates remain available outside a repository. An
argument with no candidates uses shell completion.

Git must be on PATH. The command query allows 250 ms and at most 64 KiB of
output. The description query has its own budget with the same limits and
runs only where a row still carries the fallback word. The branch and
configuration queries share a separate budget with the same limits. The file
query has its own budget with these limits. A missing Git, a failed query or a
query that exceeds either limit supplies no Git candidates.
Independent filesystem and option candidates remain available.
Errors do not print at the prompt.
The command query uses Git's experimental `--list-cmds` interface.
The description query is `git help -a`. Its answer here is 11 988 bytes and
about 10 ms. A failed one leaves every row on the word it already had.
The branch query uses `for-each-ref` and reads `refs/heads` and `refs/remotes`.
The default file query uses `ls-files --modified --others --exclude-standard -z`.
Options can select `--cached`, omit `--others` or omit `--exclude-standard`.
Folder completion uses the existing directory cache with its 400-name limit.
Path-file arguments cache at most 400 filesystem entries per directory.
A Git version that does not support a query supplies no candidates for it.

## Other commands

Tab opens a third menu, behind Git's own and `cd`'s, for any command that has
a committed specification. `docker `, `npm ` and `cargo ` reach it. `cd` never
does, because its own menu above this one answers every line its reader
claims and `cd.json`'s two rows are no answer at all to a directory name. The
command name resolves through the shell's alias table first, so an alias for
`docker` opens the menu under the name it expands to.

`git` reaches this menu for whatever Git's own leaves. That menu reads three
things and no more: the subcommand word, a branch after `switch` or
`checkout`, and a path or an option after `add`. Everything else on a `git`
line fell straight through to the shell. `git blame `, `git clean `,
`git stash ` and `git commit -` each did, while `specs/git.json` sat there
saying that `blame` takes a file, that `stash` has children of its own and
what every one of `commit`'s options does. Those lines walk that specification
now. The three Git's own menu claims it keeps, because what answers them is
the installed Git rather than a file frozen in May 2025: a branch made this
morning is in `for-each-ref`'s output and in no corpus. `crate::app::App` is
the one place that order is written down, as `App::reader`. Git's own parser
reads the line first and the walk is only ever asked about a line it declined.

A subcommand the corpus has no node for is what that costs, and it costs
nothing in the end. Git's own menu offers every name the installed Git prints,
its configured aliases among them, and the committed specification answers for
fewer. Accepting one of the rest and typing a space walks that specification
to a `git` with nowhere to go: the word is spent on `git`'s own optional
`alias` argument rather than descending anywhere, and a node that declares
subcommands offers neither them nor its own options behind such an argument.
`git sample-alias ` therefore shows nothing at all, which is where it stood
before any of this. Offering `add` there would put a subcommand one word too
late and offering `--bare` would put a global option one word too late, and
Git refuses both.

That last rule belongs to the walk rather than to `git`. A command's own
options sit in front of the subcommand rather than behind it — `git -C sample
status`, never `git status -C sample` — so a word that has started filling a
node's own arguments is past those options the same way it is past the
subcommand. A node with no subcommand of its own has no such line to sit
behind and goes on offering its options after its arguments, which is what
`git add <file> -n` and `svn commit -m` both need. `src/argwalk.rs` is where
that is written down.

An alias for `cd` is the one exception to the refusal above, and it reaches
this menu rather than `cd`'s own: the refusal tests the word as typed, not
what it expands to, because `cd`'s own reader matches only the literal word
and never claims a line that starts with an alias for it. `alias c=cd`
therefore opens on `cd.json`'s own two rows, `-` and `~`, not on a real
directory name, which stays the directory scan's alone.

An alias for `git` reaches this menu for the whole of a line rather than for
the part Git's own declined, and for the same reason: `crate::git::parse`
matches the literal `git` and nothing else. `alias g=git` therefore opens on
`git`'s own specification throughout — its subcommands, its options and their
descriptions — and never on a branch name or a file name, which stay the
installed Git's to answer. `git switch ` offers the branches. `g switch `
offers `switch`'s own options and the one suggestion its argument carries,
`-`, because that argument is one of the 4854 the corpus marks `dyn`.

`spec_menu` loads the named command's specification and asks `argwalk` to
walk the line against it. A row comes from whatever the walk says the next
word may be: a child subcommand, an option not already on the line, or one of
an argument's own listed suggestions. A subcommand or an option that answers
to several names shows once, under the first of them. Each row's label is the
spec's own description, or `"command"`, `"option"` or `"value"` for a row
whose spec carries none. A subcommand or an option row also carries the
arguments that still fit beside its name, drawn dim: `<name>` for a
mandatory one, `[name]` for an optional one, and `...` inside either for a
variadic one. `crate::spec::arg_hints` turns the row's own `args` into that
list and `ui` shows as many whole entries as fit in order, stopping at the
first that does not rather than reaching past it for a shorter one. Three
rows carry none. A suggestion row has no arguments of its own. A `help` row
names a sibling subcommand and fills `help`'s own one-word argument with it,
so the sibling's own arguments are never reached and naming them would
promise a word the menu will not offer. An option that requires a separator
takes `--name=value`, which the space in front of a hint would deny, and it
carries none until that separator is part of what the row inserts.

Rows rank the way Git's own do: exact names lead, prefixes follow and fuzzy
matches come last. Where the row would become the command's own second word,
a tie inside one of those three goes to the name `$HISTFILE` says followed
that command more often, the same reader Git's own menu reads. A row any
deeper into the line is past what that reader counted and takes nothing from
it. A further tie keeps the fuzzy score's own order. The specification's
own `priority` breaks that. The group the row came from breaks what is
still level: the subcommands lead, the values the argument in hand takes
follow and the options come last. A subcommand is the next word the command
is made of and a value is the word its argument wants. An option is
neither. The alphabetical order breaks whatever remains.

An empty argument is where that group order decides. Every row matches it
equally well. The name alone decided before it and `-` sorts under every
letter. A bare `cargo ` therefore opened on `--color` and left all 38 of
its subcommands under the fold. 323 of the 715 specifications at the top of
`specs/` carry both a subcommand and an option at their root. Typing a `-`
is how a person asks for the options instead. A row the argument leads with
outranks the group it came from and so does a row the argument reaches
better.

`priority` is a number from 0 to 100 that a specification writes against a
subcommand, an option or a listed value. A row whose specification says
nothing is worth 50 and a number outside the range is closed to it. 151 of
the 1481 specification files carry one and 4504 rows in all have it.
`svn commit ` is what it is for. The corpus puts `-m` at 100, `--username`
at 95 and `--password` at 94. The alphabetical order buried the one flag
that command cannot run without under six that configure the connection.
Nothing writes a number for a file, a folder, a make target or an ssh
host. `specs/git.json` and `specs/cd.json` carry none between them,
so no row of Git's own menu or `cd`'s is worth anything but 50.

Git's own menu reads those same two fields for the rows it names itself,
whatever this one goes on to answer behind them. "Git" above is where that
is written down.

A command can point the walk past its own specification at another's.
`sudo git switch ` walks `git`'s own specification from the `git` token
onward rather than `sudo`'s, the same way `aws account ` walks
`aws/account`'s and `python -m http.server` walks `python/http.server`'s.
Each specification a line re-roots through loads once and stays cached for
the rest of the menu, the same as the command name itself does, and a
re-root can itself point at a third: `sudo aws account ` loads all three. A
name the corpus has no answer for simply does not re-root, leaving whatever
asked for it to answer from its own specification instead — `exec` asking
for a command that does not exist offers nothing back, since `exec` itself
declares no option or subcommand of its own to fall back to.

Enter and Tab accept a row the way they already do in Git's own menu: Enter
takes the highlighted row and Tab takes the prefix every matching row agrees
on, or a row's whole name where only one agrees. Accepting a row never runs
it. Moving the cursor off the word a row would replace makes that row go
stale, the same way a Git row does.

A row that takes nothing out of the list ends the menu. `ls ` offers every
name in the directory and the argument behind it takes as many names as it is
given. The menu an accepted file would reopen is therefore the menu that was
already there and the next press would put that same name on the line a second
time. Accepting one hands the line back to the shell instead. The word is
finished and a space follows it and the press after that runs the line. A row
that changes what comes next keeps the menu open: a subcommand moves the walk
to another node and a folder moves the scan into itself. Tab reads the same
answer for a whole name it takes. The prefix several rows share is not a whole
name and never ends the menu: the rows it came from are the rows that still
match it. Neither `cd`'s own menu nor Git's is affected. Each of their queries
drops what it has just given.

A generator's own template answers for three of the four names the corpus
carries. `filepaths` and `folders` read the filesystem the way `cd`'s own menu
does, through the one directory walk a menu already keeps for its own life;
`folders` is `filepaths` with only its directories kept. A folder row carries
the same history weight `cd` weighs its own rows by, and a file row carries
none, the way `cd` never offers one to weigh in the first place. `help` offers
the sibling subcommands of the argument's own enclosing node, so `fnm help `
offers `fnm`'s own subcommands rather than `help`'s, which has none of its
own. `history` answers nothing yet; a later phase gives it a reader.

An argument one of those two templates fills also gets `cd`'s own row that
runs the line, at the top of the menu and under the highlight, whenever what
is typed already names something on disk. `ls assets/`, `ls assets` and
`ls readme` each get one, and so does `git blame readme`. Enter there runs the
line the way it does under `ls`, which is the one place a `git` line ends in a
command rather than back on the shell's own editor. The row belongs to the
`filepaths` argument rather than to anything Git's own menu decided, and a
line that menu still answers never gets one: none of the three things it reads
is a path a `cd` would take. A name still being typed does not and neither does
an empty argument. A `folders` argument refuses a file. That is not what it
asked for and the menu never offered it either. A link with no target still
counts, the same way the scan behind the rows still lists it. Nothing else
here gets the row. A subcommand is a word to go on from rather than an answer
and the menu under it is what says where.

Two arguments are answered by a reader of surmise's own. `make ` offers the
targets of the makefile beside the line and the same argument behind `-j`,
`-B` and `-e` offers them too. `ssh ` offers host names. Both readers read
files and neither runs a program. The targets come from `GNUmakefile`,
`makefile` or `Makefile`, whichever of make's own three names is there first.
The hosts come from `~/.ssh/config`, `/etc/ssh/ssh_config` and
`~/.ssh/known_hosts`. `make -qp` would give the thorough answer and it expands
the makefile to do it. Completing a line would then run whatever
`$(shell ...)` the makefile holds and reading the text cannot. A `Host` line's
patterns are not host names and a hashed `known_hosts` entry holds no name to
read. An `Include` in an SSH configuration is not followed either. Each file
is read up to 64 KiB, the cap a Git query's own output already carries.

An argument that needs anything else — a script, another native reader, a
package's own scripts, a branch name, anything else `specs/dynamic.txt`
names — still offers no rows rather than guessing at one or running one
unasked. Those two are two of the 4854 arguments that file lists and the other
4852 show nothing. Teaching `argwalk` to fill one of those in from a
generator, the way the Git branch and file readers already do their own,
remains a later phase.

A space opens the menu here too, the way a bare `cd ` or `git ` already did:
`docker ` and `docker container ` both reach it, because the widget checks
the line's first word rather than the whole line and an alias resolves
before that check does. The set that word is checked against is `surmise
specs names`'s own output, a zsh array the widget fills once from the first
space or Tab a shell asks of it — one fork, about 12 ms, spent on whichever
of the two comes first — and reads for every one after that, a hash lookup
with no fork left in it and nothing worth measuring.

Reading and normalizing a specification costs about 2.4 ms for `git`'s and
about 19 ms for the worst one in the corpus, so a menu loads a command's spec
once — the same way its Git completions and its directory scan are already
kept for one menu — and answers every later key from what that load found.

The corpus is frozen at the May 2025 release `specs/` was read from. A
command that has grown a subcommand since gets no row for it until the corpus
is regenerated.

## Build, test and lint

```sh
make build                  # cargo build --locked
make release                # cargo build --locked --release
make check                  # the gate: fmt-check, clippy, test, then shell
make install                # cargo install --locked --path .
make uninstall              # cargo uninstall surmise
make clean                  # cargo clean
make specs CORPUS=<path>    # regenerate specs/ from a spec corpus
make specs-check            # fmt and clippy for tools/spec-convert
```

`make check` is the gate and the pre-commit hook runs it. CI calls the same
Makefile targets as separate steps so each one gets its own result in the
GitHub interface and the commands keep a single definition. CI then runs
`make release` as a fifth step. The gate does not cover that step and a green
hook therefore does not promise a green CI run.

CI runs the same gate on macOS and on Linux. A failure on one does not cancel
the other, because which platform failed is the answer a matrix exists to give.
No other platform is built and no other platform is checked.

## Completion spec data

`specs/` holds a completion specification for each of 727 commands: their
subcommands, their options, their arguments and the description of every one of
those. It is 1481 JSON files plus an index and it is committed. Nothing
downloads it, nothing generates it during a build and it needs no network.
`build.rs` reads that whole tree at build time and `spec_store` is the
byte-level lookup the binary carries: `get(name)` decompresses one spec and
`commands()` returns the compiled-in list of 727 names. `spec` parses what
comes back into the shape `argwalk` walks, and "Other commands" above says
what a menu does with one. Git's own menu is the second reader of this data
and it takes two things from it: the description a subcommand row shows where
this data carries one and the arguments beside its name. "Git" above is where
the rest of those descriptions come from.
This section stays about the directory itself: what it holds and what
carrying it costs.

The point of committing it is that surmise then outlives whatever published it.
The corpus these files came from has had no release since May 2025 and a tool
that fetches its data at install time is a tool that stops working the day that
package goes away.

`tools/spec-convert` is what produced the directory. A specification is
published as a JavaScript module whose value is built by running code, so the
tool runs each one once, keeps every field that is data and drops every field
that is a function. Its engine is `boa_engine`, which is pure Rust, so
regenerating the data asks for cargo and nothing else. A whole corpus takes
about a minute across the machine's cores.

That crate sits outside this package on purpose. Its manifest carries an empty
`[workspace]` table and that is what stops cargo searching upward for one, so
`cargo build` here never sees it and `cargo install` never sees its
dependencies. `make check` does not build it either. `make specs-check` is its
gate and it is separate because the converter runs a few times a year and its
dependency tree is four times the size of this package's.

The output is deterministic. Object keys are sorted and array order is the
specification's own, so the same corpus gives the same bytes and a regeneration
diffs to exactly what changed upstream. The files are pretty printed for that
same reason. The cost is 123 MB in the working tree. The cost that matters is
7.9 MB in `.git` and a third of a second added to what the pre-commit hook
checks out.

A file is named for what reaches it. `specs/git.json` is the command `git`, and
a `loadSpec` of `aws/s3` inside another file is `specs/aws/s3.json`. A command
whose specification lives in a directory keeps its own part in `index.json`
inside it. `specs/index.json` at the top lists the commands a person can type,
every name the corpus resolves, the six whose specification is a function of
the installed tool's version, and the corpus version this data was read from.

An argument whose suggestions only code could have produced carries
`"dyn": true` and `specs/dynamic.txt` lists all 4854 of them with the reason.
That is 2.1 % of 235 174 arguments and the other 97.9 % need nothing but the
data. Every one of 50 784 subcommands and 282 837 options converted whole. A
`dyn` argument is one a native reader has to answer, which is what the Git
branch and file readers already do, and the data usually still says what to run:
`git switch` keeps its `git branch --sort=-committerdate` and loses only the
code that parsed the output.

`specs/LICENSE` and `THIRD_PARTY.md` carry the attribution. The descriptions are
the upstream's own text and the licence travels with them.

Every file under `specs/` except the top-level `index.json` compresses on its
own into one blob, so `spec_store::get` decompresses the one spec a menu needs
and leaves the rest of the corpus alone. `flate2` with the `rust_backend`
feature does both ends and neither of them reaches for a C library. The blob is
9 105 907 bytes.

The binary carries that weight now. `spec_menu` reaches
`spec_store::get_configured` for any command besides `cd`, so a linker keeps
the corpus rather than dropping it and a release build measures 12 684 080
bytes.

## Configuration

`$XDG_CONFIG_HOME/surmise/config.toml` holds the settings a person changes.
Without an absolute `XDG_CONFIG_HOME` the path is
`~/.config/surmise/config.toml`. Without either an absolute config directory
or an absolute home directory there is no config and the defaults stand. A
missing file also means defaults. `surmise settings path` prints the resolved
path whether or not the file exists.

`surmise settings show` prints every key this build reads with the value the
picker would use, as TOML a person can paste back into the file. A key the
file leaves out is named with its default anyway, because what this answers is
what the picker will do rather than what somebody wrote down. A warning the
file earned leads that output as a comment, and a parse error carries the line
and the caret under it there. A file nothing could read otherwise leaves every
value below it a default it never asked for, printed as though it had.

Four commands write that file:

```sh
surmise settings set icons nerd              # one value
surmise settings unset icons                 # drop it and let the default stand
surmise settings add disabled_commands kubectl    # one entry of a list
surmise settings remove disabled_commands kubectl # one entry back out
```

`set` and `unset` answer for `enabled` and `icons`. `add` and `remove` answer
for `disabled_commands` and `spec_dirs`. Naming the wrong one of the two says
which verb reaches that key rather than writing anything, and so does a key or
a value this build does not read. A value the picker would silently ignore is
the one thing a person typing a command must not be handed, because nothing
prints a warning at the prompt.

Only these four write there, and each of them is a person naming the change.
Nothing surmise decides for itself touches this file: a menu reads it and
leaves it where it found it. That is the whole of the difference between this
and `state.toml` above.

The file comes back with one value changed and nothing else moved. Comments,
blank lines and the order of the keys all survive, because the writer reads a
document rather than a value. `unset` is the one exception and it is TOML's
own rule rather than a choice: a comment on the line above a key belongs to
that key, so dropping the key drops that comment with it. A comment at the end
of the key's own line survives a `set`.

A write lands whole or not at all. It goes to a file beside the real one and
is renamed over it, the way `state.toml` is written, and the file keeps the
permissions it already had. Two things are refused rather than written. A file
that does not parse is left exactly as it was, because a document surmise
cannot read is one it cannot put back either. A file that cannot be read at
all is left alone for the same reason and it is the stronger case: only a file
that is not there starts a write from nothing, and treating an unreadable one
as absent would replace everything in it.

A `config.toml` that is a symbolic link is followed and the file it names is
the one written. A person who keeps their settings in a dotfiles repository
and links to them there gets that file changed and keeps the link. The
`state.toml` write refuses a link instead, because that file is surmise's own
and nothing should be able to point it somewhere else.

A parse error never reaches the prompt, because nothing surmise does may
write to a person's terminal outside the menu. It becomes a warning instead,
kept for a later `doctor` command to report, and the picker runs with
defaults meanwhile. An unknown key is a warning of the same kind rather than
an error, because a file carrying a key from a later surmise should still
work with this one. A value one of the keys below does not know is a warning
of that kind too and that key keeps its default. A file that cannot be read
at all earns one as well, because a person whose own settings are being passed
over is owed the reason rather than a menu that quietly went back to the
defaults. Only a file that is not there is silent. Every complaint one file
earns is kept rather
than the first of them, because a person sent back twice for one file has
been told half of what the reader already knew.

Four keys have a reader today.

- `enabled` turns the picker off. `pick::run` checks it before it opens the
  terminal, so `false` answers every key with `PASS` and the shell's own
  completion runs instead.
- `disabled_commands` lists first words `spec_store::get_configured` refuses
  by name, before it asks `spec_dirs` or the compiled-in data at all.
- `spec_dirs` names directories laid out like `specs/` and holding the same
  JSON. `spec_store::get_configured` searches them in order before the
  compiled-in data, so a person's own spec for a private tool is found first
  and a stale public one can be overridden the same way. A name coming off
  the shell line is refused before it reaches the filesystem if it holds a
  `..` component or is itself an absolute path.
- `icons` names the glyph set. `"text"` is the default and `"nerd"` is the
  one a patched font draws. "Use" above says what that changes and why
  nothing guesses at the answer. `pick::run` reads it beside `enabled` and
  hands it to `ui::menu`, so the key costs the run nothing it was not
  already paying.

`spec_menu` is the menu that calls `spec_store::get_configured`, once per
command name it asks for, so three of the four keys now reach what a person
sees:
`enabled` through `pick::run`, the entry point every keystroke goes through,
and `disabled_commands` and `spec_dirs` through the spec that menu completes
from.

Git's own menu is the second caller and it asks for one name only, once per
menu, for the description a subcommand row shows and the arguments beside
its name. `git` in `disabled_commands` therefore leaves every row with no
arguments beside it and on whatever `git help -a` says of it. That is the
second source the "Git" section describes rather than a row gone blank. A
`git.json` under `spec_dirs` is what those rows read instead of the
committed one.
Neither key reaches anything else that menu does: the subcommand names, the
branches and the files all come from the installed Git either way. What the
same two keys do reach is every `git` line that menu declined, since those
walk the same specification: `git` in `disabled_commands` puts `git blame `
and `git stash ` back to where they were before either menu answered them.

`plans/phase-6-settings-cli.md` names the rest of the schema. A key with no
reader stays out of this build, because a config key that does nothing is a
promise the binary does not keep. Each one lands with the phase that reads it.

Hand-writing one file for `spec_dirs` is easy: the shape is the same JSON
`specs/` already holds. Converting somebody else's published spec is not,
because that spec is TypeScript and `tools/spec-convert` is the only reader
of it. `cargo install` places a binary and nothing beside it, so that
conversion needs a clone of this repository rather than an installed surmise.

## Repository conventions

`.cargo/config.toml`, `.vscode/`, the cargo-husky hook and the three symbolic
links come from the template this repository started from. Everything else in
this section was decided here.

`.cargo/config.toml` sets `rustflags = ["-Dwarnings"]`. Every warning is an
error in local builds, in the pre-commit hook and in CI. A new clippy lint
therefore breaks the build and has to be answered rather than ignored.
Restructure the code where a lint is wrong rather than reach for `#[allow]`.
Note that a `RUSTFLAGS` environment variable replaces this setting rather than
adds to it. An empty one is enough. A shell that sets one turns the gate off
and `make check` then passes on code that CI rejects.

The crate has a library target as well as a binary target. `src/lib.rs` holds
every module. There are two reasons. The first is the tests: `cargo test`
reaches a library and a module under a binary is testable only from inside
itself. The second is the gate. `dead_code` is a warning and `pub` does not
exempt an item in a binary crate, because nothing outside that crate can reach
it. A module that lands before its caller therefore turns the gate red. A
library target makes the same items reachable.

The library target also puts the library's `//!` and `///` code fences into the
gate. `cargo test` compiles each one as Rust unless the fence is tagged `text`
or `ignore`. An example that does not build therefore turns the gate red. A
fence in `src/main.rs` is not collected, because doc-tests come from the
library alone.

`tests/` holds the integration tests. Most of them run inside a pty and read
the screen the program draws rather than the bytes it wrote. A byte stream can
carry a box-drawing character and still render as garbage. `vt100` does the
rendering and `portable-pty` opens the device. Both are dev-dependencies and
neither reaches the installed binary.

The program runs inside that pty rather than beside it. crossterm resolves
`/dev/tty` for raw mode rather than reading stdin. A child spawned any other
way would therefore put the terminal the suite was started from into raw mode.

`tests/pty/main.rs` is the only test binary and its siblings are its modules.
`term` is the harness, `pick` runs `surmise --pick LINE`, `zsh` runs the
widget in a real `zsh -i`, `history` runs the directory hook and `demo` runs
`surmise demo` and opens a menu on the directory it was given. cargo makes a
target of `tests/<name>/main.rs` as well as of a file directly under `tests/`.
The second form would compile `term` again for each one. `dead_code` counts
the methods a binary never calls and turns the gate red. One binary sees every
caller the harness has and the harness's own `#[cfg(test)]` tests still run in
it.

`pick` covers what surmise draws. `zsh` covers the widget that reaches it.
That widget has no other gate: `make shell` reads its syntax alone. The `zsh`
tests write a `.zshrc` of their own and install the widget through
`eval "$($SURMISE_BIN init zsh)"`. A change to what `init zsh` prints therefore
reaches them. `SURMISE_BIN` points the widget at the build's own binary and
`/bin/zsh` is the shell they run. macOS ships that and a Linux runner installs
it.

That `.zshrc` prints a marker once `surmise-space` is bound and every `zsh`
test asserts it. Five of them claim that no menu opened and a widget that
never loaded would satisfy all five. It also sets a `chpwd` hook, because the
claim that Enter *ran* the line needs something the shell says on a directory
change rather than on an accepted line.

That shell starts with `-d` as well as `-i`. `ZDOTDIR` gives it the `.zshrc`
above and `-d` is what keeps the machine's own `/etc/zsh/zshrc` from running in
front of it. Debian and Ubuntu ship one that calls `compinit` with no flag, and
on a machine whose completion directories are group writable that call asks the
terminal whether to continue. The question lands before the prompt does and
every test then waits for a prompt that is never drawn. The `history` tests
need no such flag, because `-f` already drops every startup file.

Two of the prototype's zsh cases are not here. The first loaded
zsh-autosuggestions and zsh-syntax-highlighting. The widget's own comment names
that pair as the reason it hangs the trigger off the space key rather than off
`self-insert`. Neither plugin ships with macOS and a test that skips itself
when they are absent checks nothing. The second measured how long the menu
takes to open. That is a number rather than a claim and the gate has no place
for it.

`history` covers the directory hook and the storage behind it. It needs no
pty: the claim is what the hook wrote rather than what a screen shows. It runs
`/bin/zsh -fc` with the widget sourced into it and then reads the database
back.

`src/fixture.rs` is `pub` rather than `#[cfg(test)]`. An integration test links
the library as an ordinary crate and a `#[cfg(test)]` module is not compiled
into that build.

`Fixture::new` also widens the query budget. A prompt is what the 250 ms is for
and a test has no prompt waiting on it. A fixture-backed test spawns Git
several times over and a loaded runner can spend that whole budget on the fork
alone, so a test measured against it fails on the machine rather than on the
code. `read_commands` takes its patience as an argument for the same reason.
The file and branch readers are reached through `App` rather than called
directly and the fixture is what moves the budget for them. The installed
binary keeps the prompt's own.

The fixture cannot speak for the Git the code under test runs. `Fixture::git`
clears the environment for its own calls and the readers in `src/git.rs` spawn
Git with whatever the shell holds, which is what an installed surmise should
do. `GIT_DIR` and its companions then send that Git to another repository
altogether and every fixture-backed test reads the wrong one. git sets them
for every hook it runs, so this arrives through the gate itself rather than
through anything a person typed. In the main worktree `GIT_DIR` is the
relative `.git` and resolves to nothing from a fixture's own directory. In a
linked worktree it is absolute and the tests read this repository. The `test`
target therefore takes those variables out before cargo runs, which covers a
commit from a worktree, a CI job and a shell that exports one of them.

`rust-toolchain.toml` pins the toolchain for the same reason. A floating stable
plus `-Dwarnings` means a new lint can turn CI red with no change to the code.
Bump the pin deliberately and answer the new lints in the same commit. Note that
a `RUSTUP_TOOLCHAIN` environment variable overrides the file. A local shell
that sets one is not testing the pinned toolchain.

`rust-version` in `Cargo.toml` is the lowest toolchain this crate compiles on
and the `msrv` CI job is what proves it. That job reads the version out of
`Cargo.toml` so the declaration stays the one place it is written. Edition 2024
sets the floor at 1.85 and two things here sit above it. A `let` chain needs
1.88 and `floor_char_boundary` needs 1.91.

`cargo check` is the whole of that job. A lint set moves between releases and
answering a new lint belongs to the pinned toolchain rather than to this one.
Clippy on the pinned toolchain already reads `rust-version` and its
`incompatible_msrv` lint therefore turns the gate red on a library call newer
than the declaration. That covers the calls and not the language. A `let` chain
compiles on the pinned toolchain whatever the declaration says and only a build
on the declared toolchain catches it.

Every cargo command in the Makefile passes `--locked`. `Cargo.lock` is tracked
and a command that quietly re-resolves it would build something other than what
the lockfile describes. `cargo install` ignores the lockfile without that flag.

`publish = false` is set on purpose. This is scaffolding rather than a release.
Note also that `cargo package` collects every file git does not ignore. Add an
`exclude` list before you take that line out.

## The pre-commit hook

The hook runs `make check` against the **staged** content rather than against
the working tree. git runs a hook in the working tree and a gate that reads the
working tree can pass a commit it never saw. The hook checks the index out into
`.git/precommit` and runs there. Your working tree is never touched and a
failing gate therefore leaves nothing to clean up.

**cargo-husky** copies `.cargo-husky/hooks/pre-commit` into `.git/hooks` from a
build script. That build script runs only when the dev-dependencies compile.
`cargo test`, `cargo clippy --all-targets` and `cargo check --all-targets`
install the hook and plain `cargo build` does not. A fresh clone is therefore
ungated until one of those runs. `make check` installs it at the clippy step.

**cargo-husky does not reinstall a hook that already exists.** Editing
`.cargo-husky/hooks/pre-commit` and running `cargo test` leaves the old hook in
place. The running gate then differs from the committed one without saying so.
To install a changed hook:

```sh
cargo clean -p cargo-husky && cargo test
```

Then confirm it with `diff .git/hooks/pre-commit .cargo-husky/hooks/pre-commit`.
The installed copy carries two extra banner lines from cargo-husky.

Three states turn the gate off with no message. `git config core.hooksPath`
makes git ignore `.git/hooks` altogether. The `diff` above still reports a match
in that state and cannot detect it. `CARGO_HUSKY_DONT_INSTALL_HOOKS` in the
environment skips the install and cargo hides the warning it prints unless you
pass `-vv`. A `pre-commit` hook that some other tool wrote first also wins,
because cargo-husky leaves a foreign hook alone.

The shell files have a gate of their own and `make shell` runs it as the last
step of `make check`. The one POSIX script gets `shellcheck` and
`shfmt -i 2 -d`. The indentation flag matches what that script already uses.
The zsh widgets get `zsh -n` and nothing else. Neither shellcheck nor shfmt
has a zsh dialect. `# shellcheck shell=zsh` is SC1103 and shellcheck then
guesses bash. It reports SC2296 and SC2298 against a nested parameter
expansion that is ordinary zsh. It reports SC2086 and SC2076 against a shell
that splits no unquoted expansion and reads a quoted `=~` as a regex anyway.
shfmt parses a widget and then asks for the bash `case` layout it does not
use. Neither tool therefore says anything true about a widget and the gate
reads its syntax alone. `zsh -n` takes only its first file argument and each widget therefore
gets a run of its own.

A missing `shellcheck` or `shfmt` fails that gate rather than skipping it.
`brew install shellcheck shfmt` is the fix on macOS and
`apt-get install shellcheck shfmt` is the one on Linux. CI installs them the
same way. zsh ships with macOS and a Linux runner installs it beside the other
two, because the pty tests run the widget in a real one. That install therefore
goes in ahead of `make test` rather than ahead of `make shell`. A gate that
quietly checks nothing is worse than no gate and `make shell` therefore says so
when the widget glob matches nothing.

`shellcheck` and `shfmt` float rather than pin. A new release of either can
turn CI red with no change to the tree and that is the failure
`rust-toolchain.toml` exists to prevent. Homebrew has no clean way to pin a
formula and the shell surface here is one widget and one hook. Answer the new
finding when it lands rather than pin against it.

`README.md`, `AGENTS.md` and `GEMINI.md` are symbolic links to this file. Every
tool and every reader therefore gets the same document. GitHub follows the link
and renders this file as the repository README.

## Licence

MIT or Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
