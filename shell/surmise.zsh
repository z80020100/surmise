# surmise in the zsh line editor.
#
#   eval "$(surmise init zsh)"
#
# Put that in ~/.zshrc AFTER zsh-autosuggestions and zsh-syntax-highlighting.
# surmise wraps the widget each of its keys already had and it has to wrap
# theirs rather than the other way round. It binds in the current keymap.
# Put it after `bindkey -e` or `bindkey -v` for that reason.
#
# CLAUDE.md holds the keymap and what SURMISE_BIN does.

typeset -g SURMISE_BIN=${SURMISE_BIN:-surmise}

# Whatever Tab did before this file was sourced stays Tab's job for every line
# surmise does not complete. `bindkey` quotes the key it reports back and a
# quoted key can split into several words. The widget name is the last word.
#
# Capture it once. A second source finds surmise's own widget on the key and
# would throw the real one away.
if [[ -z $_surmise_fallback ]]; then
  typeset -g _surmise_fallback=${${(z)$(bindkey '^I')}[-1]}
  case $_surmise_fallback in
    ''|undefined-key|surmise-complete) _surmise_fallback=expand-or-complete ;;
  esac
fi

# A first argument says the key was not a completion request. zle hands a
# widget none of its own and only `surmise-space` passes one.
surmise-complete() {
  emulate -L zsh
  local from_space=$1 result ret
  # Paint the pending change first. surmise asks the terminal where the cursor
  # is. zsh does not redraw until the widget returns and the answer would
  # otherwise be one keystroke behind.
  zle -R
  # $TTY names the real terminal device. surmise needs that name, because a
  # descriptor opened from /dev/tty cannot be polled on macOS.
  result=$(SURMISE_TTY=$TTY command $SURMISE_BIN --pick "$LBUFFER" </dev/null)
  ret=$?
  case $ret in
    0) LBUFFER=$result; zle reset-prompt ;;
    1) zle reset-prompt ;;
    # Enter runs the line. surmise was handed the half in front of the cursor
    # alone and running the rest of it unseen is not what Enter offered. Take
    # the completion and leave the running to the person.
    3) LBUFFER=$result; zle reset-prompt; [[ -n $RBUFFER ]] || zle accept-line ;;
    # 2 is PASS: nothing surmise completes. Tab hands the key back to whatever
    # held it. A space is already typed and completing it is not what was
    # asked for. Any other status is a surmise that never ran and the same
    # answer is the right one for that too.
    2|*) [[ -n $from_space ]] || zle $_surmise_fallback ;;
  esac
}

zle -N surmise-complete
bindkey '^I' surmise-complete

# Open on its own the moment the line becomes a bare `cd `. surmise then holds
# the keys until you leave it. That costs one process for the whole `cd`
# rather than one per keystroke.
#
# The trigger hangs off the space key rather than off `self-insert`. Wrapping
# `self-insert` does not survive zsh-autosuggestions. That plugin walks every
# widget `zle -la` reports and rebinds it. A saved alias looks like a built-in
# widget to that walk and the wrapper it writes then calls a built-in that
# does not exist.
if [[ -z $_surmise_space ]]; then
  typeset -g _surmise_space=${${(z)$(bindkey ' ')}[-1]}
  case $_surmise_space in
    ''|undefined-key|surmise-space) _surmise_space=self-insert ;;
  esac
fi

surmise-space() {
  # The delegated widget runs under the person's own options rather than
  # surmise's. `emulate` waits until after it for that reason.
  zle $_surmise_space
  emulate -L zsh
  [[ -z $RBUFFER && $LBUFFER =~ '^[[:blank:]]*cd[[:blank:]]$' ]] || return
  surmise-complete from-space
}

zle -N surmise-space
bindkey ' ' surmise-space
