# surmise in the fish line editor.
#
#   surmise init fish | source
#
# Put the line in ~/.config/fish/config.fish after anything else that binds
# Tab or Space. surmise binds in the default mode and in vi's insert mode.
#
# CLAUDE.md holds the keymap and what SURMISE_BIN does.

set -q SURMISE_BIN; or set -g SURMISE_BIN surmise

# The record this widget writes leads with this tag. `surmise doctor` reads it,
# the shell it runs under and this shell itself out of the one variable
# exported beside it.
set -g _surmise_tag surmise-record-4
set -gx SURMISE_WIDGET "$_surmise_tag fish/$FISH_VERSION $fish_pid"

# A failed write must not print at the prompt.
set -g _surmise_pwd $PWD
function _surmise_on_pwd --on-variable PWD
    if test "$PWD" != "$_surmise_pwd"
        command $SURMISE_BIN --record $_surmise_pwd $PWD </dev/null >/dev/null 2>&1
    end
    set -g _surmise_pwd $PWD
end

# The command names a space can open a menu for. Filled on the first space or
# Tab that asks.
function _surmise_fill_specs
    set -q _surmise_specs; and return
    set -g _surmise_specs (command $SURMISE_BIN specs names 2>/dev/null)
end

# fish keeps its history in a file of its own and names it after the session.
# An empty `fish_history` keeps none.
function _surmise_history_file
    set -l name fish
    set -q fish_history; and set name $fish_history
    test -n "$name"; or return
    set -l data ~/.local/share
    set -q XDG_DATA_HOME; and set data $XDG_DATA_HOME
    echo $data/fish/$name"_history"
end

# A first argument says the key was not a completion request. Only
# `_surmise_space` passes one.
function _surmise_complete
    set -l from_space $argv[1]
    set -l line (commandline -b | string collect)
    set -l point (commandline -C)
    set -l left (string sub -l $point -- $line)
    set -l right (string sub -s (math $point + 1) -- $line)
    _surmise_fill_specs
    # A tag, the text right of the cursor and the history file go on stdin as
    # one record, each field ended with a NUL. CLAUDE.md says why. fish keeps
    # no alias table of the shape the record carries.
    set -l histfile (_surmise_history_file)
    set -l tty (tty)
    # The answer is the text left of the cursor, a NUL and the text right of
    # it.
    set -l out (printf '%s\0' $_surmise_tag "$right" "$histfile" |
        env SURMISE_TTY=$tty $SURMISE_BIN --pick "$left" | string split0)
    set -l ret $pipestatus[2]
    switch $ret
        case 0 3
            commandline -r -- "$out[1]$out[2]$right"
            commandline -C (string length -- "$out[1]")
            commandline -f repaint
            # Enter runs the line. surmise read the half in front of the
            # cursor alone and running the rest of it unseen is not what Enter
            # offered.
            if test $ret = 3; and test -z "$right"
                commandline -f execute
            end
        case 2
            # PASS: nothing surmise completes. Tab hands the key back to
            # fish. A space is already typed and completing it is not what was
            # asked for.
            test -n "$from_space"; or commandline -f complete
        case '*'
            commandline -f repaint
    end
end

# Open on a space that ends the line, when the line's first word names a
# command surmise has a specification for. The space and any abbreviation it
# expands are fish's own and already on the line.
function _surmise_space
    set -l line (commandline -b | string collect)
    test (commandline -C) -eq (string length -- "$line"); or return
    _surmise_fill_specs
    set -l words (string split -n ' ' -- $line)
    contains -- "$words[1]" $_surmise_specs; or return
    # fish draws the line once this returns. surmise asks the terminal where
    # the cursor is and the space has to be on the screen already.
    set -l tty (tty)
    printf ' ' >$tty
    _surmise_complete from-space
end

for mode in default insert
    bind -M $mode \t _surmise_complete
    bind -M $mode ' ' self-insert expand-abbr _surmise_space
end
