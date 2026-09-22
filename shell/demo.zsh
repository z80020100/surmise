# The startup file the demo shell reads.
#
#   surmise demo
#
# That command writes this file as $ZDOTDIR/.zshrc under a throwaway
# directory and starts zsh on the directory it was run in. Nothing
# here is substituted on the way in. $SURMISE_BIN is the one thing it reads
# out of the environment and the command is what puts it there. The bytes
# `make shell` checks are therefore the bytes that run.
#
# CLAUDE.md says what the demo moves and what it leaves where it is.

# The person's own history file, which the $HISTFILE tie-break reads. zsh
# carries no default for it and `-d` drops the /etc file that usually sets
# one, so the name goes in here. SAVEHIST=0 is what keeps that file
# read-only: zsh saves the last $SAVEHIST entries as the shell leaves and a
# count of nothing leaves the file exactly as it was. This is the one file
# the demo reads and never writes, because a line typed in a demo is worth
# nothing to the ranking afterwards.
HISTFILE="${HISTFILE:-$HOME/.zsh_history}"
HISTSIZE=2000
SAVEHIST=0

bindkey -e

# What Tab falls through to on a line surmise declines. The widget captures
# whatever `^I` holds when it binds and this goes in ahead of it for that.
autoload -Uz compinit && compinit -u

# `g` opens git's own specification through the menu every other command
# reaches rather than through Git's own reader. CLAUDE.md's "Other commands"
# says why an alias for `git` lands there.
alias g=git
alias d=docker

# `surmise` here is the binary that started this demo rather than whichever
# one the PATH holds. A person reading a branch back out of a clone has no
# other one, and the settings command the opening text offers has to be this
# build's. The widget calls `command $SURMISE_BIN` and reaches past this
# either way.
surmise() { command $SURMISE_BIN "$@" }

PROMPT='%F{cyan}demo%f %F{yellow}%1~%f %# '
PROMPT_EOL_MARK=''

eval "$($SURMISE_BIN init zsh)"

print -P ''
print -P '%F{cyan}surmise demo%f'
print -P '%F{8}A clean shell with surmise the one thing loaded in it, on your own%f'
print -P '%F{8}files. Your config, your state file and your directory history are%f'
print -P '%F{8}the real ones, so a change you make here you keep and a cd you run%f'
print -P '%F{8}here is a visit your next menu ranks by. You are in the directory%f'
print -P '%F{8}you started from and a line you run here runs.%f'
# The tie-break reads $HISTFILE and a clean shell is one that never read the
# .zshrc naming it, so the fallback at the top of this file is a guess. The
# claim therefore goes only where that guess landed on a file the reader
# will take. -f rather than -r because src/histfile.rs refuses everything
# that is not a regular file: a readable directory there would earn the
# claim and teach the ranking nothing.
if [[ -f "$HISTFILE" && -r "$HISTFILE" ]]; then
  print -P '%F{8}Your shell history is read for the ranking and never written.%f'
else
  print -P '%F{8}No shell history was found, so nothing ranks by what you have%f'
  print -P '%F{8}typed. HISTFILE=... surmise demo names the file you keep.%f'
fi
print -P ''
print -P '  %F{green}git %f          the count on the top edge, the glyphs, the hints'
print -P '  %F{green}cargo %f        subcommands ahead of options'
print -P '  %F{green}svn commit %f   a priority in the specification puts -m first'
print -P '  %F{green}git revert %f   one row of description, and ^O opens the rest'
print -P '  %F{green}docker %f       a menu straight out of the specification'
print -P '  %F{green}git switch %f   the branches of the repository you are in'
print -P '  %F{green}git add %f      its files and its folders'
print -P '  %F{green}make %f         the targets of the makefile beside you'
print -P '  %F{green}cd %f           the directories here'
print -P '  %F{green}ssh %f          the hosts your own ssh configuration names'
print -P '  %F{green}g %f            an alias on the specification it expands to'
print -P ''
# The glyphs below are the bytes themselves rather than `\u` escapes. `print`
# converts an escape through the current locale and a shell without a UTF-8
# one answers `character not in range` and drops the rest of the line. The
# demo inherits whatever locale a person has and these bytes need none.
print -P '%F{8}A patched font has a set of glyphs in place of the characters in%f'
print -P '%F{8}front of each name:%f            rs   md   Makefile'
# The config is the person's own and the set may be on already. Telling
# somebody to switch on what they switched on themselves would be the demo
# talking about a config of its own rather than about theirs.
if [[ "$(command $SURMISE_BIN settings show)" == *'icons = "nerd"'* ]]; then
  print -P '%F{8}Your own config asks for that set and the menu below draws it.%f'
else
  print -P '%F{8}Boxes there are a terminal without one. Shapes there and this%f'
  print -P '%F{8}switches the menu over in the config you keep:%f'
  print -P '  %F{green}surmise settings set icons nerd%f'
  print -P '%F{8}Put it back with%f %F{green}surmise settings unset icons%f%F{8}.%f'
fi
print -P ''
print -P '%F{8}Esc leaves a menu. Ctrl-C puts the line back. exit ends the demo.%f'
print -P ''
