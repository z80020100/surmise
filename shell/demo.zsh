# The startup file the demo shell reads.
#
#   surmise demo
#
# That command makes a throwaway home, writes this file into it as
# $ZDOTDIR/.zshrc and starts zsh on the directory it was run in. Nothing
# here is substituted on the way in. $SURMISE_BIN is the one thing it reads
# out of the environment and the command is what puts it there. The bytes
# `make shell` checks are therefore the bytes that run.
#
# CLAUDE.md says what the demo moves and what it leaves where it is.

# The history file is a fixture and the $HISTFILE tie-break reads it. A demo
# that wrote its own lines back would change the order it is there to show.
setopt EXTENDED_HISTORY
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

PROMPT='%F{cyan}demo%f %F{yellow}%1~%f %# '
PROMPT_EOL_MARK=''

eval "$($SURMISE_BIN init zsh)"

print -P ''
print -P '%F{cyan}surmise demo%f'
print -P '%F{8}A home of its own. Your history, your settings and the directories%f'
print -P '%F{8}you have visited stay where they are and this home goes when you%f'
print -P '%F{8}leave. You are in the directory you started from and a line you%f'
print -P '%F{8}run here runs.%f'
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
print -P '  %F{green}ssh %f          four hosts the demo wrote because $HOME moved'
print -P '  %F{green}g %f            an alias on the specification it expands to'
print -P ''
print -P '%F{8}Esc leaves a menu. Ctrl-C puts the line back. exit ends the demo.%f'
print -P ''
