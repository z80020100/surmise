# shellcheck shell=bash
# surmise in the bash line editor.
#
#   eval "$(surmise init bash)"
#
# bash 4.4 or later. Put the line in ~/.bashrc after anything else that binds
# Tab or Space, and after `set -o vi` where that is set. surmise binds in the
# current keymap.
#
# CLAUDE.md holds the keymap and what SURMISE_BIN does.

if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))); then

  SURMISE_BIN=${SURMISE_BIN:-surmise}

  # The record this widget writes leads with this tag. `surmise doctor` reads
  # it, the shell it runs under and this shell itself out of the one variable
  # exported beside it.
  _surmise_tag=surmise-record-4
  export SURMISE_WIDGET="$_surmise_tag bash/$BASH_VERSION $$"

  # bash has no hook for a directory change. The prompt is the next place the
  # shell stops and a change since the last one is a visit. A failed write must
  # not stop the rest of the prompt or print at it, and the status the line
  # ended with is the next command's to read.
  _surmise_pwd=$PWD
  _surmise_prompt() {
    local status=$?
    if [[ $PWD != "$_surmise_pwd" ]]; then
      command "$SURMISE_BIN" --record "$_surmise_pwd" "$PWD" </dev/null >/dev/null 2>&1
      _surmise_pwd=$PWD
    fi
    return "$status"
  }
  # bash 5.1 runs every element of an array and anything older runs a string.
  if [[ " ${PROMPT_COMMAND[*]-} " != *_surmise_prompt* ]]; then
    if ((BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] >= 501)); then
      PROMPT_COMMAND+=(_surmise_prompt)
    else
      printf -v PROMPT_COMMAND '%s' "_surmise_prompt${PROMPT_COMMAND[0]:+;${PROMPT_COMMAND[0]}}"
    fi
  fi

  # The command names a space can open a menu for. `surmise specs names`
  # prints them one to a line. Filled on the first space or Tab that asks.
  declare -gA _surmise_specs=()
  _surmise_fill_specs() {
    [[ -n ${_surmise_specs_filled-} ]] && return
    _surmise_specs_filled=1
    local name
    while IFS= read -r name; do
      _surmise_specs[$name]=1
    done < <(command "$SURMISE_BIN" specs names 2>/dev/null)
  }

  # Whatever Tab did before this file was read stays Tab's job for every line
  # surmise does not complete. Capture it once. A second read finds surmise's
  # own binding there.
  if [[ -z ${_surmise_fallback-} ]]; then
    _surmise_fallback=$(bind -p 2>/dev/null | sed -n 's/^"\\C-i": //p')
    [[ -n $_surmise_fallback ]] || _surmise_fallback=complete
  fi

  # A function readline runs cannot call one of readline's own. Tab and Space
  # are therefore each a macro of three keys. The first runs surmise and binds
  # the other two to what has to happen after it: a redraw, the old Tab, the
  # line running, or nothing at all.
  _surmise_then() {
    bind "\"\\C-x\\C-_\\C-b\": ${1:-\"\"}"
    bind "\"\\C-x\\C-_\\C-c\": ${2:-\"\"}"
  }

  # A first argument says the key was not a completion request. Only
  # `_surmise_space` passes one.
  _surmise_complete() {
    local from_space=${1-} tty
    local left=${READLINE_LINE:0:READLINE_POINT} right=${READLINE_LINE:READLINE_POINT}
    _surmise_fill_specs
    # A tag, the text right of the cursor, $HISTFILE and the alias table go on
    # stdin as one record, each field ended with a NUL. CLAUDE.md says why.
    local -a record=("$_surmise_tag" "$right" "${HISTFILE-}")
    local name
    for name in "${!BASH_ALIASES[@]}"; do
      record+=("$name" "${BASH_ALIASES[$name]}")
    done
    # The answer is the text left of the cursor, a NUL and the text right of
    # it. A bash variable holds no NUL and `mapfile` splits on it instead. The
    # status follows behind one more NUL.
    tty=$(tty)
    local -a out
    mapfile -d '' out < <(
      SURMISE_TTY=$tty command "$SURMISE_BIN" --pick "$left" < <(printf '%s\0' "${record[@]}")
      printf '\0%s' "$?"
    )
    case ${out[-1]} in
    0 | 3)
      READLINE_LINE=${out[0]}${out[1]-}$right
      READLINE_POINT=${#out[0]}
      # Enter runs the line. surmise read the half in front of the cursor
      # alone and running the rest of it unseen is not what Enter offered.
      if [[ ${out[-1]} == 3 && -z $right ]]; then
        _surmise_then redraw-current-line accept-line
      else
        _surmise_then redraw-current-line
      fi
      ;;
    # 2 is PASS: nothing surmise completes. Tab hands the key back to what
    # held it. A space is already typed and completing it is not what was
    # asked for.
    2) if [[ -n $from_space ]]; then _surmise_then; else _surmise_then "$_surmise_fallback"; fi ;;
    *) _surmise_then redraw-current-line ;;
    esac
  }

  # Open on a space that ends the line, when the line's first word names a
  # command surmise has a specification for. An alias reaches the name it
  # expands to and only the first word of that value counts.
  _surmise_space() {
    READLINE_LINE="${READLINE_LINE:0:READLINE_POINT} ${READLINE_LINE:READLINE_POINT}"
    ((READLINE_POINT++))
    _surmise_then
    [[ -z ${READLINE_LINE:READLINE_POINT} ]] || return 0
    _surmise_fill_specs
    local -a words
    read -ra words <<<"$READLINE_LINE"
    local word=${words[0]-}
    [[ -n $word ]] || return 0
    read -ra words <<<"${BASH_ALIASES[$word]:-$word}"
    [[ -n ${_surmise_specs[${words[0]-}]-} ]] || return 0
    # readline draws the space once this returns. surmise asks the terminal
    # where the cursor is and the space has to be on the screen already.
    printf ' ' >"$(tty)"
    _surmise_complete from-space
  }

  bind -x '"\C-x\C-_\C-a": _surmise_complete'
  bind -x '"\C-x\C-_\C-s": _surmise_space'
  _surmise_then
  bind '"\t": "\C-x\C-_\C-a\C-x\C-_\C-b\C-x\C-_\C-c"'
  bind '" ": "\C-x\C-_\C-s\C-x\C-_\C-b\C-x\C-_\C-c"'
fi
