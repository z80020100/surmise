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

# OLDPWD belongs to this change even when a quiet cd skipped an earlier hook.
# A failed write must not stop the other directory hooks or print at the prompt.
_surmise_chpwd() {
  emulate -L zsh
  [[ -n $OLDPWD && $OLDPWD != $PWD ]] &&
    command $SURMISE_BIN --record "$OLDPWD" "$PWD" </dev/null >/dev/null 2>&1
  return 0
}

autoload -Uz add-zsh-hook
add-zsh-hook chpwd _surmise_chpwd

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

# Put the line surmise wrote on the screen with a suggestion that fits it.
#
# zsh-autosuggestions asks for a new suggestion after a widget it wrapped
# changes the line. It wraps what `zle -la` listed when it bound. Under
# ZSH_AUTOSUGGEST_MANUAL_REBIND that binding happens once and the first prompt
# is the one that gets it. A surmise sourced after that is on no list of its
# own and the suggestion the old line earned would sit on the screen behind the
# new one.
#
# `autosuggest-clear` and `autosuggest-fetch` are two of the widgets the
# plugin's README names. The fetch alone would do while the plugin answers in
# line. An asynchronous one forks and leaves the old suggestion up until its
# answer lands. The clear is what takes that suggestion down meanwhile.
#
# `_zsh_autosuggest_bind_widgets` would add surmise to the list instead. That
# walk rewraps every widget in the shell rather than surmise's two and a widget
# aliased to a builtin does not survive it. The comment on the space key below
# is what that costs. It would also undo the saving that setting exists to make.
_surmise_redraw() {
  if (( $+widgets[autosuggest-clear] && $+widgets[autosuggest-fetch] )); then
    zle autosuggest-clear -w
    zle autosuggest-fetch -w
  fi
  zle reset-prompt
}

# A first argument says the key was not a completion request. zle hands a
# widget none of its own and only `surmise-space` passes one.
surmise-complete() {
  emulate -L zsh
  local from_space=$1 result ret
  # Filled here as well as on the space key below, so whichever one runs
  # first pays the one fork `_surmise_fill_specs` costs and the other only
  # ever reads the array it left behind.
  _surmise_fill_specs
  # A tag, RBUFFER, $HISTFILE and the alias table go on stdin as one record,
  # in that order, each field ended with a NUL. A value can hold a space, a
  # newline or a quote and a NUL is the one byte none of them carry, so it is
  # the only separator that cannot read part of one field as another. `[@]`
  # keeps a value with a space in it one field rather than letting the shell
  # split it apart before `print` ever sees it.
  #
  # The tag leads because a widget and a binary can fall out of step after
  # all. `init zsh` prints this text out of the binary that runs it, so a
  # fresh `eval` always pairs the two, and a shell that ran that eval before
  # the binary was upgraded keeps sending the older shape for as long as it
  # lives. Without the tag the newer binary would read this record's second
  # field as $HISTFILE, which in the older shape is the first alias's own
  # name, and every pair behind it would land one field out.
  local -a record
  record=("surmise-record-3" "$RBUFFER" "$HISTFILE" "${(kv)aliases[@]}")
  # Paint the pending change first. surmise asks the terminal where the cursor
  # is. zsh does not redraw until the widget returns and the answer would
  # otherwise be one keystroke behind.
  zle -R
  # $TTY names the real terminal device. surmise needs that name, because a
  # descriptor opened from /dev/tty cannot be polled on macOS.
  result=$(SURMISE_TTY=$TTY command $SURMISE_BIN --pick "$LBUFFER" < <(print -rN -- "${record[@]}"))
  ret=$?
  case $ret in
    0) LBUFFER=$result; _surmise_redraw ;;
    # Nothing was written. The suggestion on the screen still fits the line.
    1) zle reset-prompt ;;
    # Enter runs the line. surmise was handed the half in front of the cursor
    # alone and running the rest of it unseen is not what Enter offered. Take
    # the completion and leave the running to the person.
    3) LBUFFER=$result; _surmise_redraw; [[ -n $RBUFFER ]] || zle accept-line ;;
    # 2 is PASS: nothing surmise completes. Tab hands the key back to whatever
    # held it. A space is already typed and completing it is not what was
    # asked for. Any other status is a surmise that never ran and the same
    # answer is the right one for that too.
    2|*) [[ -n $from_space ]] || zle $_surmise_fallback ;;
  esac
}

zle -N surmise-complete
bindkey '^I' surmise-complete

# Open on a space after any command surmise has a specification for. The
# picker keeps the keys until it returns the line to the shell.
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

# The command names a space can open a menu for. `surmise specs names`
# prints the compiled-in list, one name per line, and `cd` is one of them.
# The array stays empty until the first space or the first Tab needs an
# answer, so a shell that opens and never completes anything never pays for
# the fork that fills it.
typeset -gA _surmise_specs

_surmise_fill_specs() {
  [[ -n $_surmise_specs_filled ]] && return
  typeset -g _surmise_specs_filled=1
  local name
  for name in ${(f)"$(command $SURMISE_BIN specs names 2>/dev/null)"}; do
    _surmise_specs[$name]=1
  done
}

# Open on a space that ends the line, when the line's first word names a
# command surmise has a specification for. The word is read the way the
# shell itself would read it before running anything, so `$aliases` is
# checked ahead of `_surmise_specs` and only the alias value's own first
# word counts: `alias g=git` opens the menu on a bare `g ` and an alias that
# expands to several words is read for the one it starts with rather than
# for the rest of them.
#
# Nothing here asks what comes after that word. `docker container ` opens
# the same way `docker ` does, because a space anywhere on the line ends up
# checking the same first word. What a later word means is the picker's own
# question, and a first word with no specification never reaches it.
surmise-space() {
  # The delegated widget runs under the person's own options rather than
  # surmise's. `emulate` waits until after it for that reason.
  zle $_surmise_space
  emulate -L zsh
  [[ -z $RBUFFER ]] || return
  _surmise_fill_specs
  # `[1]` on `${(z)...}` inline reads as a string index rather than an array
  # one when the split leaves a single word, which a bare `cd ` always does.
  # Splitting into `words` first is what keeps `words[1]` an array lookup
  # whether the split left one word or several.
  local -a words
  words=(${(z)LBUFFER})
  local word=${aliases[$words[1]]:-$words[1]}
  words=(${(z)word})
  (( $+_surmise_specs[$words[1]] )) || return
  surmise-complete from-space
}

zle -N surmise-space
bindkey ' ' surmise-space
